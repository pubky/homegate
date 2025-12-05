use crate::shared::HomeserverAdminApiTrait;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::{
    PreludeSendCodeStatus, PreludeVerifyCodeStatus, SmsVerificationProviderApi,
};
use crate::sms_verification::repository::SmsVerificationRepository;
use crate::sms_verification::types::{
    SendCodeRequest, SendCodeResponse, VerifyCodeRequest, VerifyCodeResponse,
};
use regex::Regex;
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct SmsVerificationService<T: SmsVerificationProviderApi, S: HomeserverAdminApiTrait> {
    repository: SmsVerificationRepository,
    prelude_api: T,
    homeserver_admin_api: S,
    max_verified_sessions: u32,
}

impl<T: SmsVerificationProviderApi, S: HomeserverAdminApiTrait> SmsVerificationService<T, S> {
    pub fn new(
        repository: SmsVerificationRepository,
        prelude_api: T,
        homeserver_admin_api: S,
        max_verified_sessions: u32,
    ) -> Self {
        Self {
            repository,
            prelude_api,
            homeserver_admin_api,
            max_verified_sessions,
        }
    }

    /// Validates that a phone number is in E.164 format
    fn validate_phone_number(phone_number: &str) -> Result<(), SmsVerificationError> {
        // E.164 format: starts with +, followed by 1-15 digits
        let e164_regex = Regex::new(r"^\+[1-9]\d{1,14}$").unwrap();

        if e164_regex.is_match(phone_number) {
            Ok(())
        } else {
            Err(SmsVerificationError::InvalidPhoneNumber(
                phone_number.to_string(),
            ))
        }
    }

    /// Check if a phone number can create a new verification
    async fn check_verification_limit(
        &self,
        phone_number: &str,
    ) -> Result<(), SmsVerificationError> {
        let count = self
            .repository
            .count_verified_sessions(phone_number)
            .await
            .map_err(SmsVerificationError::DatabaseError)?;
        if count >= self.max_verified_sessions as i64 {
            return Err(SmsVerificationError::TooManyVerifiedSessions);
        }
        Ok(())
    }

    /// Initiates a phone number verification process
    pub async fn send_code(
        &self,
        request: SendCodeRequest,
        ip_address: IpAddr,
    ) -> Result<SendCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        self.check_verification_limit(&request.phone_number).await?;

        let prelude_response = self
            .prelude_api
            .create_verification(&request.phone_number, Some(ip_address))
            .await?;

        let status = PreludeSendCodeStatus::from_prelude_status(&prelude_response.status)?;
        if status == PreludeSendCodeStatus::Retry {
            tracing::info!(
                phone_number = %request.phone_number,
                prelude_id = %prelude_response.id,
                "User retrying verification code"
            );
        }

        // Create SMS record (will skip if active session already exists)
        self.repository
            .create_verification(&request.phone_number, &prelude_response.id)
            .await?;

        Ok(SendCodeResponse {
            status,
            reason: prelude_response.reason,
        })
    }

    /// Validates a verification code for a phone number
    pub async fn verify_code(
        &self,
        request: VerifyCodeRequest,
    ) -> Result<VerifyCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        // Confirm first that there's an active verification session in our database
        self.repository
            .check_pending_exists(&request.phone_number)
            .await?;

        let prelude_response = self
            .prelude_api
            .check_code(&request.phone_number, &request.code)
            .await?;

        let status = PreludeVerifyCodeStatus::from_prelude_status(&prelude_response.status)?;
        match status {
            PreludeVerifyCodeStatus::Success => {
                let code = self.homeserver_admin_api.generate_signup_token().await?;
                self.repository
                    .mark_verified(&prelude_response.id, &code)
                    .await?;
                Ok(VerifyCodeResponse {
                    status,
                    signup_code: Some(code),
                    homeserver_pubky: Some(self.homeserver_admin_api.get_homeserver_pubky()),
                })
            }
            PreludeVerifyCodeStatus::ExpiredOrNotFound => {
                // Mark session as permanently failed
                self.repository
                    .mark_failed(&prelude_response.id, status.as_str())
                    .await?;
                Ok(VerifyCodeResponse {
                    status,
                    signup_code: None,
                    homeserver_pubky: None,
                })
            }
            PreludeVerifyCodeStatus::Failure => {
                // Wrong code - don't mark as failed, allow retries
                Ok(VerifyCodeResponse {
                    status,
                    signup_code: None,
                    homeserver_pubky: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::HomeserverAdminApi;
    use crate::sms_verification::prelude_api::PreludeAPI;

    #[test]
    fn test_validate_phone_number() {
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+30123456789"
            )
            .is_ok()
        );
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+1234567890123"
            )
            .is_ok()
        );
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number("+12")
                .is_ok()
        );
        // Missing +
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "30123456789"
            )
            .is_err()
        );
        // Starts with +0
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+0123456789"
            )
            .is_err()
        );
        // Contains spaces
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+30 123 456 789"
            )
            .is_err()
        );
        // Contains hyphens
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+30-123-456-789"
            )
            .is_err()
        );
        // Too short (only country code)
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number("+1")
                .is_err()
        );
        // Too long (more than 15 digits)
        assert!(
            SmsVerificationService::<PreludeAPI, HomeserverAdminApi>::validate_phone_number(
                "+1234567890123456"
            )
            .is_err()
        );
    }
}

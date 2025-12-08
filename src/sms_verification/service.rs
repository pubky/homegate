use crate::HomeserverAdminAPI;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::{
    PreludeAPI, PreludeCheckCodeResponse, PreludeCreateVerificationResponse,
};
use crate::sms_verification::repository::SmsVerificationRepository;
use crate::sms_verification::types::{
    CreateVerificationRequest, CreateVerificationResponse, SendCodeRequest, SendCodeResponse,
};
use regex::Regex;
use std::net::IpAddr;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct SmsVerificationService {
    repository: SmsVerificationRepository,
    prelude_api: PreludeAPI,
    homeserver_admin_api: HomeserverAdminAPI,
    max_verified_sessions: u32,
}

impl SmsVerificationService {
    pub fn new(
        repository: SmsVerificationRepository,
        prelude_api: PreludeAPI,
        homeserver_admin_api: HomeserverAdminAPI,
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
        static E164_REGEX: OnceLock<Regex> = OnceLock::new();
        let e164_regex = E164_REGEX.get_or_init(|| {
            Regex::new(r"^\+[1-9]\d{1,14}$")
                .expect("E.164 regex pattern is valid and should compile")
        });

        if e164_regex.is_match(phone_number) {
            Ok(())
        } else {
            Err(SmsVerificationError::InvalidPhoneNumber(
                phone_number.to_string(),
            ))
        }
    }

    /// Check if a phone number can create a new verification
    pub async fn check_verification_limit(
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
    pub async fn create_verification(
        &self,
        request: CreateVerificationRequest,
        ip_address: IpAddr,
    ) -> Result<CreateVerificationResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        self.check_verification_limit(&request.phone_number).await?;

        let prelude_response = self
            .prelude_api
            .create_verification(&request.phone_number, Some(ip_address))
            .await?;

        let id = match &prelude_response {
            PreludeCreateVerificationResponse::Success { id } => id,
            PreludeCreateVerificationResponse::Retry { id } => {
                tracing::info!(
                    phone_number = %request.phone_number,
                    prelude_id = %id,
                    "User retrying verification code"
                );
                id
            }
            PreludeCreateVerificationResponse::Blocked { id, .. } => id,
        };

        // Create SMS record (will skip if active session already exists)
        self.repository
            .create_verification(&request.phone_number, id)
            .await?;

        Ok(match prelude_response {
            PreludeCreateVerificationResponse::Success { .. } => {
                CreateVerificationResponse::Success
            }
            PreludeCreateVerificationResponse::Retry { .. } => CreateVerificationResponse::Retry,
            PreludeCreateVerificationResponse::Blocked { reason, .. } => {
                CreateVerificationResponse::Blocked { reason }
            }
        })
    }

    /// Validates a verification code for a phone number
    pub async fn send_code(
        &self,
        request: SendCodeRequest,
    ) -> Result<SendCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        // Confirm first that there's an active verification session in our database
        self.repository
            .check_pending_exists(&request.phone_number)
            .await?;

        let prelude_response = self
            .prelude_api
            .check_code(&request.phone_number, &request.code)
            .await?;

        match prelude_response {
            PreludeCheckCodeResponse::Success { id, .. } => {
                let code = self.homeserver_admin_api.generate_signup_token().await?;
                self.repository.mark_verified(&id, &code).await?;
                Ok(SendCodeResponse::Success {
                    signup_code: code,
                    homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
                })
            }
            PreludeCheckCodeResponse::ExpiredOrNotFound { id, .. } => {
                self.repository
                    .mark_failed(&id, "expired_or_not_found")
                    .await?;
                Ok(SendCodeResponse::ExpiredOrNotFound)
            }
            PreludeCheckCodeResponse::Failure { .. } => {
                // Wrong code - don't mark as failed, allow retries
                Ok(SendCodeResponse::Failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_phone_number() {
        assert!(SmsVerificationService::validate_phone_number("+30123456789").is_ok());
        assert!(SmsVerificationService::validate_phone_number("+1234567890123").is_ok());
        assert!(SmsVerificationService::validate_phone_number("+12").is_ok());
        // Missing +
        assert!(SmsVerificationService::validate_phone_number("30123456789").is_err());
        // Starts with +0
        assert!(SmsVerificationService::validate_phone_number("+0123456789").is_err());
        // Contains spaces
        assert!(SmsVerificationService::validate_phone_number("+30 123 456 789").is_err());
        // Contains hyphens
        assert!(SmsVerificationService::validate_phone_number("+30-123-456-789").is_err());
        // Too short (only country code)
        assert!(SmsVerificationService::validate_phone_number("+1").is_err());
        // Too long (more than 15 digits)
        assert!(SmsVerificationService::validate_phone_number("+1234567890123456").is_err());
    }
}

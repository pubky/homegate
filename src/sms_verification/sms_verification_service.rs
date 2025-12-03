use crate::external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi};
use crate::persistence::db::Db;
use crate::sms_verification::error::SmsVerificationError;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SendCodeStatus {
    Success,
    Retry,
    Blocked,
}
impl SendCodeStatus {
    pub fn from_prelude_status(status: &str) -> Result<Self, SmsVerificationError> {
        match status {
            "success" => Ok(Self::Success),
            "retry" => Ok(Self::Retry),
            "blocked" => Ok(Self::Blocked),
            _ => Err(SmsVerificationError::InvalidResponse(format!(
                "Unknown send code status from Prelude: {}",
                status
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub phone_number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendCodeResponse {
    pub status: SendCodeStatus,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifyCodeStatus {
    Success,
    Failure,
    ExpiredOrNotFound,
}

impl VerifyCodeStatus {
    pub fn from_prelude_status(status: &str) -> Result<Self, SmsVerificationError> {
        match status {
            "success" => Ok(Self::Success),
            "failure" => Ok(Self::Failure),
            "expired_or_not_found" => Ok(Self::ExpiredOrNotFound),
            _ => Err(SmsVerificationError::InvalidResponse(format!(
                "Unknown verify code status from Prelude: {}",
                status
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct VerifyCodeRequest {
    pub phone_number: String,
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VerifyCodeResponse {
    pub status: VerifyCodeStatus,
    pub signup_code: Option<String>,
    pub homeserver_pubky: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SmsVerificationService<T: SmsVerificationProviderApi, S: HomeserverAdminApiTrait> {
    db: Db,
    prelude_api: T,
    homeserver_admin_api: S,
}

impl<T: SmsVerificationProviderApi, S: HomeserverAdminApiTrait> SmsVerificationService<T, S> {
    pub fn new(prelude_api: T, db: Db, homeserver_admin_api: S) -> Self {
        Self {
            db,
            prelude_api,
            homeserver_admin_api,
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
            .db
            .count_verified_sessions(phone_number)
            .await
            .map_err(SmsVerificationError::DatabaseError)?;
        if count >= 10 {
            return Err(SmsVerificationError::TooManyVerifiedSessions);
        }
        Ok(())
    }

    /// Initiates a phone number verification process
    pub async fn send_code(
        &self,
        request: SendCodeRequest,
    ) -> Result<SendCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        // Check if phone number has reached the maximum verified sessions limit
        self.check_verification_limit(&request.phone_number).await?;

        // Always call Prelude API to validate/create verification session
        let ip_ref = request.ip_address.as_deref();
        let prelude_response = self
            .prelude_api
            .create_verification(&request.phone_number, ip_ref)
            .await?;

        if prelude_response.status == "retry" {
            tracing::info!(
                phone_number = %request.phone_number,
                prelude_id = %prelude_response.id,
                "User retrying verification code"
            );
        }

        // Create SMS record (will skip if active session already exists)
        self.db
            .create_sms(&request.phone_number, &prelude_response.id)
            .await?;

        Ok(SendCodeResponse {
            status: SendCodeStatus::from_prelude_status(&prelude_response.status)?,
            reason: prelude_response.reason,
        })
    }

    /// Validates a verification code for a phone number
    pub async fn verify_code(
        &self,
        request: VerifyCodeRequest,
    ) -> Result<VerifyCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(&request.phone_number)?;

        // TODO: Make database call to check verification attempt before calling PreludeAPI
        // This should:
        // - Verify that a verification was initiated for this phone number
        // - Check that the verification hasn't expired
        // - Check rate limits for failed attempts

        let prelude_response = self
            .prelude_api
            .check_code(&request.phone_number, &request.code)
            .await?;

        let status = VerifyCodeStatus::from_prelude_status(&prelude_response.status)?;

        let signup_code = if status == VerifyCodeStatus::Success {
            let code = self.homeserver_admin_api.generate_signup_token().await?;
            self.db.verify_sms(&prelude_response.id, &code).await?;
            Some(code)
        } else {
            None
        };

        Ok(VerifyCodeResponse {
            status,
            signup_code,
            homeserver_pubky: None, // TODO: Get from homeserver_admin_api
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_apis::{HomeserverAdminApi, PreludeAPI};

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

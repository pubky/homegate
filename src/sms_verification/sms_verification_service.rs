use crate::app_context::AppContext;
use crate::persistence::db::Db;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::{CheckCodeResponse, PreludeAPI, VerificationResponse};
use regex::Regex;

pub struct SmsVerificationService {
    prelude_api: PreludeAPI,
    db: Db,
}

impl SmsVerificationService {
    pub fn new(context: AppContext) -> Self {
        let prelude_api = PreludeAPI::new(&context);
        let db = context.db.clone();
        Self { prelude_api, db }
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

    /// Initiates a phone number verification process
    pub async fn verify_init(
        &self,
        phone_number: &str,
        ip_address: Option<&str>,
    ) -> Result<VerificationResponse, SmsVerificationError> {
        Self::validate_phone_number(phone_number)?;

        // TODO: Make database call to check/store verification attempt before calling PreludeAPI
        // This should:
        // - Check if phone number already has a pending verification
        // - Check rate limits for this phone number/IP
        // - Store the verification attempt in the database

        let response = self
            .prelude_api
            .create_verification(phone_number, ip_address)
            .await?;

        self.db.create_sms(phone_number, &response.id).await?;

        Ok(response)
    }

    /// Validates a verification code for a phone number
    pub async fn verify_finalise(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<CheckCodeResponse, SmsVerificationError> {
        Self::validate_phone_number(phone_number)?;

        // TODO: Make database call to check verification attempt before calling PreludeAPI
        // This should:
        // - Verify that a verification was initiated for this phone number
        // - Check that the verification hasn't expired
        // - Check rate limits for failed attempts

        let response = self.prelude_api.check_code(phone_number, code).await?;

        // If verification successful then update database
        if response.status == "success" {
            let signup_code = generate_signup_code();
            self.db.verify_sms(&response.id, &signup_code).await?;
        }

        Ok(response)
    }
}

fn generate_signup_code() -> String {
    todo!("Implement Base32 Crockford signup code generation")
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

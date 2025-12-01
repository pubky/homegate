use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::{CheckCodeResponse, VerificationResponse};
use async_trait::async_trait;

/// Trait for SMS verification provider API implementations
#[async_trait]
pub trait SmsVerificationProviderApi: Send + Sync {
    /// Creates a verification request for the given phone number
    async fn create_verification(
        &self,
        phone_number: &str,
        ip_address: Option<&str>,
    ) -> Result<VerificationResponse, SmsVerificationError>;

    /// Checks a verification code for the given phone number
    async fn check_code(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<CheckCodeResponse, SmsVerificationError>;
}

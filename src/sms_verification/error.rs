use thiserror::Error;

use crate::{
    infrastructure::database::DbError,
    sms_verification::{PhoneNumber, prelude_api::PreludeError},
};

#[derive(Error, Debug)]
pub enum SmsVerificationError {
    #[error(
        "Invalid phone number format: {0}. Phone number must be in E.164 format (e.g., +30123456789)"
    )]
    InvalidPhoneNumber(String),

    #[error("Invalid code format: {0}. Code must be exactly 6 digits (0-9)")]
    InvalidCode(String),

    #[error("Phone number has exceeded weekly verification limit (2 verifications per 7 days)")]
    WeeklyLimitExceeded,

    #[error("Phone number has exceeded annual verification limit (4 verifications per 365 days)")]
    AnnualLimitExceeded,

    #[error("No active verification session for phone number: {0}")]
    NoActiveVerification(PhoneNumber),

    #[error("External service rate limit exceeded")]
    RateLimited { retry_after: Option<u64> },

    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("{0}")]
    Database(#[from] DbError),
}

impl From<PreludeError> for SmsVerificationError {
    fn from(error: PreludeError) -> Self {
        match error {
            PreludeError::RateLimited { retry_after } => {
                SmsVerificationError::RateLimited { retry_after }
            }
            PreludeError::RequestFailed(e) => SmsVerificationError::RequestFailed(e),
        }
    }
}

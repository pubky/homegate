use thiserror::Error;

use crate::{
    infrastructure::sql::DbError,
    sms_verification::{PhoneNumber, prelude_api::PreludeError},
};

#[derive(Error, Debug)]
pub enum SmsVerificationError {
    #[error("Phone number blocked for verification")]
    Blocked,

    #[error("Phone number has exceeded weekly verification limit (2 verifications per 7 days)")]
    WeeklyLimitExceeded,

    #[error("Phone number has exceeded annual verification limit (4 verifications per 365 days)")]
    AnnualLimitExceeded,

    /// This can be either Homegate not having a PENDING entry in its table or Prelude expiring the verification request for this number
    /// Either way the user must start from the top.
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

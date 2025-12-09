use thiserror::Error;

use crate::infrastructure::database::DbError;

#[derive(Error, Debug)]
pub enum SmsVerificationError {
    #[error(
        "Invalid phone number format: {0}. Phone number must be in E.164 format (e.g., +30123456789)"
    )]
    InvalidPhoneNumber(String),

    #[error("Phone number has too many verified sessions")]
    TooManyVerifiedSessions,

    #[error("No active verification session for phone number: {0}")]
    NoActiveVerification(String),

    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("{0}")]
    Database(#[from] DbError),
}

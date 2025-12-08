use thiserror::Error;

use crate::sms_verification::repository::SmsVerificationRepositoryError;

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

    #[error("Database error: {0}")]
    DatabaseError(#[from] SmsVerificationRepositoryError),

    #[error("API error (status {status}): {message}")]
    ApiError { status: u16, message: String },

    #[error("Invalid response from API: {0}")]
    InvalidResponse(String),

    #[error("Homeserver admin API error: {0}")]
    HomeserverAdminError(#[from] crate::shared::HomeserverAdminApiError),
}

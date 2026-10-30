use thiserror::Error;

use crate::{
    infrastructure::sql::DbError,
    shared::{Dependency, FailureKind, RejectionReason},
    sms_verification::prelude_api::PreludeError,
};

#[derive(Error, Debug)]
pub enum SmsVerificationError {
    #[error("Phone number blocked for verification")]
    Blocked,

    #[error("Phone number has exceeded weekly verification limit")]
    WeeklyLimitExceeded,

    #[error("Phone number has exceeded annual verification limit")]
    AnnualLimitExceeded,

    /// This can be either Homegate not having a PENDING entry in its table or Prelude expiring the verification request for this number
    /// Either way the user must start from the top.
    #[error("No active verification session for phone number")]
    NoActiveVerification,

    #[error("Too many incorrect code attempts. Please request a new verification code.")]
    MaxValidationAttemptsExceeded,

    #[error("Invalid phone number format. Must be in E.164 format (e.g., +30123456789)")]
    InvalidPhoneNumber,

    #[error("External service rate limit exceeded")]
    RateLimited { retry_after: Option<u64> },

    #[error("Homeserver temporarily unavailable, please retry")]
    HomeserverUnavailable,

    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("{0}")]
    Database(#[from] DbError),
}

impl SmsVerificationError {
    /// How this error is counted in metrics.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Blocked => FailureKind::Rejected(RejectionReason::Blocked),
            Self::WeeklyLimitExceeded => {
                FailureKind::Rejected(RejectionReason::WeeklyLimitExceeded)
            }
            Self::AnnualLimitExceeded => {
                FailureKind::Rejected(RejectionReason::AnnualLimitExceeded)
            }
            Self::NoActiveVerification => {
                FailureKind::Rejected(RejectionReason::NoActiveVerification)
            }
            Self::MaxValidationAttemptsExceeded => {
                FailureKind::Rejected(RejectionReason::MaxValidationAttemptsExceeded)
            }
            Self::InvalidPhoneNumber => FailureKind::Rejected(RejectionReason::InvalidPhoneNumber),
            Self::RateLimited { .. } => FailureKind::Rejected(RejectionReason::PreludeRateLimited),
            Self::HomeserverUnavailable => FailureKind::NotCounted,
            Self::RequestFailed(_) => FailureKind::DependencyError(Dependency::Prelude),
            Self::Database(_) => FailureKind::NotCounted,
        }
    }
}

impl From<PreludeError> for SmsVerificationError {
    fn from(error: PreludeError) -> Self {
        match error {
            PreludeError::RateLimited { retry_after } => {
                SmsVerificationError::RateLimited { retry_after }
            }
            PreludeError::RegionBlocked => SmsVerificationError::Blocked,
            PreludeError::InvalidPhoneNumber => SmsVerificationError::InvalidPhoneNumber,
            PreludeError::RequestFailed(e) => SmsVerificationError::RequestFailed(e),
        }
    }
}

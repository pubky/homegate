use crate::infrastructure::sql::DbError;
use crate::shared::{Dependency, FailureKind, HasherBusy, RejectionReason, SignupIssuanceError};

#[derive(thiserror::Error, Debug)]
pub enum AppleVerificationError {
    #[error("invalid_apple_id_token")]
    InvalidAppleIdToken,

    #[error("weekly_limit_exceeded")]
    WeeklyLimitExceeded,

    #[error("annual_limit_exceeded")]
    AnnualLimitExceeded,

    #[error("homeserver_unavailable")]
    HomeserverUnavailable,

    #[error("apple_verifier_unavailable")]
    AppleVerifierUnavailable,

    #[error("server_busy")]
    ServerBusy(#[from] HasherBusy),

    #[error("internal_error")]
    Database(#[from] DbError),
}

impl AppleVerificationError {
    /// How this error is counted in metrics.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::InvalidAppleIdToken => {
                FailureKind::Rejected(RejectionReason::InvalidAppleIdToken)
            }
            Self::WeeklyLimitExceeded => {
                FailureKind::Rejected(RejectionReason::WeeklyLimitExceeded)
            }
            Self::AnnualLimitExceeded => {
                FailureKind::Rejected(RejectionReason::AnnualLimitExceeded)
            }
            Self::HomeserverUnavailable => FailureKind::NotCounted,
            Self::AppleVerifierUnavailable => FailureKind::DependencyError(Dependency::AppleJwks),
            Self::Database(_) => FailureKind::NotCounted,
            // Overload is the server's own state, not a client rejection; it
            // shows as a 503 in the request counter.
            Self::ServerBusy(_) => FailureKind::NotCounted,
        }
    }
}

impl From<SignupIssuanceError> for AppleVerificationError {
    fn from(error: SignupIssuanceError) -> Self {
        match error {
            SignupIssuanceError::WeeklyLimitExceeded => Self::WeeklyLimitExceeded,
            SignupIssuanceError::AnnualLimitExceeded => Self::AnnualLimitExceeded,
            SignupIssuanceError::HomeserverUnavailable => Self::HomeserverUnavailable,
            SignupIssuanceError::Database(error) => Self::Database(error),
        }
    }
}

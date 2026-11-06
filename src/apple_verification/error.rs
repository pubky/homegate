use crate::infrastructure::sql::DbError;
use crate::shared::SignupIssuanceError;

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

    #[error("internal_error")]
    Database(#[from] DbError),
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

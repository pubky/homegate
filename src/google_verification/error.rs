use crate::infrastructure::sql::DbError;
use crate::shared::SignupIssuanceError;

#[derive(thiserror::Error, Debug)]
pub enum GoogleVerificationError {
    #[error("invalid_request")]
    InvalidRequest,

    #[error("invalid_google_id_token")]
    InvalidGoogleIdToken,

    #[error("weekly_limit_exceeded")]
    WeeklyLimitExceeded,

    #[error("annual_limit_exceeded")]
    AnnualLimitExceeded,

    #[error("homeserver_unavailable")]
    HomeserverUnavailable,

    #[error("google_verifier_unavailable")]
    GoogleVerifierUnavailable,

    #[error("internal_error")]
    Database(#[from] DbError),
}

impl From<SignupIssuanceError> for GoogleVerificationError {
    fn from(error: SignupIssuanceError) -> Self {
        match error {
            SignupIssuanceError::WeeklyLimitExceeded => Self::WeeklyLimitExceeded,
            SignupIssuanceError::AnnualLimitExceeded => Self::AnnualLimitExceeded,
            SignupIssuanceError::HomeserverUnavailable => Self::HomeserverUnavailable,
            SignupIssuanceError::Database(error) => Self::Database(error),
        }
    }
}

use crate::infrastructure::sql::DbError;

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

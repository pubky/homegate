use thiserror::Error;

use crate::infrastructure::sql::DbError;
use crate::shared::SignupIssuanceError;

#[derive(Error, Debug)]
pub enum IpVerificationError {
    #[error("IP address has exceeded weekly verification limit")]
    WeeklyLimitExceeded,

    #[error("IP address has exceeded annual verification limit")]
    AnnualLimitExceeded,

    #[error("Could not determine client IP address")]
    IpAddressRequired,

    #[error("Homeserver temporarily unavailable, please retry")]
    HomeserverUnavailable,

    #[error("{0}")]
    Database(#[from] DbError),
}

impl From<SignupIssuanceError> for IpVerificationError {
    fn from(error: SignupIssuanceError) -> Self {
        match error {
            SignupIssuanceError::WeeklyLimitExceeded => Self::WeeklyLimitExceeded,
            SignupIssuanceError::AnnualLimitExceeded => Self::AnnualLimitExceeded,
            SignupIssuanceError::HomeserverUnavailable => Self::HomeserverUnavailable,
            SignupIssuanceError::Database(error) => Self::Database(error),
        }
    }
}

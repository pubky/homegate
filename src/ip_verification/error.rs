use thiserror::Error;

use crate::infrastructure::sql::DbError;

#[derive(Error, Debug)]
pub enum IpVerificationError {
    #[error("IP address has exceeded weekly verification limit")]
    WeeklyLimitExceeded,

    #[error("IP address has exceeded annual verification limit")]
    AnnualLimitExceeded,

    #[error("Could not determine client IP address")]
    IpAddressRequired,

    #[error("IP verification is not enabled")]
    ServiceDisabled,

    #[error("Homeserver temporarily unavailable, please retry")]
    HomeserverUnavailable,

    #[error("{0}")]
    Database(#[from] DbError),
}

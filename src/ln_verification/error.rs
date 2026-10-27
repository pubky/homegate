use crate::ln_verification::phoenixd_api::PhoenixdError;
use crate::shared::{Dependency, FailureKind};

#[derive(thiserror::Error, Debug)]
pub enum LnVerificationError {
    #[error("Phoenixd: {0}")]
    Phoenixd(#[from] PhoenixdError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Homeserver temporarily unavailable, please retry")]
    HomeserverUnavailable,
}

impl LnVerificationError {
    /// How this error is counted in metrics.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Phoenixd(_) => FailureKind::DependencyError(Dependency::Phoenixd),
            Self::Database(_) | Self::HomeserverUnavailable => FailureKind::NotCounted,
        }
    }
}

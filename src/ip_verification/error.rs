use thiserror::Error;

use crate::infrastructure::sql::DbError;
use crate::shared::{Dependency, FailureKind, RejectionReason, SignupIssuanceError};

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

impl IpVerificationError {
    /// How this error is counted in metrics.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::WeeklyLimitExceeded => {
                FailureKind::Rejected(RejectionReason::WeeklyLimitExceeded)
            }
            Self::AnnualLimitExceeded => {
                FailureKind::Rejected(RejectionReason::AnnualLimitExceeded)
            }
            Self::IpAddressRequired => FailureKind::Rejected(RejectionReason::IpAddressRequired),
            Self::HomeserverUnavailable => FailureKind::DependencyError(Dependency::Homeserver),
            Self::Database(_) => FailureKind::Internal,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::SignupType;

    #[test]
    fn rejection_reasons_are_pre_created_for_ip() {
        let errors = [
            IpVerificationError::WeeklyLimitExceeded,
            IpVerificationError::AnnualLimitExceeded,
            IpVerificationError::IpAddressRequired,
            IpVerificationError::HomeserverUnavailable,
        ];
        for error in errors {
            if let FailureKind::Rejected(reason) = error.failure_kind() {
                assert!(
                    RejectionReason::for_signup_type(SignupType::Ip).contains(&reason),
                    "{reason:?} missing from RejectionReason::for_signup_type(Ip)"
                );
            }
        }
    }
}

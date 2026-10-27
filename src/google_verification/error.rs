use crate::infrastructure::sql::DbError;
use crate::shared::{Dependency, FailureKind, RejectionReason, SignupIssuanceError};

#[derive(thiserror::Error, Debug)]
pub enum GoogleVerificationError {
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

impl GoogleVerificationError {
    /// How this error is counted in metrics.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::InvalidGoogleIdToken => {
                FailureKind::Rejected(RejectionReason::InvalidGoogleIdToken)
            }
            Self::WeeklyLimitExceeded => {
                FailureKind::Rejected(RejectionReason::WeeklyLimitExceeded)
            }
            Self::AnnualLimitExceeded => {
                FailureKind::Rejected(RejectionReason::AnnualLimitExceeded)
            }
            Self::HomeserverUnavailable => FailureKind::DependencyError(Dependency::Homeserver),
            Self::GoogleVerifierUnavailable => FailureKind::DependencyError(Dependency::GoogleJwks),
            Self::Database(_) => FailureKind::Internal,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::SignupType;

    #[test]
    fn rejection_reasons_are_pre_created_for_google() {
        let errors = [
            GoogleVerificationError::InvalidGoogleIdToken,
            GoogleVerificationError::WeeklyLimitExceeded,
            GoogleVerificationError::AnnualLimitExceeded,
            GoogleVerificationError::HomeserverUnavailable,
            GoogleVerificationError::GoogleVerifierUnavailable,
        ];
        for error in errors {
            if let FailureKind::Rejected(reason) = error.failure_kind() {
                assert!(
                    RejectionReason::for_signup_type(SignupType::Google).contains(&reason),
                    "{reason:?} missing from RejectionReason::for_signup_type(Google)"
                );
            }
        }
    }
}

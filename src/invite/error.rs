use thiserror::Error;

use crate::infrastructure::sql::DbError;

#[derive(Error, Debug)]
pub enum InviteError {
    #[error("Invalid pubkey format")]
    InvalidPubkey,

    #[error("Weekly invite limit exceeded")]
    WeeklyLimitExceeded,

    #[error("Annual invite limit exceeded")]
    AnnualLimitExceeded,

    #[error("Proof file not found at pubky path /homegate/proof")]
    ProofNotFound,

    #[error("Proof verification failed: hash does not match")]
    ProofMismatch,

    #[error("Homeserver temporarily unavailable, please retry")]
    HomeserverUnavailable,

    #[error("{0}")]
    Database(#[from] DbError),

    #[error("Database transaction error: {0}")]
    Transaction(#[from] sqlx::Error),
}

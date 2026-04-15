use thiserror::Error;

use crate::infrastructure::sql::DbError;

#[derive(Error, Debug)]
pub enum InviteError {
    #[error("Invalid pubkey format")]
    InvalidPubkey,

    #[error("Invalid hex encoding in hashProofPreimage")]
    InvalidPreimage,

    #[error("You have used all invite codes available this week.")]
    WeeklyLimitExceeded,

    #[error("You have used all invite codes available this year.")]
    AnnualLimitExceeded,

    #[error("You need to post more before you are eligible for an invite code.")]
    InsufficientPosts,

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

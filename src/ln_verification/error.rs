use crate::ln_verification::phoenixd_api::PhoenixdError;

#[derive(thiserror::Error, Debug)]
pub enum LnVerificationError {
    #[error("Phoenixd: {0}")]
    Phoenixd(#[from] PhoenixdError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Homeserver API error: {0}")]
    Homeserver(reqwest::Error),
}

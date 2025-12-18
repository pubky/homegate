use crate::ln_verification::phoenixd_api::WebsocketError;

#[derive(thiserror::Error, Debug)]
pub enum LnVerificationError {
    #[error("Phoenixd API error: {0}")]
    Phoenixd(#[from] reqwest::Error),

    #[error("Phoenixd websocket error: {0:?}")]
    PhoenixdWebsocket(#[from] WebsocketError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Homeserver API error: {0}")]
    Homeserver(reqwest::Error),
}

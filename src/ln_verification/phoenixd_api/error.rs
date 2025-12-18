use thiserror::Error;

/// Errors for the Phoenixd API
#[derive(Error, Debug)]
pub enum PhoenixdError {
    #[error("Api: {0:?}")]
    Api(#[from] reqwest::Error),

    #[error("Websocket: {0:?}")]
    Websocket(#[from] reqwest_websocket::Error),

    #[error("Deserialization: {0}")]
    Deserialization(#[from] serde_json::Error),
}

use thiserror::Error;

use crate::infrastructure::database::DbError;

#[derive(Error, Debug)]
pub enum HttpServerError {
    #[error("Database connection failed: {0}")]
    Database(#[from] DbError),

    #[error("Router initialization failed")]
    RouterInit,

    #[error("HTTP server failed to bind or start: {0}")]
    ServerStart(#[from] std::io::Error),
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Database item not found: {0}")]
    NotFound(String),
}

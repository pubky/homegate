use sqlx::postgres::PgPool;

use crate::infrastructure::sql::{Migrator, connection_string::ConnectionString};

/// The SqlDb is a wrapper around the postgres connection pool.
/// It is used to connect to the database and run queries.
///
/// It is cheaply cloneable. Internally,
/// the connection pool is simply a reference-counted handle to the inner pool state.
/// When the last remaining handle to the pool is dropped,
/// the connections owned by the pool are immediately closed (also by dropping).
/// See https://docs.rs/sqlx/latest/sqlx/struct.Pool.html
#[derive(Clone)]
pub struct SqlDb {
    /// Connection pool to the database
    pool: PgPool,
}

impl std::fmt::Debug for SqlDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DbConnection")
    }
}

impl SqlDb {
    /// Connect to the database.
    pub async fn connect(con_string: &ConnectionString) -> Result<Self, sqlx::Error> {
        let pool: PgPool = PgPool::connect(con_string.as_str()).await?;
        let db = Self { pool };
        Migrator::run(&db)
            .await
            .map_err(|e| sqlx::Error::Protocol(format!("Migration failed: {}", e)))?;

        Ok(db)
    }

    /// Get the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a test database without running migrations
    /// If the DB_CONNECTION_STRING environment variable is not set, a temporary directory is used for the sqlite database
    /// If the DB_CONNECTION_STRING environment variable is set, the test database is created on the existing database
    #[cfg(test)]
    pub async fn test_without_migrations(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a test database and run migrations
    /// If the DB_CONNECTION_STRING environment variable is not set, a temporary directory is used for the sqlite database
    /// If the DB_CONNECTION_STRING environment variable is set, the migrations are run on the existing database
    #[cfg(test)]
    pub async fn test(pool: PgPool) -> Self {
        use crate::infrastructure::sql::migrator;
        let db = Self::test_without_migrations(pool).await;
        migrator::Migrator::<'_>::run(&db)
            .await
            .expect("Failed to run migrations");
        db
    }
}

impl From<PgPool> for SqlDb {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn test_pg_db_available(pool: PgPool) {
        let _db = SqlDb::test_without_migrations(pool).await;
    }
}

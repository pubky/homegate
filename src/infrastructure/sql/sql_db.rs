use sqlx::postgres::PgPool;

use crate::infrastructure::sql::connection_string::ConnectionString;

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
    /// Note: Migrations are not run automatically. Each module is responsible
    /// for running its own migrations during initialization.
    pub async fn connect(con_string: &ConnectionString) -> Result<Self, sqlx::Error> {
        let pool: PgPool = PgPool::connect(con_string.as_str()).await?;
        Ok(Self { pool })
    }

    /// Get the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a test database without running migrations.
    /// Each module test should run its own migrations after calling this.
    #[cfg(test)]
    pub async fn test_without_migrations(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a test database and run the provided migrations.
    /// Use this in module tests by passing the module's migrations.
    #[cfg(test)]
    pub async fn test_with_migrations(
        pool: PgPool,
        migrations: Vec<Box<dyn crate::infrastructure::sql::MigrationTrait>>,
    ) -> Self {
        let db = Self::test_without_migrations(pool).await;
        crate::infrastructure::sql::Migrator::run(&db, migrations)
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

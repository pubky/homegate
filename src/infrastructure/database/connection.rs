#![allow(unexpected_cfgs)]

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;

/// A connection string for a postgres database.
/// See https://www.postgresql.org/docs/current/libpq-connect.html#LIBPQ-CONNSTRING-URIS
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionString(url::Url);

impl ConnectionString {
    /// Create a new connection string from a string.
    pub fn new(con_string: &str) -> anyhow::Result<Self> {
        let con = Self(url::Url::parse(con_string)?);
        if !con.is_postgres() {
            anyhow::bail!("Only postgres database urls are supported");
        }
        Ok(con)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn is_postgres(&self) -> bool {
        self.0.scheme() == "postgres" || self.0.scheme() == "postgresql"
    }

    /// Get the database name
    /// For postgres, this is the database name directly
    pub fn database_name(&self) -> &str {
        self.0.path().trim_start_matches("/")
    }

    /// Set the database name
    pub fn set_database_name(&mut self, db_name: &str) {
        self.0.set_path(db_name);
    }
}

#[cfg_attr(not(test), allow(unexpected_cfgs))]
#[cfg(any(test, feature = "testing"))]
impl ConnectionString {
    /// Returns a connection string for a test database.
    /// This is a postgres database that is not real.
    /// It is used as an indicator for a empheral test database.
    pub fn default_test_db() -> Self {
        Self::new("postgres://localhost:5432/postgres?pubky-test=true").unwrap()
    }

    pub fn is_test_db(&self) -> bool {
        self.0
            .query_pairs()
            .any(|(key, value)| key == "pubky-test" && value == "true")
    }
}

impl From<url::Url> for ConnectionString {
    fn from(url: url::Url) -> Self {
        Self(url)
    }
}

impl FromStr for ConnectionString {
    type Err = url::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(url::Url::parse(s)?))
    }
}

impl Display for ConnectionString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for ConnectionString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConnectionString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl Default for ConnectionString {
    fn default() -> Self {
        Self::new("postgres://localhost:5432/pubky_homegate").unwrap()
    }
}

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
    pool: PgPool,
}

impl std::fmt::Debug for SqlDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DbConnection")
    }
}

impl SqlDb {
    /// Connect to the database and run migrations.
    pub async fn connect(con_string: &ConnectionString) -> Result<Self, sqlx::Error> {
        let pool: PgPool = PgPool::connect(con_string.as_str()).await?;
        let db = Self { pool };

        use crate::infrastructure::database::migrations::Migrator;
        Migrator::run(&db)
            .await
            .map_err(|e| sqlx::Error::Protocol(format!("Migration failed: {}", e)))?;

        Ok(db)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a test database without running migrations
    pub async fn test_without_migrations(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a test database and run migrations
    pub async fn test(pool: PgPool) -> Self {
        use crate::infrastructure::database::migrations::Migrator;
        let db = Self::test_without_migrations(pool).await;
        Migrator::run(&db).await.expect("Failed to run migrations");
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

    #[tokio::test]
    async fn test_create_db() {
        let con_strings = vec!["postgres://localhost:5432/pubky_homeserver"];
        for con_string in con_strings {
            let _: ConnectionString = con_string.parse().unwrap();
        }
    }

    #[sqlx::test]
    async fn test_pg_db_available(pool: PgPool) {
        let _db = SqlDb::test_without_migrations(pool).await;
    }
}

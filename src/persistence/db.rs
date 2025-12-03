use chrono::{DateTime, Utc};
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;
use thiserror::Error;

use crate::persistence::sql::{Migrator, SqlDb};

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SMS verification not found: {0}")]
    NotFound(String),
    #[error("Phone number already verified")]
    AlreadyVerified,
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Query building error: {0}")]
    QueryBuild(String),
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct SmsVerification {
    pub unique_id: i32,
    pub phone_number: String,
    pub prelude_id: String,
    pub created_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
    pub signup_code: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct Db {
    sql_db: SqlDb,
}

impl Db {
    /// Connect to the database and run migrations.
    pub async fn connect(
        connection_string: &crate::persistence::sql::ConnectionString,
    ) -> Result<Self, DbError> {
        let sql_db = SqlDb::connect(connection_string)
            .await
            .map_err(DbError::Database)?;

        let migrator = Migrator::new(&sql_db);
        migrator
            .run()
            .await
            .map_err(|e| DbError::Database(sqlx::Error::Protocol(e.to_string())))?;

        Ok(Self { sql_db })
    }

    /// Create a Db from an existing connection pool and run migrations.
    /// This is primarily used in tests with #[sqlx::test] fixtures.
    pub async fn from_pool(pool: sqlx::PgPool) -> Result<Self, DbError> {
        let sql_db = SqlDb::from(pool);
        let migrator = Migrator::new(&sql_db);
        migrator
            .run()
            .await
            .map_err(|e| DbError::Database(sqlx::Error::Protocol(e.to_string())))?;

        Ok(Self { sql_db })
    }

    /// Get access to the underlying connection pool.
    /// This is primarily used in tests for direct database queries.
    #[cfg(test)]
    pub fn pool(&self) -> &sqlx::PgPool {
        self.sql_db.pool()
    }

    /// Create a new SMS verification record only if no active session exists for this phone_number and prelude_id
    pub async fn create_sms(&self, phone_number: &str, prelude_id: &str) -> Result<(), DbError> {
        // Build subquery to check for existing active sessions
        let subquery = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("verified_at").is_null())
            .to_owned();

        // Build INSERT statement with NOT EXISTS condition
        let statement = Query::insert()
            .into_table("sms_verifications")
            .columns(["phone_number", "prelude_id"])
            .select_from(
                Query::select()
                    .expr(Expr::value(phone_number))
                    .expr(Expr::value(prelude_id))
                    .cond_where(Expr::exists(subquery).not())
                    .to_owned(),
            )
            .map_err(|e| DbError::QueryBuild(format!("Failed to build insert query: {}", e)))?
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(self.sql_db.pool())
            .await?;

        Ok(())
    }

    pub async fn count_verified_sessions(&self, phone_number: &str) -> Result<i64, DbError> {
        let statement = Query::select()
            .expr(Expr::col("unique_id").count())
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("verified_at").is_not_null())
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(self.sql_db.pool())
            .await?;
        let count: i64 = row.try_get(0)?;
        Ok(count)
    }

    /// Verify an SMS by setting verified_at and signup_code
    pub async fn verify_sms(&self, prelude_id: &str, signup_code: &str) -> Result<(), DbError> {
        // First, check if a verification exists and whether it's already verified
        // TODO: we can do this check within the update query
        let check_statement = Query::select()
            .column("verified_at")
            .from("sms_verifications")
            .and_where(Expr::col("prelude_id").eq(prelude_id))
            .to_owned();

        let (check_query, check_values) = check_statement.build_sqlx(PostgresQueryBuilder);
        let row_result = sqlx::query_with(&check_query, check_values)
            .fetch_optional(self.sql_db.pool())
            .await?;

        match row_result {
            None => {
                return Err(DbError::NotFound(prelude_id.to_string()));
            }
            Some(row) => {
                let verified_at: Option<DateTime<Utc>> = row.try_get("verified_at")?;
                if verified_at.is_some() {
                    return Err(DbError::AlreadyVerified);
                }
            }
        }

        // Update the verification record
        let update_statement = Query::update()
            .table("sms_verifications")
            .values([
                ("verified_at", Expr::current_timestamp().into()),
                ("signup_code", signup_code.as_bytes().to_vec().into()),
            ])
            .and_where(Expr::col("prelude_id").eq(prelude_id))
            .and_where(Expr::col("verified_at").is_null())
            .to_owned();

        let (query, values) = update_statement.build_sqlx(PostgresQueryBuilder);
        let result = sqlx::query_with(&query, values)
            .execute(self.sql_db.pool())
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(prelude_id.to_string()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn test_one_active_session_per_phone(pool: PgPool) {
        let db = Db::from_pool(pool).await.unwrap();
        let phone = "+30123456789";

        // Try to verify without creating first
        let result = db
            .verify_sms("nonexistent_prelude_id", "test_signup_code")
            .await;
        assert!(matches!(result, Err(DbError::NotFound(_))));

        // Create first verification
        db.create_sms(phone, "prelude_id_1").await.unwrap();

        // Try to create second verification before first is verified - should be ignored
        db.create_sms(phone, "prelude_id_2").await.unwrap();

        // Should only have one record (second was not created)
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count.0, 1, "Should only have 1 active session");

        // Verify the record is the first one
        let prelude_id: (String,) =
            sqlx::query_as("SELECT prelude_id FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            prelude_id.0, "prelude_id_1",
            "Should keep the first prelude_id"
        );

        // Verify the first session
        db.verify_sms("prelude_id_1", "signup_code_1")
            .await
            .unwrap();

        // Try to verify again - should fail
        let result = db.verify_sms("prelude_id_1", "signup_code_1").await;
        assert!(matches!(result, Err(DbError::AlreadyVerified)));

        // Should be able to create a new verification after the first is verified
        db.create_sms(phone, "prelude_id_3").await.unwrap();

        // Should now have 2 records total
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count.0, 2, "Should have 2 records after verification");
    }

    #[sqlx::test]
    async fn test_count_verified_sessions_mixed(pool: PgPool) {
        let db = Db::from_pool(pool).await.unwrap();
        let phone = "+30666666666";

        let count = db.count_verified_sessions(phone).await.unwrap();
        assert_eq!(count, 0);

        db.create_sms(phone, "prelude_1").await.unwrap();
        db.verify_sms("prelude_1", "code_1").await.unwrap();

        db.create_sms(phone, "prelude_2").await.unwrap();
        db.verify_sms("prelude_2", "code_2").await.unwrap();

        db.create_sms(phone, "prelude_3").await.unwrap();
        // Don't verify prelude_3

        let count = db.count_verified_sessions(phone).await.unwrap();
        assert_eq!(count, 2, "Should only count verified sessions");
    }
}

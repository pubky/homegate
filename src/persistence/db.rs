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
    #[error("No active verification session for phone number: {0}")]
    NoActiveVerification(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Query building error: {0}")]
    QueryBuild(String),
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum VerificationStatus {
    #[sqlx(rename = "PENDING")]
    Pending,
    #[sqlx(rename = "VERIFIED")]
    Verified,
    #[sqlx(rename = "FAILED")]
    Failed,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationStatus::Pending => "PENDING",
            VerificationStatus::Verified => "VERIFIED",
            VerificationStatus::Failed => "FAILED",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct SmsVerification {
    pub unique_id: i32,
    pub phone_number: String,
    pub prelude_id: String,
    pub created_at: DateTime<Utc>,
    pub finalised_at: Option<DateTime<Utc>>,
    pub signup_code: Option<Vec<u8>>,
    pub status: VerificationStatus,
    pub failure_reason: Option<String>,
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

    /// Create a new SMS verification record only if no pending session exists for this phone_number
    pub async fn create_sms(&self, phone_number: &str, prelude_id: &str) -> Result<(), DbError> {
        // Build subquery to check for existing pending sessions
        let subquery = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str()))
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
            .and_where(Expr::col("status").eq(VerificationStatus::Verified.as_str()))
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(self.sql_db.pool())
            .await?;
        let count: i64 = row.try_get(0)?;
        Ok(count)
    }

    /// Check if an active (pending) verification session exists for a phone number.
    pub async fn check_pending_verification_exists(
        &self,
        phone_number: &str,
    ) -> Result<(), DbError> {
        let statement = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str()))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row_result = sqlx::query_with(&query, values)
            .fetch_optional(self.sql_db.pool())
            .await?;

        match row_result {
            Some(_) => Ok(()),
            None => Err(DbError::NoActiveVerification(phone_number.to_string())),
        }
    }

    /// Verify an SMS by setting finalised_at, status, and signup_code
    pub async fn verify_sms(&self, prelude_id: &str, signup_code: &str) -> Result<(), DbError> {
        // Update the verification record
        let update_statement = Query::update()
            .table("sms_verifications")
            .values([
                ("finalised_at", Expr::current_timestamp().into()),
                ("signup_code", signup_code.as_bytes().to_vec().into()),
                ("status", VerificationStatus::Verified.as_str().into()),
            ])
            .and_where(Expr::col("prelude_id").eq(prelude_id))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str())) // Safety: only update if pending
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

    /// Mark an SMS verification as failed
    pub async fn mark_verification_failed(
        &self,
        prelude_id: &str,
        failure_reason: &str,
    ) -> Result<(), DbError> {
        let update_statement = Query::update()
            .table("sms_verifications")
            .values([
                ("status", VerificationStatus::Failed.as_str().into()),
                ("finalised_at", Expr::current_timestamp().into()),
                ("failure_reason", failure_reason.into()),
            ])
            .and_where(Expr::col("prelude_id").eq(prelude_id))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str())) // Safety: only update if pending
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

        // Try to verify again - should fail with NotFound (0 rows affected due to verified_at IS NULL check)
        let result = db.verify_sms("prelude_id_1", "signup_code_1").await;
        assert!(matches!(result, Err(DbError::NotFound(_))));

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

    #[sqlx::test]
    async fn test_check_verification_exists(pool: PgPool) {
        let db = Db::from_pool(pool).await.unwrap();
        let phone = "+30123456789";

        // Case 1: No verification exists
        let result = db.check_pending_verification_exists(phone).await;
        assert!(matches!(result, Err(DbError::NoActiveVerification(_))));

        // Case 2: Active verification exists
        db.create_sms(phone, "prelude_123").await.unwrap();
        let result = db.check_pending_verification_exists(phone).await;
        assert!(result.is_ok());

        // Case 3: Verification exists but already verified
        db.verify_sms("prelude_123", "signup_code").await.unwrap();
        let result = db.check_pending_verification_exists(phone).await;
        assert!(matches!(result, Err(DbError::NoActiveVerification(_))));

        // Case 4: New active verification after first was verified
        db.create_sms(phone, "prelude_456").await.unwrap();
        let result = db.check_pending_verification_exists(phone).await;
        assert!(result.is_ok());
    }

    #[sqlx::test]
    async fn test_mark_failed_prevents_duplicate_marking(pool: PgPool) {
        let db = Db::from_pool(pool).await.unwrap();
        let phone = "+30444444444";

        // Create and mark as failed
        db.create_sms(phone, "prelude_dup").await.unwrap();
        db.mark_verification_failed("prelude_dup", "expired_or_not_found")
            .await
            .unwrap();

        // Try to mark again - should fail with NotFound (0 rows affected)
        let result = db
            .mark_verification_failed("prelude_dup", "expired_or_not_found")
            .await;
        assert!(matches!(result, Err(DbError::NotFound(_))));
    }
}

use crate::infrastructure::database::DbError;
use crate::infrastructure::database::SqlDb;
use chrono::{DateTime, Utc};
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SmsVerificationRepositoryError {
    #[error("SMS verification not found: {0}")]
    NotFound(String),

    #[error("No active verification session for phone number: {0}")]
    NoActiveVerification(String),

    #[error("{0}")]
    DatabaseError(#[from] DbError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
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
pub struct SmsVerificationRepository {
    db: SqlDb,
}

impl SmsVerificationRepository {
    pub fn new(db: SqlDb) -> Self {
        Self { db }
    }

    /// Create a new SMS verification record only if no pending session exists for this phone_number
    pub async fn create_verification(
        &self,
        phone_number: &str,
        prelude_id: &str,
    ) -> Result<(), SmsVerificationRepositoryError> {
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
            .execute(self.db.pool())
            .await?;

        Ok(())
    }

    pub async fn count_verified_sessions(
        &self,
        phone_number: &str,
    ) -> Result<i64, SmsVerificationRepositoryError> {
        let statement = Query::select()
            .expr(Expr::col("unique_id").count())
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("status").eq(VerificationStatus::Verified.as_str()))
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(self.db.pool())
            .await?;
        let count: i64 = row.try_get(0)?;
        Ok(count)
    }

    /// Check if an active (pending) verification session exists for a phone number.
    pub async fn check_pending_exists(
        &self,
        phone_number: &str,
    ) -> Result<(), SmsVerificationRepositoryError> {
        let statement = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number").eq(phone_number))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str()))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row_result = sqlx::query_with(&query, values)
            .fetch_optional(self.db.pool())
            .await?;

        match row_result {
            Some(_) => Ok(()),
            None => Err(SmsVerificationRepositoryError::NoActiveVerification(
                phone_number.to_string(),
            )),
        }
    }

    /// Verify an SMS by setting finalised_at, status, and signup_code
    pub async fn mark_verified(
        &self,
        prelude_id: &str,
        signup_code: &str,
    ) -> Result<(), SmsVerificationRepositoryError> {
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
            .execute(self.db.pool())
            .await?;

        if result.rows_affected() == 0 {
            return Err(SmsVerificationRepositoryError::NotFound(
                prelude_id.to_string(),
            ));
        }

        Ok(())
    }

    /// Mark an SMS verification as failed
    pub async fn mark_failed(
        &self,
        prelude_id: &str,
        failure_reason: &str,
    ) -> Result<(), SmsVerificationRepositoryError> {
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
            .execute(self.db.pool())
            .await?;

        if result.rows_affected() == 0 {
            return Err(SmsVerificationRepositoryError::NotFound(
                prelude_id.to_string(),
            ));
        }

        Ok(())
    }
}

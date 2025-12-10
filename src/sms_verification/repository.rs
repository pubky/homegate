use crate::infrastructure::database::DbError;
use crate::infrastructure::database::SqlDb;
use crate::sms_verification::hasher_argon2id::HasherArgon2id;
use crate::sms_verification::phone_number::PhoneNumber;
use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;

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
pub struct SmsVerificationEntity {
    pub id: i32,
    pub phone_number_hash: String,
    pub prelude_id: String,
    pub created_at: NaiveDateTime,
    pub finalised_at: Option<NaiveDateTime>,
    pub signup_code: Option<String>,
    pub status: VerificationStatus,
    pub failure_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SmsVerificationRepository {
    db: SqlDb,
    hasher_argon2id: HasherArgon2id,
}

impl SmsVerificationRepository {
    pub fn new(db: SqlDb, hasher_argon2id: HasherArgon2id) -> Self {
        Self {
            db,
            hasher_argon2id,
        }
    }

    /// Create a new SMS verification record only if no pending session exists for this phone_number
    pub async fn create_verification(
        &self,
        phone_number: &PhoneNumber,
        prelude_id: &str,
    ) -> Result<(), DbError> {
        let hashed_phone = self
            .hasher_argon2id
            .hash_phone_number(phone_number.as_str())?;

        // Build subquery to check for existing pending sessions
        let subquery = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number_hash").eq(hashed_phone.as_str()))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str()))
            .to_owned();

        // Build INSERT statement with NOT EXISTS condition
        let statement = Query::insert()
            .into_table("sms_verifications")
            .columns(["phone_number_hash", "prelude_id"])
            .select_from(
                Query::select()
                    .expr(Expr::value(hashed_phone.as_str()))
                    .expr(Expr::value(prelude_id))
                    .cond_where(Expr::exists(subquery).not())
                    .to_owned(),
            )
            .expect("Failed to build insert query")
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(self.db.pool())
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    pub async fn count_verified_sessions(
        &self,
        phone_number: &PhoneNumber,
    ) -> Result<i64, DbError> {
        let hashed_phone = self
            .hasher_argon2id
            .hash_phone_number(phone_number.as_str())?;

        let statement = Query::select()
            .expr(Expr::col("id").count())
            .from("sms_verifications")
            .and_where(Expr::col("phone_number_hash").eq(hashed_phone.as_str()))
            .and_where(Expr::col("status").eq(VerificationStatus::Verified.as_str()))
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(self.db.pool())
            .await
            .map_err(DbError::from)?;
        let count: i64 = row.try_get(0).map_err(DbError::from)?;
        Ok(count)
    }

    /// Error if no active (pending) verification session exists for a phone number.
    pub async fn err_if_no_active_verification(
        &self,
        phone_number: &PhoneNumber,
    ) -> Result<(), DbError> {
        let hashed_phone = self
            .hasher_argon2id
            .hash_phone_number(phone_number.as_str())?;

        let statement = Query::select()
            .expr(Expr::value(1))
            .from("sms_verifications")
            .and_where(Expr::col("phone_number_hash").eq(hashed_phone.as_str()))
            .and_where(Expr::col("status").eq(VerificationStatus::Pending.as_str()))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row_result = sqlx::query_with(&query, values)
            .fetch_optional(self.db.pool())
            .await
            .map_err(DbError::from)?;

        match row_result {
            Some(_) => Ok(()),
            None => Err(DbError::NotFound(phone_number.to_string())),
        }
    }

    /// Verify an SMS by setting finalised_at, status, and signup_code
    pub async fn mark_verified(&self, prelude_id: &str, signup_code: &str) -> Result<(), DbError> {
        // Update the verification record
        let update_statement = Query::update()
            .table("sms_verifications")
            .values([
                ("finalised_at", Expr::current_timestamp().into()),
                ("signup_code", signup_code.into()),
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
            return Err(DbError::NotFound(prelude_id.to_string()));
        }

        Ok(())
    }

    pub async fn mark_failed(&self, prelude_id: &str, failure_reason: &str) -> Result<(), DbError> {
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
            return Err(DbError::NotFound(prelude_id.to_string()));
        }

        Ok(())
    }

    /// Fetch a verification record by phone number (for testing/inspection)
    #[cfg(test)]
    pub async fn get_by_phone_number(
        &self,
        phone_number: &PhoneNumber,
    ) -> Result<SmsVerificationEntity, DbError> {
        let hashed_phone = self
            .hasher_argon2id
            .hash_phone_number(phone_number.as_str())?;

        let statement = Query::select()
            .columns([
                "id",
                "phone_number_hash",
                "prelude_id",
                "created_at",
                "finalised_at",
                "signup_code",
                "status",
                "failure_reason",
            ])
            .from("sms_verifications")
            .and_where(Expr::col("phone_number_hash").eq(hashed_phone.as_str()))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_as_with(&query, values)
            .fetch_one(self.db.pool())
            .await
            .map_err(DbError::from)
    }

    /// Fetch a verification record by prelude_id (for testing/inspection)
    #[cfg(test)]
    pub async fn get_by_prelude_id(
        &self,
        prelude_id: &str,
    ) -> Result<SmsVerificationEntity, DbError> {
        let statement = Query::select()
            .columns([
                "id",
                "phone_number_hash",
                "prelude_id",
                "created_at",
                "finalised_at",
                "signup_code",
                "status",
                "failure_reason",
            ])
            .from("sms_verifications")
            .and_where(Expr::col("prelude_id").eq(prelude_id))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_as_with(&query, values)
            .fetch_one(self.db.pool())
            .await
            .map_err(DbError::from)
    }
}

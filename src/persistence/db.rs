use chrono::{DateTime, Utc};
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;
use thiserror::Error;

use crate::persistence::sql::SqlDb;

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
    pub fn new(sql_db: SqlDb) -> Self {
        Self { sql_db }
    }

    /// Create a new SMS verification record
    /// Perhaps dont need phone_number here
    pub async fn create_sms(&self, phone_number: &str, prelude_id: &str) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table("sms_verifications")
            .columns(["phone_number", "prelude_id"])
            .values([phone_number.into(), prelude_id.into()])
            .map_err(|e| DbError::QueryBuild(format!("Failed to build insert query: {}", e)))?
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(self.sql_db.pool())
            .await?;

        Ok(())
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
    async fn test_create_multiple_sms_same_phone(pool: PgPool) {
        let sql_db = SqlDb::test(pool).await;
        let db = Db::new(sql_db);

        // Should be able to create multiple verifications for same phone with different prelude_ids
        db.create_sms("+30123456789", "prelude_id_1").await.unwrap();
        let result = db.create_sms("+30123456789", "prelude_id_2").await;
        assert!(result.is_ok());
    }

    #[sqlx::test]
    async fn test_full_flow(pool: PgPool) {
        let sql_db = SqlDb::test(pool).await;
        let db = Db::new(sql_db);

        // Try to verify without creating
        let result = db
            .verify_sms("nonexistent_prelude_id", "test_signup_code")
            .await;
        assert!(matches!(result, Err(DbError::NotFound(_))));

        // Create verification
        db.create_sms("+30123456789", "prelude_id_xyz")
            .await
            .unwrap();

        // Verify
        db.verify_sms("prelude_id_xyz", "signup_code_123")
            .await
            .unwrap();

        // Verify again should fail
        let result = db.verify_sms("prelude_id_xyz", "signup_code_123").await;
        assert!(matches!(result, Err(DbError::AlreadyVerified)));
    }
}

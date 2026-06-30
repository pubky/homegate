use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;

use crate::infrastructure::sql::{DbError, UnifiedExecutor};

const TABLE_NAME: &str = "google_verifications";

#[derive(Clone, Debug)]
pub struct GoogleVerificationRepository;

impl GoogleVerificationRepository {
    async fn count_verifications_since(
        executor: &mut UnifiedExecutor<'_>,
        google_identity_hash: &str,
        since: NaiveDateTime,
    ) -> Result<i64, DbError> {
        let statement = Query::select()
            .expr(Expr::col("id").count())
            .from(TABLE_NAME)
            .and_where(Expr::col("google_identity_hash").eq(google_identity_hash))
            .and_where(Expr::col("created_at").gte(since))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;
        let count: i64 = row.try_get(0).map_err(DbError::from)?;
        Ok(count)
    }

    pub async fn count_verifications_in_last_days(
        executor: &mut UnifiedExecutor<'_>,
        google_identity_hash: &str,
        days: i64,
    ) -> Result<i64, DbError> {
        let since = chrono::Utc::now().naive_utc() - chrono::Duration::days(days);
        Self::count_verifications_since(executor, google_identity_hash, since).await
    }

    pub async fn create_verification(
        executor: &mut UnifiedExecutor<'_>,
        google_identity_hash: &str,
        signup_code: &str,
    ) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table(TABLE_NAME)
            .columns(["google_identity_hash", "signup_code"])
            .values([google_identity_hash.into(), signup_code.into()])
            .expect("Failed to build insert query")
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }
}

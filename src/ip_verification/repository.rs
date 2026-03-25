use crate::infrastructure::sql::{DbError, UnifiedExecutor};
use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;

#[derive(Clone, Debug)]
pub struct IpVerificationRepository;

impl IpVerificationRepository {
    /// Count verifications for an IP hash since a given timestamp.
    pub async fn count_verifications_since(
        executor: &mut UnifiedExecutor<'_>,
        ip_address_hash: &str,
        since: NaiveDateTime,
    ) -> Result<i64, DbError> {
        let statement = Query::select()
            .expr(Expr::col("id").count())
            .from("ip_verifications")
            .and_where(Expr::col("ip_address_hash").eq(ip_address_hash))
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

    /// Count verifications for an IP hash in the last N days.
    pub async fn count_verifications_in_last_days(
        executor: &mut UnifiedExecutor<'_>,
        ip_address_hash: &str,
        days: i64,
    ) -> Result<i64, DbError> {
        let since = chrono::Utc::now().naive_utc() - chrono::Duration::days(days);
        Self::count_verifications_since(executor, ip_address_hash, since).await
    }

    /// Insert a new IP verification record.
    pub async fn create_verification(
        executor: &mut UnifiedExecutor<'_>,
        ip_address_hash: &str,
        signup_code: &str,
    ) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table("ip_verifications")
            .columns(["ip_address_hash", "signup_code"])
            .values([ip_address_hash.into(), signup_code.into()])
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

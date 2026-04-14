use crate::infrastructure::sql::{DbError, UnifiedExecutor};
use crate::invite::types::InviteStatus;
use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;

pub struct InviteRepository;

impl InviteRepository {
    /// Count claimed invitations within a time window for a given pubkey
    pub async fn count_claimed_in_last_days(
        executor: &mut UnifiedExecutor<'_>,
        pubkey: &str,
        days: i64,
    ) -> Result<i64, DbError> {
        let now = chrono::Utc::now().naive_utc();
        let since = now - chrono::Duration::days(days);
        Self::count_claimed_since(executor, pubkey, since).await
    }

    async fn count_claimed_since(
        executor: &mut UnifiedExecutor<'_>,
        pubkey: &str,
        since: NaiveDateTime,
    ) -> Result<i64, DbError> {
        let statement = Query::select()
            .expr(Expr::col("id").count())
            .from("invite_friend")
            .and_where(Expr::col("pubkey").eq(pubkey))
            .and_where(Expr::col("status").eq(InviteStatus::Claimed.as_str()))
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

    /// Find the most recent unclaimed signup code for a pubkey
    pub async fn find_unclaimed_signup_code(
        executor: &mut UnifiedExecutor<'_>,
        pubkey: &str,
    ) -> Result<Option<String>, DbError> {
        let statement = Query::select()
            .column("signup_code")
            .from("invite_friend")
            .and_where(Expr::col("pubkey").eq(pubkey))
            .and_where(Expr::col("status").eq(InviteStatus::Unclaimed.as_str()))
            .order_by("created_at", sea_query::Order::Desc)
            .limit(1)
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_optional(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;

        match row {
            Some(row) => {
                let code: String = row.try_get("signup_code").map_err(DbError::from)?;
                Ok(Some(code))
            }
            None => Ok(None),
        }
    }

    /// Insert an unclaimed invitation record
    pub async fn insert_unclaimed(
        executor: &mut UnifiedExecutor<'_>,
        pubkey: &str,
        proof_hash: &str,
        signup_code: &str,
    ) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table("invite_friend")
            .columns(["pubkey", "proof_hash", "signup_code", "status"])
            .values([
                pubkey.into(),
                proof_hash.into(),
                signup_code.into(),
                InviteStatus::Unclaimed.as_str().into(),
            ])
            .expect("Failed to build insert query")
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    /// Mark an unclaimed signup code as claimed
    pub async fn mark_claimed(
        executor: &mut UnifiedExecutor<'_>,
        signup_code: &str,
    ) -> Result<(), DbError> {
        let statement = Query::update()
            .table("invite_friend")
            .value("status", InviteStatus::Claimed.as_str())
            .and_where(Expr::col("signup_code").eq(signup_code))
            .and_where(Expr::col("status").eq(InviteStatus::Unclaimed.as_str()))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    /// Insert a failed invitation record
    pub async fn insert_failed(
        executor: &mut UnifiedExecutor<'_>,
        pubkey: &str,
        proof_hash: &str,
        failure_reason: &str,
    ) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table("invite_friend")
            .columns(["pubkey", "proof_hash", "status", "failure_reason"])
            .values([
                pubkey.into(),
                proof_hash.into(),
                InviteStatus::Failed.as_str().into(),
                failure_reason.into(),
            ])
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

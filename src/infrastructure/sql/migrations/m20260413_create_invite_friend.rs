use async_trait::async_trait;
use sea_query::{ColumnDef, Expr, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::sql::MigrationTrait;

pub struct M20260413CreateInviteFriend;

#[async_trait]
impl MigrationTrait for M20260413CreateInviteFriend {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("invite_friend")
            .if_not_exists()
            .col(
                ColumnDef::new("id")
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(ColumnDef::new("pubkey").text().not_null())
            .col(ColumnDef::new("proof_hash").text().not_null())
            .col(
                ColumnDef::new("created_at")
                    .timestamp()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(ColumnDef::new("signup_code").text())
            .col(
                ColumnDef::new("status")
                    .text()
                    .not_null()
                    .default("UNCLAIMED"),
            )
            .col(ColumnDef::new("failure_reason").text())
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create index on pubkey
        let index = Index::create()
            .name("idx_invite_friend_pubkey")
            .table("invite_friend")
            .col("pubkey")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create composite index on pubkey and status
        let index = Index::create()
            .name("idx_invite_friend_pubkey_status")
            .table("invite_friend")
            .col("pubkey")
            .col("status")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20260413_create_invite_friend"
    }
}

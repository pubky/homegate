use async_trait::async_trait;
use chrono::NaiveDateTime;
use sea_query::{ColumnDef, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::sql::MigrationTrait;

pub struct M20251216CreateLnVerification;

#[async_trait]
impl MigrationTrait for M20251216CreateLnVerification {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("lightning_verifications")
            .if_not_exists()
            .col(
                ColumnDef::new("payment_hash")
                    .char_len(64)
                    .not_null()
                    .primary_key(),
            ) // 64 hex characters
            .col(ColumnDef::new("amount_sat").integer().not_null())
            .col(
                ColumnDef::new("signup_code")
                    .text()
                    .null()
                    .default(sea_query::Expr::val(None::<String>)),
            )
            .col(ColumnDef::new("expires_at").timestamp().not_null())
            .col(
                ColumnDef::new("finalised_at")
                    .timestamp()
                    .null()
                    .default(sea_query::Expr::val(None::<NaiveDateTime>)),
            )
            .col(
                ColumnDef::new("created_at")
                    .timestamp()
                    .not_null()
                    .default(sea_query::Expr::current_timestamp()),
            )
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create index on payment hash
        let index = Index::create()
            .name("idx_lightning_invoices_payment_hash")
            .table("lightning_verifications")
            .col("payment_hash")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20251216_create_ln_verification"
    }
}

use async_trait::async_trait;
use sea_query::{ColumnDef, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::database::migrations::MigrationTrait;

pub struct M20251201CreateSmsVerifications;

#[async_trait]
impl MigrationTrait for M20251201CreateSmsVerifications {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("sms_verifications")
            .if_not_exists()
            .col(
                ColumnDef::new("id")
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(ColumnDef::new("phone_number_hash").text().not_null())
            .col(ColumnDef::new("prelude_id").text().not_null())
            .col(
                ColumnDef::new("created_at")
                    .timestamp()
                    .not_null()
                    .default(sea_query::Expr::current_timestamp()),
            )
            .col(ColumnDef::new("finalised_at").timestamp())
            .col(ColumnDef::new("signup_code").text())
            .col(
                ColumnDef::new("status")
                    .text()
                    .not_null()
                    .default("PENDING"),
            )
            .col(ColumnDef::new("failure_reason").text())
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create index on phone_number
        let index = Index::create()
            .name("idx_sms_verifications_phone")
            .table("sms_verifications")
            .col("phone_number_hash")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create index on prelude_id
        let index = Index::create()
            .name("idx_sms_verifications_prelude_id")
            .table("sms_verifications")
            .col("prelude_id")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create composite index on phone_number and status
        let index = Index::create()
            .name("idx_sms_verifications_phone_status")
            .table("sms_verifications")
            .col("phone_number_hash")
            .col("status")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20251201_create_sms_verifications"
    }
}

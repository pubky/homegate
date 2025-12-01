use async_trait::async_trait;
use sea_query::{ColumnDef, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::persistence::sql::migration::MigrationTrait;

pub struct M20251201CreateSmsVerifications;

#[async_trait]
impl MigrationTrait for M20251201CreateSmsVerifications {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("sms_verifications")
            .if_not_exists()
            .col(
                ColumnDef::new("unique_id")
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(ColumnDef::new("phone_number").text().not_null())
            .col(ColumnDef::new("prelude_id").text().not_null())
            .col(
                ColumnDef::new("created_at")
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(sea_query::Expr::current_timestamp()),
            )
            .col(ColumnDef::new("verified_at").timestamp_with_time_zone())
            .col(ColumnDef::new("signup_code").binary())
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        // Create index on phone_number
        let index = Index::create()
            .name("idx_sms_verifications_phone")
            .table("sms_verifications")
            .col("phone_number")
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

        Ok(())
    }

    fn name(&self) -> &str {
        "m20251201_create_sms_verifications"
    }
}

use async_trait::async_trait;
use sea_query::{ColumnDef, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::sql::MigrationTrait;

pub struct M20261105CreateAppleVerifications;

#[async_trait]
impl MigrationTrait for M20261105CreateAppleVerifications {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("apple_verifications")
            .if_not_exists()
            .col(
                ColumnDef::new("id")
                    .integer()
                    .primary_key()
                    .auto_increment()
                    .not_null(),
            )
            .col(ColumnDef::new("apple_identity_hash").text().not_null())
            .col(ColumnDef::new("signup_code").text().not_null())
            .col(
                ColumnDef::new("created_at")
                    .timestamp()
                    .not_null()
                    .default(sea_query::Expr::current_timestamp()),
            )
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        let index = Index::create()
            .name("idx_apple_verifications_identity_hash_created_at")
            .table("apple_verifications")
            .col("apple_identity_hash")
            .col("created_at")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20261105_create_apple_verifications"
    }
}

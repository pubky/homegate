use async_trait::async_trait;
use sea_query::{ColumnDef, Index, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::sql::MigrationTrait;

pub struct M20260324CreateIpVerifications;

#[async_trait]
impl MigrationTrait for M20260324CreateIpVerifications {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::create()
            .table("ip_verifications")
            .if_not_exists()
            .col(
                ColumnDef::new("id")
                    .integer()
                    .primary_key()
                    .auto_increment()
                    .not_null(),
            )
            .col(ColumnDef::new("ip_address_hash").text().not_null())
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
            .name("idx_ip_verifications_ip_hash_created_at")
            .table("ip_verifications")
            .col("ip_address_hash")
            .col("created_at")
            .to_owned();

        let query = index.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20260324_create_ip_verifications"
    }
}

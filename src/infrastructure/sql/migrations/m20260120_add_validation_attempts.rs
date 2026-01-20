use async_trait::async_trait;
use sea_query::{ColumnDef, PostgresQueryBuilder, Table};
use sqlx::Transaction;

use crate::infrastructure::sql::MigrationTrait;

pub struct M20260120AddValidationAttempts;

#[async_trait]
impl MigrationTrait for M20260120AddValidationAttempts {
    async fn up(&self, tx: &mut Transaction<'static, sqlx::Postgres>) -> anyhow::Result<()> {
        let statement = Table::alter()
            .table("sms_verifications")
            .add_column(ColumnDef::new("attempts").integer().not_null().default(0))
            .to_owned();

        let query = statement.build(PostgresQueryBuilder);
        sqlx::query(&query).execute(&mut **tx).await?;

        Ok(())
    }

    fn name(&self) -> &str {
        "m20260120_add_validation_attempts"
    }
}

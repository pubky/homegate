mod m20251201_create_sms_verifications;

use crate::infrastructure::sql::{MigrationTrait, Migrator, SqlDb};

use m20251201_create_sms_verifications::M20251201CreateSmsVerifications;

/// Returns the list of migrations for the SMS verification module.
fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(M20251201CreateSmsVerifications)]
}

/// Run all migrations for the SMS verification module.
pub async fn run_migrations(db: &SqlDb) -> anyhow::Result<()> {
    Migrator::run(db, migrations()).await
}

/// Create a test database with this module's migrations applied.
#[cfg(test)]
pub async fn test_db(pool: sqlx::PgPool) -> SqlDb {
    SqlDb::test_with_migrations(pool, migrations()).await
}

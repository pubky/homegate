mod m20251216_create_ln_verification;

use crate::infrastructure::sql::{MigrationTrait, Migrator, SqlDb};

use m20251216_create_ln_verification::M20251216CreateLnVerification;

/// Returns the list of migrations for the Lightning verification module.
fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(M20251216CreateLnVerification)]
}

/// Run all migrations for the Lightning verification module.
pub async fn run_migrations(db: &SqlDb) -> anyhow::Result<()> {
    Migrator::run(db, migrations()).await
}

/// Create a test database with this module's migrations applied.
#[cfg(test)]
pub async fn test_db(pool: sqlx::PgPool) -> SqlDb {
    SqlDb::test_with_migrations(pool, migrations()).await
}

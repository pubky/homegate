use crate::persistence::{config::EnvConfig, db::Db, sql::SqlDb};

/// The application context that is used to access the application resources.
/// It is clonable but should preferably be passed as a reference as
/// it is not guaranteed that each object is cheap to clone.
#[derive(Debug, Clone)]
pub struct AppContext {
    pub db: Db,
    pub config: EnvConfig,
}

impl AppContext {
    /// Load the application context from the environment.
    pub async fn load() -> anyhow::Result<Self> {
        let config = EnvConfig::load()
            .map_err(|e| anyhow::Error::new(e).context("Failed to load config."))?;
        let sql_db = SqlDb::connect(&config.database_url)
            .await
            .map_err(|e| anyhow::Error::new(e).context("Failed to connect to database."))?;
        Ok(Self { db: Db::new(sql_db), config })
    }
}

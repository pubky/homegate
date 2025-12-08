use crate::{
    SmsVerificationError,
    infrastructure::{config::EnvConfig, database::SqlDb},
    shared::HomeserverAdminAPI,
    sms_verification::{
        prelude_api::PreludeAPI, repository::SmsVerificationRepositoryError,
        service::SmsVerificationService,
    },
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub sms_verification: SmsVerificationService,
}

impl AppState {
    /// Creates a production AppState from config
    pub async fn from_config(config: &EnvConfig) -> Result<Self, SmsVerificationError> {
        // Connect to database (migrations run automatically)
        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(SmsVerificationRepositoryError::Database)?;

        Self::create_from_db(config, db)
    }

    #[cfg(test)]
    pub fn from_config_with_db(
        config: &EnvConfig,
        db: SqlDb,
    ) -> Result<Self, SmsVerificationError> {
        Self::create_from_db(config, db)
    }

    fn create_from_db(config: &EnvConfig, db: SqlDb) -> Result<Self, SmsVerificationError> {
        use crate::sms_verification::repository::SmsVerificationRepository;

        let prelude_api = PreludeAPI::from_config(config);
        let homeserver_admin_api = HomeserverAdminAPI::from_config(config);

        // Initialize SMS verification with repository pattern
        let sms_repo = SmsVerificationRepository::new(db.clone());
        let sms_verification = SmsVerificationService::new(
            sms_repo,
            prelude_api,
            homeserver_admin_api,
            config.max_verified_sessions,
        );

        Ok(Self { sms_verification })
    }
}

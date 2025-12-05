use crate::{
    infrastructure::{
        config::EnvConfig,
        database::{DbError, SqlDb},
    },
    shared::{HomeserverAdminApi, HomeserverAdminApiTrait},
    sms_verification::{
        prelude_api::{PreludeAPI, SmsVerificationProviderApi},
        repository::SmsVerificationRepository,
        service::SmsVerificationService,
    },
};

#[derive(Clone, Debug)]
pub struct AppState<T, S>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService<T, S>,
}

impl AppState<PreludeAPI, HomeserverAdminApi> {
    /// Creates a production AppState from config
    pub async fn from_config(config: &EnvConfig) -> Result<Self, DbError> {
        // Connect to database (migrations run automatically)
        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(DbError::Database)?;

        let prelude_api = PreludeAPI::from_config(config);
        let homeserver_admin_api = HomeserverAdminApi::from_config(config);

        // Initialize SMS verification with repository pattern
        let sms_repo = SmsVerificationRepository::new(db.clone());
        let sms_verification = SmsVerificationService::new(
            sms_repo,
            prelude_api,
            homeserver_admin_api,
            config.max_verified_sessions,
        );

        Ok(Self {
            db,
            sms_verification,
        })
    }
}

impl
    AppState<
        crate::sms_verification::prelude_api::MockSmsVerificationProviderApi,
        crate::shared::homeserver::MockHomeserverAdminApi,
    >
{
    /// Creates a mock AppState for testing with mock SMS verification provider and mock homeserver admin API
    pub async fn create_mock_state(
        config: &EnvConfig,
        pool: Option<sqlx::PgPool>,
    ) -> Result<Self, DbError> {
        let db = match pool {
            Some(p) => {
                // For tests, use SqlDb::test() which handles migrations
                SqlDb::test(p).await
            }
            None => {
                // Connect normally (migrations run automatically)
                SqlDb::connect(&config.database_url)
                    .await
                    .map_err(DbError::Database)?
            }
        };

        let mock_prelude_api =
            crate::sms_verification::prelude_api::MockSmsVerificationProviderApi::new();
        let mock_homeserver_admin_api = crate::shared::homeserver::MockHomeserverAdminApi::new();

        // Initialize SMS verification with repository pattern
        let sms_repo = SmsVerificationRepository::new(db.clone());
        let sms_verification = SmsVerificationService::new(
            sms_repo,
            mock_prelude_api,
            mock_homeserver_admin_api,
            config.max_verified_sessions,
        );

        Ok(Self {
            db,
            sms_verification,
        })
    }
}

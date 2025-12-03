use crate::{
    Db, EnvConfig, HomeserverAdminApi, PreludeAPI, SmsVerificationService,
    external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi},
    persistence::db::DbError,
};

#[derive(Clone, Debug)]
pub struct AppState<T, S>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    pub db: Db,
    pub sms_verification_service: SmsVerificationService<T, S>,
}

impl AppState<PreludeAPI, HomeserverAdminApi> {
    /// Creates a production AppState from config
    pub async fn from_config(config: &EnvConfig) -> Result<Self, DbError> {
        let db = Db::connect(&config.database_url).await?;
        let prelude_api = PreludeAPI::from_config(config);
        let homeserver_admin_api = HomeserverAdminApi::from_config(config);
        let sms_verification_service = SmsVerificationService::new(
            db.clone(),
            prelude_api,
            homeserver_admin_api,
            config.max_verified_sessions,
        );

        Ok(Self {
            db,
            sms_verification_service,
        })
    }
}

impl
    AppState<
        crate::MockSmsVerificationProviderApi,
        crate::external_apis::homeserver::MockHomeserverAdminApi,
    >
{
    /// Creates a mock AppState for testing with mock SMS verification provider and mock homeserver admin API
    pub async fn create_mock_state(config: &EnvConfig) -> Result<Self, DbError> {
        let db = Db::connect(&config.database_url).await?;
        let mock_prelude_api = crate::MockSmsVerificationProviderApi::new();
        let mock_homeserver_admin_api =
            crate::external_apis::homeserver::MockHomeserverAdminApi::new();
        let sms_verification_service = SmsVerificationService::new(
            db.clone(),
            mock_prelude_api,
            mock_homeserver_admin_api,
            config.max_verified_sessions,
        );

        Ok(Self {
            db,
            sms_verification_service,
        })
    }
}

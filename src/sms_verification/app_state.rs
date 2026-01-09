use crate::{
    infrastructure::{config::EnvConfig, sql::SqlDb},
    shared::HomeserverAdminAPI,
    sms_verification::{prelude_api::PreludeAPI, service::SmsVerificationService},
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService,
}

impl AppState {
    pub fn new(config: &EnvConfig, db: SqlDb) -> Self {
        let prelude_api = PreludeAPI::new(&config.prelude_api_url, &config.prelude_api_key);
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );
        let sms_verification = SmsVerificationService::new(
            prelude_api,
            homeserver_admin_api.clone(),
            config.max_sms_verifications_per_week,
            config.max_sms_verifications_per_year,
        );
        Self {
            db,
            sms_verification,
        }
    }
}

use crate::{
    infrastructure::{config::EnvConfig, database::SqlDb},
    shared::HomeserverAdminAPI,
    sms_verification::{
        HasherArgon2id, SmsVerificationRepository, prelude_api::PreludeAPI,
        service::SmsVerificationService,
    },
};

#[derive(Clone, Debug)]
pub struct AppState {
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
        let phone_hasher = HasherArgon2id::new();
        let sms_repo = SmsVerificationRepository::new(db.clone(), phone_hasher);
        let sms_verification = SmsVerificationService::new(
            sms_repo,
            prelude_api,
            homeserver_admin_api,
            config.max_sms_verifications_per_week,
            config.max_sms_verifications_per_year,
        );
        Self { sms_verification }
    }
}

use crate::{
    SmsVerificationError,
    infrastructure::{config::EnvConfig, database::SqlDb},
    shared::HomeserverAdminAPI,
    sms_verification::{prelude_api::PreludeAPI, service::SmsVerificationService},
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub sms_verification: SmsVerificationService,
}

impl AppState {
    pub fn new(config: &EnvConfig, db: SqlDb) -> Result<Self, SmsVerificationError> {
        use crate::sms_verification::repository::SmsVerificationRepository;

        let prelude_api = PreludeAPI::new(&config.prelude_api_url, &config.prelude_api_key);
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );

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

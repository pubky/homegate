use crate::{
    infrastructure::{config::SmsVerificationConfig, sql::SqlDb},
    shared::HasherArgon2id,
    shared::HomeserverAdminAPI,
    sms_verification::{prelude_api::PreludeAPI, service::SmsVerificationService},
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        sms: &SmsVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
    ) -> Self {
        let prelude_api = PreludeAPI::new(&sms.prelude_api_url, &sms.prelude_api_key);
        let sms_verification = SmsVerificationService::new(
            prelude_api,
            homeserver_api.clone(),
            sms.max_verifications_per_week,
            sms.max_verifications_per_year,
            sms.max_failed_validation_attempts,
            sms.limit_whitelist.clone(),
            hasher,
        );
        Self {
            db,
            sms_verification,
        }
    }
}

use crate::{
    infrastructure::{config::SmsVerificationConfig, sql::SqlDb},
    shared::HasherArgon2id,
    shared::HomeserverAdminAPI,
    shared::Metrics,
    sms_verification::{prelude_api::PreludeAPI, service::SmsVerificationService},
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService,
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        sms: &SmsVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
        metrics: Metrics,
    ) -> Self {
        let prelude_api = PreludeAPI::new(&sms.prelude_api_url, &sms.prelude_api_key);
        let sms_verification = SmsVerificationService::new(
            prelude_api,
            homeserver_api.clone(),
            sms,
            hasher,
            metrics.clone(),
        );
        Self {
            db,
            sms_verification,
            metrics,
        }
    }
}

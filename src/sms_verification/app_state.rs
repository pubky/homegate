use crate::{
    infrastructure::{
        config::{HomeserverConfig, SmsVerificationConfig},
        sql::SqlDb,
    },
    shared::HomeserverAdminAPI,
    sms_verification::{prelude_api::PreludeAPI, service::SmsVerificationService},
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService,
}

impl AppState {
    pub fn new(homeserver: &HomeserverConfig, sms: &SmsVerificationConfig, db: SqlDb) -> Self {
        let prelude_api = PreludeAPI::new(&sms.prelude_api_url, &sms.prelude_api_key);
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &homeserver.admin_api_url,
            &homeserver.admin_password,
            &homeserver.pubky,
        );
        let sms_verification = SmsVerificationService::new(
            prelude_api,
            homeserver_admin_api.clone(),
            sms.max_verifications_per_week,
            sms.max_verifications_per_year,
            sms.max_failed_validation_attempts,
            sms.limit_whitelist.clone(),
        );
        Self {
            db,
            sms_verification,
        }
    }
}

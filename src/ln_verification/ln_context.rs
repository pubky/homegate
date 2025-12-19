use crate::{
    infrastructure::{EnvConfig, sql::SqlDb},
    ln_verification::{phoenixd_api::PhoenixdAPI, service::LnVerificationService},
    shared::HomeserverAdminAPI,
};

/// Context for the lightning verification
/// Used for easier dependency injection
#[derive(Clone, Debug)]
pub struct LnContext {
    pub service: LnVerificationService,
    pub phoenixd_api: PhoenixdAPI,
    pub homeserver_api: HomeserverAdminAPI,
}

impl LnContext {
    pub fn new(db: SqlDb, config: &EnvConfig) -> Self {
        let phoenixd_api =
            PhoenixdAPI::new(&config.phoenixd_api_url, &config.phoenixd_api_password);
        let homeserver_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );
        let service = LnVerificationService::new(
            db.clone(),
            phoenixd_api.clone(),
            homeserver_api.clone(),
            config.lightning_invoice_price_sat,
            config.lightning_invoice_description.clone(),
            config.lightning_invoice_expiry_seconds,
        );
        Self {
            service,
            phoenixd_api,
            homeserver_api,
        }
    }
}

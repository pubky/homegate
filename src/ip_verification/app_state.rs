use crate::{
    infrastructure::{config::EnvConfig, sql::SqlDb},
    shared::HomeserverAdminAPI,
};

use super::service::IpVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub ip_verification: IpVerificationService,
}

impl AppState {
    pub fn new(config: &EnvConfig, db: SqlDb) -> Self {
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );
        let ip_verification = IpVerificationService::new(
            homeserver_admin_api,
            config.max_ip_verifications_per_week,
            config.max_ip_verifications_per_year,
            config.ip_verification_enabled,
        );
        Self {
            db,
            ip_verification,
        }
    }
}

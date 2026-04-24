use crate::{
    infrastructure::{
        config::{HomeserverConfig, IpVerificationConfig},
        sql::SqlDb,
    },
    shared::HomeserverAdminAPI,
};

use super::service::IpVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub ip_verification: IpVerificationService,
}

impl AppState {
    pub fn new(homeserver: &HomeserverConfig, ip: &IpVerificationConfig, db: SqlDb) -> Self {
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &homeserver.admin_api_url,
            &homeserver.admin_password,
            &homeserver.pubky,
        );
        let ip_verification = IpVerificationService::new(
            homeserver_admin_api,
            ip.max_verifications_per_week,
            ip.max_verifications_per_year,
        );
        Self {
            db,
            ip_verification,
        }
    }
}

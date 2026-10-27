use crate::{
    infrastructure::{config::IpVerificationConfig, sql::SqlDb},
    shared::HasherArgon2id,
    shared::HomeserverAdminAPI,
    shared::Metrics,
};

use super::service::IpVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub ip_verification: IpVerificationService,
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        ip: &IpVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
        metrics: Metrics,
    ) -> Self {
        let ip_verification =
            IpVerificationService::new(db, homeserver_api.clone(), ip, hasher, metrics.clone());
        Self {
            ip_verification,
            metrics,
        }
    }
}

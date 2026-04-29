use std::path::PathBuf;

use crate::{
    infrastructure::{config::IpVerificationConfig, sql::SqlDb},
    shared::HomeserverAdminAPI,
};

use super::service::IpVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub ip_verification: IpVerificationService,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        ip: &IpVerificationConfig,
        db: SqlDb,
        pepper_path: PathBuf,
    ) -> Self {
        let ip_verification =
            IpVerificationService::new(db, homeserver_api.clone(), ip, pepper_path);
        Self { ip_verification }
    }
}

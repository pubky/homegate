use crate::infrastructure::{config::GoogleVerificationConfig, sql::SqlDb};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI, Metrics};

use super::service::GoogleVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub google_verification: GoogleVerificationService,
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        google: &GoogleVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
        metrics: Metrics,
    ) -> Self {
        let google_verification = GoogleVerificationService::new(
            db,
            homeserver_api.clone(),
            google,
            hasher,
            metrics.clone(),
        );
        Self {
            google_verification,
            metrics,
        }
    }
}

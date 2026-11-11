use crate::infrastructure::{config::AppleVerificationConfig, sql::SqlDb};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI, Metrics};

use super::service::AppleVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub apple_verification: AppleVerificationService,
    pub metrics: Metrics,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        apple: &AppleVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
        metrics: Metrics,
    ) -> Self {
        let apple_verification = AppleVerificationService::new(
            db,
            homeserver_api.clone(),
            apple,
            hasher,
            metrics.clone(),
        );
        Self {
            apple_verification,
            metrics,
        }
    }
}

use crate::infrastructure::{config::GoogleVerificationConfig, sql::SqlDb};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};

use super::service::GoogleVerificationService;

#[derive(Clone, Debug)]
pub struct AppState {
    pub google_verification: GoogleVerificationService,
}

impl AppState {
    pub fn new(
        homeserver_api: &HomeserverAdminAPI,
        google: &GoogleVerificationConfig,
        db: SqlDb,
        hasher: HasherArgon2id,
    ) -> Self {
        let google_verification =
            GoogleVerificationService::new(db, homeserver_api.clone(), google, hasher);
        Self {
            google_verification,
        }
    }
}

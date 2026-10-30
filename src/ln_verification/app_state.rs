use std::sync::Arc;

use crate::ln_verification::service::LnVerificationService;
use crate::shared::HomeserverAdminAPI;

/// Application state for the Lightning Network verification HTTP handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    pub ln_service: Arc<LnVerificationService>,
    pub homeserver_api: HomeserverAdminAPI,
}

impl AppState {
    /// Create a new AppState instance.
    ///
    /// Note: The caller is responsible for starting the background syncer
    /// by calling `ln_service.run_background_sync()` in a spawned task.
    pub fn new(ln_service: Arc<LnVerificationService>, homeserver_api: HomeserverAdminAPI) -> Self {
        Self {
            ln_service,
            homeserver_api,
        }
    }
}

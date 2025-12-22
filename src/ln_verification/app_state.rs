use crate::ln_verification::{
    invoice_background_syncer::InvoiceBackgroundSyncer, service::LnVerificationService,
};
use crate::shared::HomeserverAdminAPI;

/// Application state for the Lightning Network verification HTTP handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    pub syncer: InvoiceBackgroundSyncer,
    pub ln_service: LnVerificationService,
    pub homeserver_api: HomeserverAdminAPI,
}

impl AppState {
    /// Create a new AppState instance.
    ///
    /// Note: The caller is responsible for starting the background syncer
    /// by calling `syncer.run()` in a spawned task.
    pub fn new(
        syncer: InvoiceBackgroundSyncer,
        ln_service: LnVerificationService,
        homeserver_api: HomeserverAdminAPI,
    ) -> Self {
        Self {
            syncer,
            ln_service,
            homeserver_api,
        }
    }
}

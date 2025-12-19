use std::process;

use crate::{
    EnvConfig,
    ln_verification::{
        invoice_background_syncer::InvoiceBackgroundSyncer, phoenixd_api::PhoenixdAPI, service::LnVerificationService
    },
    shared::HomeserverAdminAPI,
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub syncer: InvoiceBackgroundSyncer,
    pub ln_service: LnVerificationService,
    pub homeserver_api: HomeserverAdminAPI,
}

impl AppState {
    /// Create a new AppState instance
    /// This will start the invoice background syncer in a new background task.
    pub async fn new(config: &EnvConfig, db: crate::infrastructure::sql::SqlDb) -> Self {
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
        let syncer = InvoiceBackgroundSyncer::new(service.clone(), phoenixd_api).await;
        let syncer_clone = syncer.clone();
        tokio::task::spawn(async move {
            if let Err(e) = syncer_clone.run().await {
                tracing::error!(error = %e, "Error running invoice background syncer");
                process::exit(1); // Force a restart of the server
            }
        });
        Self {
            syncer,
            ln_service: service,
            homeserver_api: homeserver_api,
        }
    }
}


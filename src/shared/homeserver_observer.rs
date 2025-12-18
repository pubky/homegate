use std::time::Duration;

use crate::{EnvConfig, shared::HomeserverAdminAPI};

/// Observer for the homeserver admin API
/// This is used to check if the homeserver is running and responsive
/// If it is not responsive, we'll crash the server
pub struct HomeserverObserver {
    admin_api: HomeserverAdminAPI,
}

impl HomeserverObserver {
    pub fn new(admin_api: HomeserverAdminAPI) -> Self {
        Self { admin_api }
    }

    /// Continuously checks if the homeserver is responsive and errors if it is not
    /// This is used to ensure that the homeserver is always responsive
    /// This method is blocking.
    pub async fn err_on_unresponsive(&self) -> Result<(), reqwest::Error> {
        let mut i = 0;
        loop {
            i += 1;
            let should_do_password_check = i % 60 == 0; // Occasional expensive password check to verify the admin password is correct.
            if should_do_password_check {
                self.admin_api.verify_password().await?;
            } else {
                self.admin_api.health_check().await?;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    /// Spawns a task to continuously check if the homeserver is responsive.
    /// This will crash the server if the homeserver is not responsive.
    /// This is to ensure that homegate does not hand out lightning invoices to users if we can't generate signup tokens.
    pub async fn spawn_crash_on_unresponsive_homeserver(config: &EnvConfig) {
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );

        // Check homeserver password to prevent the server from starting if the homeserver is not responsive/password is incorrect.
        if let Err(e) = homeserver_admin_api.verify_password().await {
            tracing::error!(
                "Homeserver connection failed: {:?}. Stopping server because the homeserver is a critical dependency.",
                e
            );
            std::process::exit(1);
        }

        // Continuously check if the homeserver is responsive in the background.
        let homeserver_observer = HomeserverObserver::new(homeserver_admin_api);
        tokio::spawn(async move {
            if let Err(e) = homeserver_observer.err_on_unresponsive().await {
                tracing::error!("Homeserver is not responsive: {:?}", e);
                tracing::error!("Shutting down server due to homeserver being unresponsive");
                std::process::exit(1);
            }
        });
    }
}

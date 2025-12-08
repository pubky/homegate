use crate::EnvConfig;
use thiserror::Error;
use url::Url;

#[derive(Error, Debug)]
pub enum HomeserverAdminApiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error (status {status}): {message}")]
    ApiError { status: u16, message: String },
}

#[derive(Clone, Debug)]
pub struct HomeserverAdminAPI {
    http_client: reqwest::Client,
    admin_password: String,
    base_url: Url,
    homeserver_pubky: String,
}

impl HomeserverAdminAPI {
    pub fn from_config(config: &EnvConfig) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: config.homeserver_admin_password.clone(),
            base_url: config.homeserver_admin_api_url.clone(),
            homeserver_pubky: config.homeserver_pubky.clone(),
        }
    }

    /// Generates a signup token by calling the homeserver admin API
    pub async fn generate_signup_token(&self) -> Result<String, HomeserverAdminApiError> {
        let url = self
            .base_url
            .join("generate_signup_token")
            .expect("Failed to join URL path");
        let response = self
            .http_client
            .get(url)
            .header("X-Admin-Password", &self.admin_password)
            .send()
            .await?;
        Ok(response.error_for_status()?.text().await?)
    }

    pub fn get_homeserver_pubky(&self) -> String {
        self.homeserver_pubky.clone()
    }
}

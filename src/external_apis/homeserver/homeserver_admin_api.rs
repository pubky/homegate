use crate::app_context::AppContext;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HomeserverAdminApiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error (status {status}): {message}")]
    ApiError { status: u16, message: String },
}

pub struct HomeserverAdminApi {
    http_client: reqwest::Client,
    admin_password: String,
    base_url: String,
}

impl HomeserverAdminApi {
    pub fn new(context: &AppContext) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: context.config.homeserver_admin_password.clone(),
            base_url: context.config.homeserver_api_url.clone(),
        }
    }
}

#[async_trait]
pub trait HomeserverAdminApiTrait: Send + Sync {
    /// Generates a signup token
    async fn generate_signup_token(&self) -> Result<String, HomeserverAdminApiError>;
}

#[async_trait]
impl HomeserverAdminApiTrait for HomeserverAdminApi {
    /// Generates a signup token by calling the homeserver admin API
    async fn generate_signup_token(&self) -> Result<String, HomeserverAdminApiError> {
        let url = format!("{}/generate_signup_token", self.base_url);

        let response = self
            .http_client
            .get(&url)
            .header("X-Admin-Password", &self.admin_password)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            return Err(HomeserverAdminApiError::ApiError {
                status: status.as_u16(),
                message: error_body,
            });
        }

        let token = response.text().await?;
        Ok(token)
    }
}

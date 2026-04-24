use crate::infrastructure::config::SignupQuotaConfig;
use url::Url;

#[derive(Clone, Debug)]
pub struct HomeserverAdminAPI {
    http_client: reqwest::Client,
    admin_password: String,
    base_url: Url,
    homeserver_pubky: String,
}

impl HomeserverAdminAPI {
    pub fn new(base_url: &Url, admin_password: &str, homeserver_pubky: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: admin_password.to_owned(),
            base_url: base_url.clone(),
            homeserver_pubky: homeserver_pubky.to_owned(),
        }
    }

    /// Generates a signup token with homeserver system defaults (GET).
    pub async fn generate_signup_token(&self) -> Result<String, reqwest::Error> {
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
        response.error_for_status()?.text().await
    }

    /// Generates a signup token with explicit quota limits (POST).
    pub async fn generate_signup_token_with_quota(
        &self,
        quota: &SignupQuotaConfig,
    ) -> Result<String, reqwest::Error> {
        let url = self
            .base_url
            .join("generate_signup_token")
            .expect("Failed to join URL path");
        let response = self
            .http_client
            .post(url)
            .header("X-Admin-Password", &self.admin_password)
            .json(quota)
            .send()
            .await?;
        response.error_for_status()?.text().await
    }

    pub fn get_homeserver_pubky(&self) -> String {
        self.homeserver_pubky.clone()
    }

    /// Verifies the admin password by making a GET request to the /info endpoint.
    /// This request might take a moment.
    pub async fn verify_password(&self) -> Result<(), reqwest::Error> {
        let url = self
            .base_url
            .join("/info")
            .expect("Failed to join URL path");
        let response = self
            .http_client
            .get(url)
            .header("X-Admin-Password", &self.admin_password)
            .send()
            .await?;
        response.error_for_status()?;
        Ok(())
    }
}

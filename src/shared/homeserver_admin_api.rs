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
    #[cfg(test)]
    pub fn new(base_url: &Url, admin_password: &str, homeserver_pubky: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: admin_password.to_owned(),
            base_url: base_url.clone(),
            homeserver_pubky: homeserver_pubky.to_owned(),
        }
    }

    pub fn from_config(base_url: &Url, admin_password: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: admin_password.to_owned(),
            base_url: base_url.clone(),
            homeserver_pubky: String::new(),
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

    /// Verifies the admin password by calling GET /info and extracts the
    /// homeserver's public key from the response.
    pub async fn fetch_info(&mut self) -> anyhow::Result<()> {
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
        let body: serde_json::Value = response.error_for_status()?.json().await?;
        let pubky = body["public_key"].as_str().ok_or_else(|| {
            anyhow::anyhow!("Homeserver /info response missing 'public_key' field")
        })?;
        self.homeserver_pubky = pubky.to_owned();
        tracing::info!(pubky = %self.homeserver_pubky, "Fetched homeserver public key");
        Ok(())
    }
}

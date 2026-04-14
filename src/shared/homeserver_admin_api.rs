use url::Url;

#[derive(Clone, Debug)]
pub struct HomeserverAdminAPI {
    http_client: reqwest::Client,
    admin_password: String,
    base_url: Url,
    public_base_url: Url,
    homeserver_pubky: String,
}

impl HomeserverAdminAPI {
    pub fn new(
        base_url: &Url,
        public_base_url: &Url,
        admin_password: &str,
        homeserver_pubky: &str,
    ) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: admin_password.to_owned(),
            base_url: base_url.clone(),
            public_base_url: public_base_url.clone(),
            homeserver_pubky: homeserver_pubky.to_owned(),
        }
    }

    /// Generates a signup token by calling the homeserver admin API
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

    pub fn get_homeserver_pubky(&self) -> String {
        self.homeserver_pubky.clone()
    }

    /// Fetches a public file from a pubky user's storage on the homeserver.
    /// Returns `Ok(Some(content))` if found, `Ok(None)` if 404, or `Err` on failure.
    pub async fn get_pubky_file(
        &self,
        pubkey_z32: &str,
        path: &str,
    ) -> Result<Option<String>, reqwest::Error> {
        let url = self
            .public_base_url
            .join(path)
            .expect("Failed to join URL path");
        let host = format!("_pubky.{}", pubkey_z32);
        let response = self
            .http_client
            .get(url)
            .header("Host", host)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let text = response.error_for_status()?.text().await?;
        Ok(Some(text))
    }

    /// Checks whether a signup token has been claimed by querying the homeserver's
    /// public `/signup_tokens/{code}` endpoint.
    /// Returns `Ok(true)` if claimed, `Ok(false)` if not, or `Err` on failure.
    pub async fn is_signup_token_claimed(&self, signup_code: &str) -> Result<bool, reqwest::Error> {
        let url = self
            .public_base_url
            .join(&format!("signup_tokens/{}", signup_code))
            .expect("Failed to join URL path");
        tracing::debug!("Checking signup token claimed status at: {}", url);
        let response = self.http_client.get(url).send().await?;
        let status = response.status();
        let body = response.error_for_status()?.text().await?;
        tracing::debug!(
            "Signup token claimed response: status={}, body={:?}",
            status,
            body
        );
        // The homeserver returns JSON: {"status":"used"|"unused", ...}
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(parsed["status"] == "used")
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

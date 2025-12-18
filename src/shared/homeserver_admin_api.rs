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
}

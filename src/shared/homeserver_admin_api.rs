#[cfg(test)]
use std::sync::Arc;

use url::Url;
#[cfg(test)]
use wiremock::MockServer;

#[derive(Clone, Debug)]
pub struct HomeserverAdminAPI {
    http_client: reqwest::Client,
    admin_password: String,
    base_url: Url,
    homeserver_pubky: String,

    #[cfg(test)]
    pub mock_server: Option<Arc<MockServer>>,
}

impl HomeserverAdminAPI {
    pub fn new(base_url: &Url, admin_password: &str, homeserver_pubky: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            admin_password: admin_password.to_owned(),
            base_url: base_url.clone(),
            homeserver_pubky: homeserver_pubky.to_owned(),
            #[cfg(test)]
            mock_server: None,
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

    #[cfg(test)]
    pub async fn test() -> Self {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{header, method, path},
        };

        let signup_token = "token123456";
        let homeserver_pubky = "pubky123456";
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/generate_signup_token"))
            .and(header("X-Admin-Password", "test-pass"))
            .respond_with(ResponseTemplate::new(200).set_body_string(signup_token))
            .expect(1)
            .mount(&mock_server)
            .await;
        let mut api = Self::new(
            &mock_server.uri().parse().unwrap(),
            "test-pass",
            &homeserver_pubky,
        );
        api.mock_server = Some(Arc::new(mock_server));
        api
    }
}

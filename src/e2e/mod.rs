mod http;

use crate::{AppState, EnvConfig, HttpServer};
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;

/// Test server wrapper for E2E HTTP tests
pub struct TestServer {
    #[allow(dead_code)]
    pub server: HttpServer,
    pub base_url: String,
    pub client: Client,
}

impl TestServer {
    /// Create a new test server with an isolated database from #[sqlx::test]
    pub async fn new(pool: PgPool) -> Self {
        // Create test config
        let config = EnvConfig {
            database_url: Default::default(),
            http_listen_socket: "127.0.0.1:0".parse().unwrap(),
            prelude_api_key: "test-key".to_string(),
            prelude_api_url: "http://localhost".parse().unwrap(),
            homeserver_api_url: "http://localhost".parse().unwrap(),
            homeserver_admin_password: "test-pass".to_string(),
            homeserver_pubky: "test-homeserver-pubky".to_string(),
            max_verified_sessions: 10,
        };

        // Create mock state from the test pool
        let state = AppState::create_mock_state(&config, Some(pool))
            .await
            .expect("Failed to create mock state");

        // Start server on dynamic port (127.0.0.1:0)
        let listen_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = HttpServer::start(listen_addr, state)
            .await
            .expect("Failed to start HTTP server");

        let base_url = server.url_string();
        let client = Client::new();

        Self {
            server,
            base_url,
            client,
        }
    }

    /// Send verification code to a phone number
    pub async fn send_code(&self, phone: &str) -> reqwest::Response {
        self.client
            .post(&format!("{}/v1/sms_verification/send_code", self.base_url))
            .json(&serde_json::json!({ "phone_number": phone }))
            .send()
            .await
            .expect("Failed to send request")
    }

    /// Verify code for a phone number
    pub async fn verify_code(&self, phone: &str, code: &str) -> reqwest::Response {
        self.client
            .post(&format!(
                "{}/v1/sms_verification/verify_code",
                self.base_url
            ))
            .json(&serde_json::json!({ "phone_number": phone, "code": code }))
            .send()
            .await
            .expect("Failed to send request")
    }

    /// Send verification code with custom headers (for IP extraction tests)
    pub async fn send_code_with_headers(
        &self,
        phone: &str,
        headers: Vec<(&str, &str)>,
    ) -> reqwest::Response {
        let mut req = self
            .client
            .post(&format!("{}/v1/sms_verification/send_code", self.base_url))
            .json(&serde_json::json!({ "phone_number": phone }));

        for (name, value) in headers {
            req = req.header(name, value);
        }

        req.send().await.expect("Failed to send request")
    }
}

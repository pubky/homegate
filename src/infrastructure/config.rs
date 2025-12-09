use std::net::SocketAddr;

use crate::infrastructure::database::ConnectionString;
use url::Url;

/// The environment configuration.
/// This is the configuration that is loaded from the environment variables.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EnvConfig {
    #[serde(default)]
    pub database_url: ConnectionString,
    pub http_listen_socket: SocketAddr,
    pub prelude_api_key: String,
    #[serde(default = "default_prelude_api_url")]
    pub prelude_api_url: Url,
    pub homeserver_admin_api_url: Url,
    pub homeserver_admin_password: String,
    pub homeserver_pubky: String,
    #[serde(default = "default_max_verified_sessions")]
    pub max_verified_sessions: u32,
    pub phone_number_pepper: String,
}

fn default_prelude_api_url() -> Url {
    Url::parse("https://api.prelude.dev").expect("Default Prelude API URL is valid")
}

fn default_max_verified_sessions() -> u32 {
    10
}

impl EnvConfig {
    /// Load the environment configuration from the environment variables.
    /// The environment variables are prefixed with "HG_".
    pub fn load() -> Result<EnvConfig, envy::Error> {
        envy::prefixed("HG_").from_env::<EnvConfig>()
    }

    #[cfg(test)]
    pub fn for_test(prelude_api_url: Url, homeserver_admin_api_url: Url) -> Self {
        Self {
            database_url: Default::default(),
            http_listen_socket: "127.0.0.1:0".parse().unwrap(),
            prelude_api_key: "test-key".to_string(),
            prelude_api_url,
            homeserver_admin_api_url,
            homeserver_admin_password: "test-pass".to_string(),
            homeserver_pubky: "test-homeserver-pubky".to_string(),
            max_verified_sessions: 10,
            phone_number_pepper: "test-pepper-for-phone-number-hashing".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EnvConfig;

    #[test]
    fn test_load_config() {
        let config = envy::from_iter::<_, EnvConfig>([
            (
                String::from("DATABASE_URL"),
                String::from("postgres://localhost:5432/pubky_homegate"),
            ),
            (
                String::from("HTTP_LISTEN_SOCKET"),
                String::from("127.0.0.1:5000"),
            ),
            (
                String::from("PRELUDE_API_KEY"),
                String::from("test-prelude-api-key"),
            ),
            (
                String::from("HOMESERVER_ADMIN_API_URL"),
                String::from("http://localhost:6288"),
            ),
            (
                String::from("HOMESERVER_ADMIN_PASSWORD"),
                String::from("test-admin-password"),
            ),
            (
                String::from("HOMESERVER_PUBKY"),
                String::from("test-homeserver-pubky"),
            ),
            (
                String::from("PHONE_NUMBER_PEPPER"),
                String::from("test-pepper"),
            ),
        ])
        .expect("Failed to load config");

        assert_eq!(
            config.database_url.as_str(),
            "postgres://localhost:5432/pubky_homegate"
        );
        assert_eq!(config.prelude_api_key, "test-prelude-api-key");
        assert_eq!(
            config.homeserver_admin_api_url.as_str(),
            "http://localhost:6288/"
        );
        assert_eq!(config.homeserver_admin_password, "test-admin-password");
    }
}

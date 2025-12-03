use std::net::SocketAddr;

use crate::persistence::sql::connection_string::ConnectionString;

/// The environment configuration.
/// This is the configuration that is loaded from the environment variables.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EnvConfig {
    #[serde(default)]
    pub database_url: ConnectionString,
    pub http_listen_socket: SocketAddr,
    pub prelude_api_key: String,
    #[serde(default = "default_prelude_api_url")]
    pub prelude_api_url: String,
    pub homeserver_api_url: String,
    pub homeserver_admin_password: String,
    #[serde(default = "default_max_verified_sessions")]
    pub max_verified_sessions: u32,
}

fn default_prelude_api_url() -> String {
    "https://api.prelude.dev".to_string()
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
                String::from("HOMESERVER_API_URL"),
                String::from("http://localhost:6288"),
            ),
            (
                String::from("HOMESERVER_ADMIN_PASSWORD"),
                String::from("test-admin-password"),
            ),
        ])
        .expect("Failed to load config");

        assert_eq!(
            config.database_url.as_str(),
            "postgres://localhost:5432/pubky_homegate"
        );
        assert_eq!(config.prelude_api_key, "test-prelude-api-key");
        assert_eq!(config.homeserver_api_url, "http://localhost:6288");
        assert_eq!(config.homeserver_admin_password, "test-admin-password");
    }
}

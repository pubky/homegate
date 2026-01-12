use std::net::SocketAddr;

use crate::infrastructure::sql::ConnectionString;
use crate::sms_verification::PhoneNumber;
use url::Url;

/// The environment configuration.
/// This is the configuration that is loaded from the environment variables.
/// TODO: Use config.toml instead. Env is a bit limited eg for grouping config items into structs
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EnvConfig {
    #[serde(default)]
    pub database_url: ConnectionString,
    #[serde(default = "default_http_listen_socker")]
    pub http_listen_socket: SocketAddr,
    pub prelude_api_key: String,
    #[serde(default = "default_prelude_api_url")]
    pub prelude_api_url: Url,
    pub homeserver_admin_api_url: Url,
    pub homeserver_admin_password: String,
    pub homeserver_pubky: String,
    #[serde(default = "default_max_sms_verifications_per_week")]
    pub max_sms_verifications_per_week: u32,
    #[serde(default = "default_max_sms_verifications_per_year")]
    pub max_sms_verifications_per_year: u32,
    #[serde(default)]
    pub sms_verifications_limit_whitelist: Vec<PhoneNumber>,
    #[serde(default = "default_lightning_verification_price_sat")]
    pub lightning_invoice_price_sat: u64,
    #[serde(default = "default_lightning_verification_expiry_seconds")]
    pub lightning_invoice_expiry_seconds: u64,
    #[serde(default = "default_lightning_verification_description")]
    pub lightning_invoice_description: String,
    pub phoenixd_api_url: Url,
    pub phoenixd_api_password: String,
    #[serde(default = "default_allow_cors")]
    pub allow_cors: bool,
}

fn default_allow_cors() -> bool {
    false
}

fn default_max_sms_verifications_per_week() -> u32 {
    2
}

fn default_max_sms_verifications_per_year() -> u32 {
    4
}

fn default_lightning_verification_expiry_seconds() -> u64 {
    60 * 10
}

fn default_lightning_verification_description() -> String {
    "Pubky Homegate Verification".to_string()
}

fn default_lightning_verification_price_sat() -> u64 {
    1000
}

fn default_prelude_api_url() -> Url {
    Url::parse("https://api.prelude.dev").expect("Default Prelude API URL is valid")
}

fn default_http_listen_socker() -> SocketAddr {
    "0.0.0.0:8080"
        .parse()
        .expect("Default HTTP listen socket is valid")
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
            http_listen_socket: "127.0.0.1:0"
                .parse()
                .expect("Default HTTP listen socket is valid"),
            prelude_api_key: "test-key".to_string(),
            prelude_api_url,
            homeserver_admin_api_url,
            homeserver_admin_password: "test-pass".to_string(),
            homeserver_pubky: "test-homeserver-pubky".to_string(),
            max_sms_verifications_per_week: 2,
            max_sms_verifications_per_year: 4,
            sms_verifications_limit_whitelist: vec![],
            allow_cors: true,
            lightning_invoice_price_sat: 1000,
            lightning_invoice_expiry_seconds: 60 * 10,
            lightning_invoice_description: "Verification".to_string(),
            phoenixd_api_url: Url::parse("http://localhost:9740")
                .expect("Default Phoenixd API URL is valid"),
            phoenixd_api_password:
                "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef".to_string(),
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
                String::from("lightning_invoice_price_sat"),
                String::from("1000"),
            ),
            (
                String::from("lightning_invoice_expiry_seconds"),
                String::from("600"),
            ),
            (
                String::from("lightning_invoice_description"),
                String::from("Verification"),
            ),
            (
                String::from("PHOENIXD_API_URL"),
                String::from("http://localhost:9740"),
            ),
            (
                String::from("PHOENIXD_API_PASSWORD"),
                String::from("test-password"),
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

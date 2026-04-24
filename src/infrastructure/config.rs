use std::net::SocketAddr;

use crate::infrastructure::sql::ConnectionString;
use crate::sms_verification::PhoneNumber;
use url::Url;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub database_url: ConnectionString,
    #[serde(default = "default_http_listen_socket")]
    pub http_listen_socket: SocketAddr,
    #[serde(default)]
    pub allow_cors: bool,
    #[serde(default)]
    pub accept_proxy_ip_headers: bool,
    pub homeserver: HomeserverConfig,
    pub sms_verification: Option<SmsVerificationConfig>,
    pub ln_verification: Option<LnVerificationConfig>,
    pub ip_verification: Option<IpVerificationConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HomeserverConfig {
    pub admin_api_url: Url,
    pub admin_password: String,
    pub pubky: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SmsVerificationConfig {
    pub prelude_api_key: String,
    #[serde(default = "default_prelude_api_url")]
    pub prelude_api_url: Url,
    #[serde(default = "default_max_sms_verifications_per_week")]
    pub max_verifications_per_week: u32,
    #[serde(default = "default_max_sms_verifications_per_year")]
    pub max_verifications_per_year: u32,
    /// Maximum number of failed code validation attempts per verification session.
    /// After this many failed attempts, the session is marked as failed.
    /// Prelude seems to fail silently after 5 failures regardless of how its configured.
    /// We count attempts here to guard against this.
    #[serde(default = "default_max_sms_failed_validation_attempts")]
    pub max_failed_validation_attempts: u32,
    #[serde(default)]
    pub limit_whitelist: Vec<PhoneNumber>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LnVerificationConfig {
    pub phoenixd_api_url: Url,
    pub phoenixd_api_password: String,
    #[serde(default = "default_lightning_invoice_price_sat")]
    pub invoice_price_sat: u64,
    #[serde(default = "default_lightning_invoice_expiry_seconds")]
    pub invoice_expiry_seconds: u64,
    #[serde(default = "default_lightning_invoice_description")]
    pub invoice_description: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IpVerificationConfig {
    #[serde(default = "default_max_ip_verifications_per_week")]
    pub max_verifications_per_week: u32,
    #[serde(default = "default_max_ip_verifications_per_year")]
    pub max_verifications_per_year: u32,
}

fn default_http_listen_socket() -> SocketAddr {
    "0.0.0.0:8080"
        .parse()
        .expect("Default HTTP listen socket is valid")
}

fn default_prelude_api_url() -> Url {
    Url::parse("https://api.prelude.dev").expect("Default Prelude API URL is valid")
}

fn default_max_sms_verifications_per_week() -> u32 {
    2
}

fn default_max_sms_verifications_per_year() -> u32 {
    4
}

fn default_max_sms_failed_validation_attempts() -> u32 {
    5
}

fn default_lightning_invoice_price_sat() -> u64 {
    1000
}

fn default_lightning_invoice_expiry_seconds() -> u64 {
    60 * 10
}

fn default_lightning_invoice_description() -> String {
    "Pubky Homegate Verification".to_string()
}

fn default_max_ip_verifications_per_week() -> u32 {
    2
}

fn default_max_ip_verifications_per_year() -> u32 {
    4
}

impl AppConfig {
    /// Load configuration from a TOML file.
    /// Path is read from `HG_CONFIG_PATH` env var, defaulting to `config.toml`.
    pub fn load() -> anyhow::Result<AppConfig> {
        let path = std::env::var("HG_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;
        let config: AppConfig = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse '{}': {}", path, e))?;
        Ok(config)
    }
}

#[cfg(test)]
impl SmsVerificationConfig {
    pub fn for_test(prelude_api_url: Url) -> Self {
        Self {
            prelude_api_key: "test-key".to_string(),
            prelude_api_url,
            max_verifications_per_week: 2,
            max_verifications_per_year: 4,
            max_failed_validation_attempts: 2,
            limit_whitelist: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config_all_routes() {
        let toml = r#"
database_url = "postgres://localhost:5432/pubky_homegate"
http_listen_socket = "127.0.0.1:5000"

[homeserver]
admin_api_url = "http://localhost:6288"
admin_password = "test-admin-password"
pubky = "test-homeserver-pubky"

[sms_verification]
prelude_api_key = "test-prelude-api-key"

[ln_verification]
phoenixd_api_url = "http://localhost:9740"
phoenixd_api_password = "test-password"

[ip_verification]
"#;
        let config: AppConfig = toml::from_str(toml).expect("Failed to parse config");
        assert_eq!(
            config.database_url.as_str(),
            "postgres://localhost:5432/pubky_homegate"
        );
        assert_eq!(config.homeserver.admin_password, "test-admin-password");
        assert!(config.sms_verification.is_some());
        assert!(config.ln_verification.is_some());
        assert!(config.ip_verification.is_some());
    }

    #[test]
    fn test_ln_verification_requires_phoenixd_url() {
        let toml = r#"
[homeserver]
admin_api_url = "http://localhost:6288"
admin_password = "test-admin-password"
pubky = "test-homeserver-pubky"

[ln_verification]
phoenixd_api_password = "test-password"
"#;
        let err = toml::from_str::<AppConfig>(toml).unwrap_err();
        assert!(
            err.to_string().contains("phoenixd_api_url"),
            "Should require phoenixd_api_url, got: {err}"
        );
    }

    #[test]
    fn test_sms_verification_requires_prelude_api_key() {
        let toml = r#"
[homeserver]
admin_api_url = "http://localhost:6288"
admin_password = "test-admin-password"
pubky = "test-homeserver-pubky"

[sms_verification]
"#;
        let err = toml::from_str::<AppConfig>(toml).unwrap_err();
        assert!(
            err.to_string().contains("prelude_api_key"),
            "Should require prelude_api_key, got: {err}"
        );
    }

    #[test]
    fn test_homeserver_config_is_required() {
        let toml = r#"
database_url = "postgres://localhost:5432/pubky_homegate"
"#;
        let err = toml::from_str::<AppConfig>(toml).unwrap_err();
        assert!(
            err.to_string().contains("homeserver"),
            "Should require homeserver section, got: {err}"
        );
    }

    #[test]
    fn test_load_config_no_optional_routes() {
        let toml = r#"
[homeserver]
admin_api_url = "http://localhost:6288"
admin_password = "test-admin-password"
pubky = "test-homeserver-pubky"
"#;
        let config: AppConfig = toml::from_str(toml).expect("Failed to parse config");
        assert!(config.sms_verification.is_none());
        assert!(config.ln_verification.is_none());
        assert!(config.ip_verification.is_none());
    }
}

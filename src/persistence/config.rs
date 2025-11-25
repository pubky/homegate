use crate::persistence::sql::connection_string::ConnectionString;


/// The environment configuration.
/// This is the configuration that is loaded from the environment variables.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EnvConfig {
    #[serde(default)]
    pub database_url: ConnectionString,
    pub prelude_api_key: String,
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
                String::from("PRELUDE_API_KEY"),
                String::from("test-prelude-api-key"),
            ),
        ])
            .expect("Failed to load config");

        assert_eq!(config.database_url.as_str(), "postgres://localhost:5432/pubky_homegate");
        assert_eq!(config.prelude_api_key, "test-prelude-api-key");
    }
}
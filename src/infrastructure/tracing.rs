use tracing_subscriber::EnvFilter;

use super::config::LoggingConfig;

pub fn init_tracing(logging: Option<&LoggingConfig>) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| match logging {
        Some(logging) => {
            let mut filter = EnvFilter::new(&logging.level);
            for module_level in &logging.module_levels {
                filter = filter.add_directive(
                    module_level
                        .parse()
                        .expect("Invalid module_levels directive in config"),
                );
            }
            filter
        }
        None => EnvFilter::new("info"),
    });
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}

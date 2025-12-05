use anyhow::Context;
use homegate::{EnvConfig, HttpServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file.
    // Init tracing after  the .env file is loaded so that the logging is configured.
    // If the .env loading fails, log it after tracing is actually initialized.
    let dot_env_load_result = dotenvy::dotenv();
    tracing_subscriber::fmt::init();
    if let Err(e) = dot_env_load_result {
        tracing::debug!("Failed to load .env file: {}", e);
    };

    let config = EnvConfig::load().context("Failed to load config")?;

    let http_server = HttpServer::start(config).await?;

    tracing::info!("Homeserver HTTP listening on {}", http_server.url_string());

    tracing::info!("Press Ctrl+C to stop the Homeserver");
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Homeserver");
    Ok(())
}

mod infrastructure;
mod ip_verification;
mod ln_verification;
mod shared;
mod sms_verification;

#[cfg(test)]
mod e2e;

use anyhow::Context;
use infrastructure::{AppConfig, http::HttpServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig::load().context("Failed to load config")?;

    let http_server = HttpServer::start(config).await?;

    tracing::info!("Homegate HTTP listening on {}", http_server.url_string());

    tracing::info!("Press Ctrl+C to stop Homegate");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

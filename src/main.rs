mod google_verification;
mod infrastructure;
mod ip_verification;
mod ln_verification;
mod shared;
mod sms_verification;

#[cfg(test)]
mod e2e;

use anyhow::Context;
use clap::Parser;
use infrastructure::{AppConfig, DataDir, http::HttpServer};
use shared::HasherArgon2id;

#[derive(Parser)]
#[command(name = "homegate", about = "Pubky Homegate service")]
struct Cli {
    /// Path to the data directory (contains config.toml, pepper.txt, etc.)
    #[arg(long, default_value_os_t = DataDir::default_path())]
    data_dir: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = DataDir::new(cli.data_dir).context("Invalid data directory")?;

    let config = AppConfig::load(&data_dir).context("Failed to load config")?;

    infrastructure::tracing::init_tracing(config.logging.as_ref());

    let hasher = HasherArgon2id::new(data_dir.pepper_file_path());
    let http_server = HttpServer::start(config, hasher).await?;

    tracing::info!("Homegate HTTP listening on {}", http_server.url_string());

    tracing::info!("Press Ctrl+C to stop Homegate");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

mod persistence;
mod app_context;


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

    let context = app_context::AppContext::load().await.map_err(|e| e.context("Failed to load application context"))?;
    
    Ok(())
}

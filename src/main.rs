use anyhow::Context;
use homegate::{
    AppState, HomeserverAdminApi, HttpServer, MockSmsVerificationProviderApi,
    SmsVerificationService,
};

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

    let config = homegate::EnvConfig::load().context("Failed to load config")?;

    let db = homegate::Db::connect(&config.database_url)
        .await
        .context("Failed to connect to database and run migrations")?;

    // Initialize SMS verification service with mock Prelude API and real Homeserver Admin API
    let mock_prelude_api = MockSmsVerificationProviderApi::new();
    let homeserver_admin_api = HomeserverAdminApi::from_config(&config);
    let sms_verification_service =
        SmsVerificationService::new(mock_prelude_api, db.clone(), homeserver_admin_api);

    let state = AppState {
        db,
        sms_verification_service,
    };

    let http_server = HttpServer::start(config.http_listen_socket, state).await?;

    tracing::info!("Homeserver HTTP listening on {}", http_server.url_string());

    tracing::info!("Press Ctrl+C to stop the Homeserver");
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Homeserver");
    Ok(())
}

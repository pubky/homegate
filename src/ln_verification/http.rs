use std::process;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::{
    infrastructure::{config::LnVerificationConfig, http::HttpServerError},
    ln_verification::{
        VerificationId, app_state::AppState, error::LnVerificationError, phoenixd_api::PhoenixdAPI,
        service::LnVerificationService, types::VerificationResponse,
    },
    shared::{HomeserverAdminAPI, Metrics},
};

/// Default timeout for long-polling verification requests
/// Set to 25 seconds to be below common reverse proxy timeouts (30 seconds)
/// This way, you don't have to deal with proxy timeouts like nginx cutting the connection
/// before the application can respond with a timeout message.
const DEFAULT_TIMEOUT_SECS: u64 = 25;

pub async fn router(
    homeserver_api: &HomeserverAdminAPI,
    ln: &LnVerificationConfig,
    db: crate::infrastructure::sql::SqlDb,
    metrics: Metrics,
) -> Result<Router, HttpServerError> {
    let phoenixd_api = PhoenixdAPI::new(
        &ln.phoenixd_api_url,
        &ln.phoenixd_api_password,
        metrics.clone(),
    );

    let ln_service = LnVerificationService::new(
        db,
        phoenixd_api,
        homeserver_api.clone(),
        ln.invoice_price_sat,
        ln.invoice_description.clone(),
        ln.invoice_expiry_seconds,
        metrics,
    );
    let ln_service = std::sync::Arc::new(ln_service);

    let service_for_task = ln_service.clone();
    tokio::task::spawn(async move {
        if let Err(e) = service_for_task.run_background_sync().await {
            tracing::error!(error = %e, "Error running invoice background syncer");
            process::exit(1); // Force a restart of the server
        }
    });

    let state = AppState::new(ln_service, homeserver_api.clone());
    Ok(Router::new()
        .route("/", post(create_verification_handler))
        .route("/{id}", get(get_verification_handler))
        .route("/{id}/await", get(await_verification_handler))
        .route("/info", get(get_info_handler))
        // /price kept for backwards compatibility. Remove after removed from Franky.
        .route("/price", get(get_info_handler))
        .with_state(state))
}

/// Create a new Lightning Network verification handler
async fn create_verification_handler(
    State(state): State<AppState>,
) -> Result<Json<CreateVerificationResponse>, LnVerificationError> {
    let (verification, invoice) = state.ln_service.create_verification().await?;
    tracing::info!("Created verification {}", verification.id);
    let response = CreateVerificationResponse {
        id: verification.id,
        bolt11_invoice: invoice.invoice,
        amount_sat: invoice.requested_sat,
        expires_at: invoice.expires_at.timestamp_millis(),
    };
    Ok(Json(response))
}

/// Get a Lightning Network verification handler
async fn get_verification_handler(
    State(state): State<AppState>,
    Path(id): Path<VerificationId>,
) -> Response {
    let verification = match state.ln_service.get_and_sync_verification(&id).await {
        Ok(Some(verification)) => verification,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response();
        }
        Err(e) => return e.into_response(),
    };

    Json(GetVerificationResponse::from_entity(
        verification,
        state.homeserver_api.get_homeserver_pubky(),
    ))
    .into_response()
}

/// Await for a Lightning Network verification to be finalized handler
async fn await_verification_handler(
    State(state): State<AppState>,
    Path(id): Path<VerificationId>,
) -> impl IntoResponse {
    let response = match state
        .ln_service
        .get_and_await_verification(&id, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .await
    {
        Ok(Some(response)) => response,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response();
        }
        Err(e) => return e.into_response(),
    };

    match response {
        VerificationResponse::Success(verification) => {
            if verification.is_finalised() {
                tracing::info!("Awaited verification {}", verification.id);
            }
            Json(GetVerificationResponse::from_entity(
                verification,
                state.homeserver_api.get_homeserver_pubky(),
            ))
            .into_response()
        }
        VerificationResponse::TimedOut => (
            StatusCode::REQUEST_TIMEOUT,
            "Long poll timeout. Please try again.".to_string(),
        )
            .into_response(),
    }
}

/// Get Lightning verification info handler.
/// Used by Franky to check if the user is in a country which allows Lightning verification.
/// IP-based geo-blocking is done at the nginx level; this endpoint confirms service availability.
async fn get_info_handler(State(state): State<AppState>) -> Json<GetInfoResponse> {
    Json(GetInfoResponse {
        amount_sat: state.ln_service.get_price_sat(),
    })
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVerificationResponse {
    id: VerificationId,
    bolt11_invoice: String,
    amount_sat: u64,
    expires_at: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVerificationResponse {
    id: VerificationId,
    amount_sat: u64,
    expires_at: i64,
    is_paid: bool,
    signup_code: Option<String>,
    homeserver_pubky: String,
    created_at: i64,
}

impl GetVerificationResponse {
    fn from_entity(
        entity: crate::ln_verification::LightningVerificationEntity,
        homeserver_pubky: String,
    ) -> Self {
        let is_paid = entity.is_finalised();
        Self {
            id: entity.id,
            amount_sat: entity.amount_sat as u64,
            expires_at: entity.expires_at.and_utc().timestamp_millis(),
            is_paid,
            signup_code: entity.signup_code,
            homeserver_pubky,
            created_at: entity.created_at.and_utc().timestamp_millis(),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetInfoResponse {
    amount_sat: u64,
}

impl IntoResponse for LnVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            LnVerificationError::Phoenixd(ref err) => {
                tracing::error!(error = %err, "Phoenixd API error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            LnVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            LnVerificationError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during LN verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{
        WiremockServers, setup_homeserver_slow_distinct_signup_tokens, start_phoenixd_mock,
    };
    use crate::infrastructure::sql::SqlDb;
    use crate::ln_verification::phoenixd_api::PhoenixdAPI;
    use crate::shared::HomeserverAdminAPI;
    use axum_test::TestServer;
    use sqlx::PgPool;

    /// Creates a test router whose PhoenixD and homeserver URLs are placeholders,
    /// for endpoints such as /info that never call them.
    async fn create_test_router(pool: PgPool) -> Router {
        let placeholder_url = "http://localhost:1".parse().unwrap();
        create_test_router_with_dependencies(pool, &placeholder_url, &placeholder_url).await
    }

    /// Creates a test router for LN verification WITHOUT starting the background sync task.
    /// This is suitable for testing endpoints that don't require websocket connectivity.
    async fn create_test_router_with_dependencies(
        pool: PgPool,
        phoenixd_url: &url::Url,
        homeserver_url: &url::Url,
    ) -> Router {
        let db = SqlDb::test(pool).await;
        let metrics = Metrics::new();
        let phoenixd_api = PhoenixdAPI::new(phoenixd_url, "unused", metrics.clone());
        let homeserver_api = HomeserverAdminAPI::new(
            homeserver_url,
            "test-pass",
            "test-homeserver-pubky",
            metrics.clone(),
        );

        let ln_service = crate::ln_verification::service::LnVerificationService::new(
            db,
            phoenixd_api,
            homeserver_api.clone(),
            1000, // amount_sat
            "Test Invoice".to_string(),
            600, // invoice_expiry_seconds
            metrics,
        );
        let ln_service = std::sync::Arc::new(ln_service);

        // Note: We intentionally skip spawning the background sync task
        let state = AppState::new(ln_service, homeserver_api.clone());
        Router::new()
            .route("/", post(create_verification_handler))
            .route("/{id}", get(get_verification_handler))
            .route("/info", get(get_info_handler))
            .with_state(state)
    }

    /// Tests the /info endpoint returns 200 OK with the configured price.
    /// This endpoint is used by Franky to check if Lightning verification is available
    /// (geo-blocking is done at nginx level).
    #[sqlx::test]
    async fn test_info_endpoint_returns_ok_with_price(pool: PgPool) {
        let router = create_test_router(pool).await;
        let server = TestServer::new(router).expect("Failed to create test server");

        let response = server.get("/info").await;
        response.assert_status_ok();

        let body: serde_json::Value = response.json();
        assert_eq!(
            body["amountSat"], 1000,
            "Response should include the configured price"
        );
    }

    /// One paid invoice must yield one signup code, however many requests race for it.
    #[sqlx::test]
    async fn test_concurrent_get_verification_requests_return_one_signup_code(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(true).await;
        setup_homeserver_slow_distinct_signup_tokens()
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        let router = create_test_router_with_dependencies(
            pool,
            &phoenixd.uri().parse().unwrap(),
            &servers.homeserver_server.uri().parse().unwrap(),
        )
        .await;
        let server = TestServer::new(router).expect("Failed to create test server");
        let created: CreateVerificationResponse = server.post("/").await.json();
        let verification_path = format!("/{}", created.id);

        let (first, second) = tokio::join!(
            server.get(&verification_path).into_future(),
            server.get(&verification_path).into_future(),
        );

        let first: GetVerificationResponse = first.json();
        let second: GetVerificationResponse = second.json();
        assert!(first.signup_code.is_some());
        assert_eq!(second.signup_code, first.signup_code);
    }
}

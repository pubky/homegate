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
    infrastructure::{
        config::{HomeserverConfig, LnVerificationConfig},
        http::HttpServerError,
    },
    ln_verification::{
        VerificationId, app_state::AppState, error::LnVerificationError, phoenixd_api::PhoenixdAPI,
        service::LnVerificationService, types::VerificationResponse,
    },
    shared::HomeserverAdminAPI,
};

/// Default timeout for long-polling verification requests
/// Set to 25 seconds to be below common reverse proxy timeouts (30 seconds)
/// This way, you don't have to deal with proxy timeouts like nginx cutting the connection
/// before the application can respond with a timeout message.
const DEFAULT_TIMEOUT_SECS: u64 = 25;

pub async fn router(
    homeserver: &HomeserverConfig,
    ln: &LnVerificationConfig,
    db: crate::infrastructure::sql::SqlDb,
) -> Result<Router, HttpServerError> {
    let phoenixd_api = PhoenixdAPI::new(&ln.phoenixd_api_url, &ln.phoenixd_api_password);
    let homeserver_api = HomeserverAdminAPI::new(
        &homeserver.admin_api_url,
        &homeserver.admin_password,
        &homeserver.pubky,
    );

    let ln_service = LnVerificationService::new(
        db,
        phoenixd_api,
        homeserver_api.clone(),
        ln.invoice_price_sat,
        ln.invoice_description.clone(),
        ln.invoice_expiry_seconds,
    );
    let ln_service = std::sync::Arc::new(ln_service);

    let service_for_task = ln_service.clone();
    tokio::task::spawn(async move {
        if let Err(e) = service_for_task.run_background_sync().await {
            tracing::error!(error = %e, "Error running invoice background syncer");
            process::exit(1); // Force a restart of the server
        }
    });

    let state = AppState::new(ln_service, homeserver_api);
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
    use crate::infrastructure::sql::SqlDb;
    use crate::ln_verification::phoenixd_api::PhoenixdAPI;
    use crate::shared::HomeserverAdminAPI;
    use axum_test::TestServer;
    use sqlx::PgPool;

    /// Creates a test router for LN verification WITHOUT starting the background sync task.
    /// This is suitable for testing endpoints that don't require websocket connectivity.
    async fn create_test_router(pool: PgPool) -> Router {
        let db = SqlDb::test(pool).await;
        // Use placeholder URLs - these won't be called for the /info endpoint
        let phoenixd_api = PhoenixdAPI::new(&"http://localhost:1".parse().unwrap(), "unused");
        let homeserver_api = HomeserverAdminAPI::new(
            &"http://localhost:1".parse().unwrap(),
            "unused",
            "test-homeserver-pubky",
        );

        let ln_service = crate::ln_verification::service::LnVerificationService::new(
            db,
            phoenixd_api,
            homeserver_api.clone(),
            1000, // amount_sat
            "Test Invoice".to_string(),
            600, // invoice_expiry_seconds
        );
        let ln_service = std::sync::Arc::new(ln_service);

        // Note: We intentionally skip spawning the background sync task for this test
        let state = AppState::new(ln_service, homeserver_api);
        Router::new()
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
}

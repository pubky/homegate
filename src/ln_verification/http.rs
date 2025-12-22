use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::{
    EnvConfig,
    infrastructure::http::HttpServerError,
    ln_verification::{app_state::AppState, error::LnVerificationError, payment_hash::PaymentHash},
};

pub async fn router(
    config: &EnvConfig,
    db: &crate::infrastructure::sql::SqlDb,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db.clone()).await;
    Ok(Router::new()
        .route("/", post(create_verification_handler))
        .route("/{id}", get(get_verification_handler))
        .route("/{id}/await", get(await_verification_handler))
        .with_state(state))
}

/// Create a new Lightning Network verification handler
async fn create_verification_handler(
    State(state): State<AppState>,
) -> Result<Json<CreateVerificationResponse>, LnVerificationError> {
    let (verification, invoice) = state.ln_service.create_verification().await?;
    tracing::info!("Created verification {}", verification.payment_hash);
    let response = CreateVerificationResponse {
        id: verification.payment_hash,
        bolt11_invoice: invoice.invoice,
        amount_sat: invoice.requested_sat,
        expires_at: invoice.expires_at.timestamp_millis(),
    };
    Ok(Json(response))
}

/// Get a Lightning Network verification handler
async fn get_verification_handler(
    State(state): State<AppState>,
    Path(id): Path<PaymentHash>,
) -> Response {
    let verification = match state.ln_service.get_verification(&id).await {
        Ok(Some(verification)) => verification,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response(),
        Err(e) => return e.into_response(),
    };
    Json(GetVerificationResponse {
        id: verification.payment_hash.clone(),
        amount_sat: verification.amount_sat as u64,
        expires_at: verification.expires_at.and_utc().timestamp_millis(),
        is_paid: verification.is_finalised(),
        signup_code: verification.signup_code,
        homeserver_pubky: state.homeserver_api.get_homeserver_pubky(),
        created_at: verification.created_at.and_utc().timestamp_millis(),
    })
    .into_response()
}

/// Await for a Lightning Network verification to be finalized handler
async fn await_verification_handler(
    State(state): State<AppState>,
    Path(id): Path<PaymentHash>,
) -> impl IntoResponse {
    let mut verification = match state.ln_service.get_verification(&id).await {
        Ok(Some(verification)) => verification,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response(),
        Err(e) => return e.into_response(),
    };

    if !verification.is_finalised() {
        verification = match state
            .syncer
            .wait_for_payment(&id, Duration::from_secs(60))
            .await
        {
            Ok(Some(verification)) => verification,
            Ok(None) => {
                return (
                    StatusCode::REQUEST_TIMEOUT,
                    "Long poll timeout. Please try again.".to_string(),
                )
                    .into_response();
            }
            Err(e) => return e.into_response(),
        };
        tracing::info!("Awaited verification {}", verification.payment_hash);
    };

    Json(GetVerificationResponse {
        id: verification.payment_hash.clone(),
        amount_sat: verification.amount_sat as u64,
        expires_at: verification.expires_at.and_utc().timestamp_millis(),
        is_paid: verification.is_finalised(),
        signup_code: verification.signup_code,
        homeserver_pubky: state.homeserver_api.get_homeserver_pubky(),
        created_at: verification.created_at.and_utc().timestamp_millis(),
    })
    .into_response()
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVerificationResponse {
    id: PaymentHash,
    bolt11_invoice: String,
    amount_sat: u64,
    expires_at: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVerificationResponse {
    id: PaymentHash,
    amount_sat: u64,
    expires_at: i64,
    is_paid: bool,
    signup_code: Option<String>,
    homeserver_pubky: String,
    created_at: i64,
}

impl IntoResponse for LnVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            LnVerificationError::Phoenixd(ref err) => {
                tracing::error!(error = %err, "Phoenixd API error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            LnVerificationError::Homeserver(ref err) => {
                tracing::error!(error = %err, "Homeserver API error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            LnVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, "Internal Server Error").into_response()
    }
}

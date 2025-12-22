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
    EnvConfig,
    infrastructure::http::HttpServerError,
    ln_verification::{
        app_state::AppState, error::LnVerificationError,
        invoice_background_syncer::InvoiceBackgroundSyncer, payment_hash::PaymentHash,
        phoenixd_api::PhoenixdAPI, service::LnVerificationService,
    },
    shared::HomeserverAdminAPI,
};

pub async fn router(
    config: &EnvConfig,
    db: &crate::infrastructure::sql::SqlDb,
) -> Result<Router, HttpServerError> {
    let phoenixd_api = PhoenixdAPI::new(&config.phoenixd_api_url, &config.phoenixd_api_password);
    let homeserver_api = HomeserverAdminAPI::new(
        &config.homeserver_admin_api_url,
        &config.homeserver_admin_password,
        &config.homeserver_pubky,
    );
    let ln_service = LnVerificationService::new(
        db.clone(),
        phoenixd_api.clone(),
        homeserver_api.clone(),
        config.lightning_invoice_price_sat,
        config.lightning_invoice_description.clone(),
        config.lightning_invoice_expiry_seconds,
    );

    let syncer = InvoiceBackgroundSyncer::new(ln_service.clone(), phoenixd_api).await;
    let syncer_for_task = syncer.clone();
    tokio::task::spawn(async move {
        if let Err(e) = syncer_for_task.run().await {
            tracing::error!(error = %e, "Error running invoice background syncer");
            process::exit(1); // Force a restart of the server
        }
    });

    let state = AppState::new(syncer, ln_service, homeserver_api);
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
    Json(GetVerificationResponse::from_entity(
        verification,
        state.homeserver_api.get_homeserver_pubky(),
    ))
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

    Json(GetVerificationResponse::from_entity(
        verification,
        state.homeserver_api.get_homeserver_pubky(),
    ))
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

impl GetVerificationResponse {
    fn from_entity(
        entity: crate::ln_verification::LightningVerificationEntity,
        homeserver_pubky: String,
    ) -> Self {
        let is_paid = entity.is_finalised();
        Self {
            id: entity.payment_hash,
            amount_sat: entity.amount_sat as u64,
            expires_at: entity.expires_at.and_utc().timestamp_millis(),
            is_paid,
            signup_code: entity.signup_code,
            homeserver_pubky,
            created_at: entity.created_at.and_utc().timestamp_millis(),
        }
    }
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

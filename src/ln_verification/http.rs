use std::{process, time::Duration};

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
        error::LnVerificationError, invoice_background_syncer::InvoiceBackgroundSyncer,
        ln_context::LnContext, payment_hash::PaymentHash, service::LnVerificationService,
    },
    shared::HomeserverAdminAPI,
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub syncer: InvoiceBackgroundSyncer,
    pub ln_service: LnVerificationService,
    pub homeserver_api: HomeserverAdminAPI,
}

impl AppState {
    /// Create a new AppState instance
    /// This will start the invoice background syncer in a new background task.
    pub async fn new(config: &EnvConfig, db: crate::infrastructure::sql::SqlDb) -> Self {
        let context = LnContext::new(db, config);
        let syncer = InvoiceBackgroundSyncer::new(&context).await;
        let syncer_clone = syncer.clone();
        tokio::task::spawn(async move {
            if let Err(e) = syncer_clone.run().await {
                tracing::error!(error = %e, "Error running invoice background syncer");
                process::exit(1); // Force a restart of the server
            }
        });
        Self {
            syncer,
            ln_service: context.service.clone(),
            homeserver_api: context.homeserver_api.clone(),
        }
    }
}

pub async fn router(
    config: &EnvConfig,
    db: &crate::infrastructure::sql::SqlDb,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db.clone()).await;
    Ok(Router::new()
        .route("/", post(create_verification_handler))
        .route("/{payment_hash}", get(get_verification_handler))
        .route("/{payment_hash}/await", get(await_verification_handler))
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
    Path(payment_hash): Path<PaymentHash>,
) -> Response {
    let verification = match state.ln_service.get_verification(&payment_hash).await {
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
    Path(payment_hash): Path<PaymentHash>,
) -> impl IntoResponse {
    let mut verification = match state.ln_service.get_verification(&payment_hash).await {
        Ok(Some(verification)) => verification,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response(),
        Err(e) => return e.into_response(),
    };

    if !verification.is_finalised() {
        verification = match state
            .syncer
            .wait_for_payment(&payment_hash, Duration::from_secs(60))
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

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
        error::LnVerificationError, invoice_background_syncer::InvoiceBackgroundSyncer, ln_context::LnContext, payment_hash::PaymentHash, service::LnVerificationService
    }, shared::HomeserverAdminAPI,
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
        .route("/{payment_hash}/await", get(wait_for_verification_handler))
        .with_state(state))
}

#[axum::debug_handler]
async fn create_verification_handler(
    State(state): State<AppState>,
) -> Result<Json<RequestVerificationResponse>, LnVerificationError> {
    let (verification, invoice) = state.ln_service.create_verification().await?;
    let response = RequestVerificationResponse {
        id: verification.payment_hash,
        bolt11_invoice: invoice.invoice,
        amount_sat: invoice.requested_sat,
        expires_at_timestamp: invoice.expires_at.timestamp_millis(),
    };
    Ok(Json(response))
}

async fn wait_for_verification_handler(
    State(state): State<AppState>,
    Path(payment_hash): Path<PaymentHash>,
) -> impl IntoResponse {
    let verification = match state.ln_service.get_verification(&payment_hash).await {
        Ok(Some(verification)) => verification,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response(),
        Err(e) => return e.into_response(),
    };

    if !verification.is_finalised() {
        tokio::select! {
            _ = wait_for_payment(&state, &payment_hash) => {}
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                // If the payment is not finalized after 60 seconds, return a timeout error
                // This is to keep the number of connections open to a minimum.
                return (StatusCode::REQUEST_TIMEOUT, "Long poll timeout. Please try again.".to_string()).into_response();
            }
        };
    };


    Json(PaymentVerifiedResponse {
        id: verification.payment_hash,
        signup_code: verification.signup_code.unwrap(),
        homeserver_pubky: state.homeserver_api.get_homeserver_pubky(),
    }).into_response()
}

async fn wait_for_payment(state: &AppState, payment_hash: &PaymentHash) -> Result<(), LnVerificationError> {
    let mut receiver = state.syncer.subscribe();
    loop {
        let verification = receiver.recv().await.expect("Should never happen");
        if &verification.payment_hash == payment_hash {
            return Ok(());
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestVerificationResponse {
    id: PaymentHash,
    bolt11_invoice: String,
    amount_sat: u64,
    expires_at_timestamp: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentVerifiedResponse {
    id: PaymentHash,
    signup_code: String,
    homeserver_pubky: String,
}

impl IntoResponse for LnVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            LnVerificationError::Phoenixd(ref err) => {
                tracing::error!(error = %err, "Phoenixd API error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            LnVerificationError::PhoenixdWebsocket(ref err) => {
                tracing::error!(error = %err, "Phoenixd websocket error");
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

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::json;

use crate::sms_verification::{
    error::SmsVerificationError,
    types::{
        CreateVerificationRequest, CreateVerificationResponse, SendCodeRequest, SendCodeResponse,
    },
};
use crate::{
    EnvConfig, infrastructure::http::RequestOrigin, sms_verification::app_state::AppState,
};

pub async fn router(config: &EnvConfig) -> Result<Router, SmsVerificationError> {
    let state = AppState::from_config(config).await?;
    Ok(Router::new()
        .route("/send_code", post(send_code_handler))
        .route("/verify_code", post(verify_code_handler))
        .with_state(state))
}

#[cfg(test)]
pub async fn router_with_db(
    config: &EnvConfig,
    db: crate::infrastructure::database::SqlDb,
) -> Result<Router, SmsVerificationError> {
    use crate::sms_verification::app_state::AppState;

    let state = AppState::from_config_with_db(config, db)?;

    Ok(Router::new()
        .route("/send_code", post(send_code_handler))
        .route("/verify_code", post(verify_code_handler))
        .with_state(state))
}

async fn send_code_handler(
    State(state): State<AppState>,
    RequestOrigin(ip_address): RequestOrigin,
    Json(request): Json<CreateVerificationRequest>,
) -> Result<Json<CreateVerificationResponse>, SmsVerificationError> {
    let response = state
        .sms_verification
        .create_verification(request, ip_address)
        .await?;
    Ok(Json(response))
}

async fn verify_code_handler(
    State(state): State<AppState>,
    Json(request): Json<SendCodeRequest>,
) -> Result<Json<SendCodeResponse>, SmsVerificationError> {
    let response = state.sms_verification.send_code(request).await?;
    Ok(Json(response))
}

impl IntoResponse for SmsVerificationError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match self {
            SmsVerificationError::InvalidPhoneNumber(ref _phone) => (
                StatusCode::BAD_REQUEST,
                "invalid_phone_number",
                self.to_string(),
            ),
            SmsVerificationError::TooManyVerifiedSessions => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_verified_sessions",
                self.to_string(),
            ),
            SmsVerificationError::RequestFailed(ref err) => {
                tracing::error!(error = %err, "Failed to communicate with SMS provider");
                (
                    StatusCode::BAD_GATEWAY,
                    "external_service_error",
                    "Failed to communicate with SMS provider".to_string(),
                )
            }
            SmsVerificationError::ApiError {
                status,
                ref message,
            } => {
                tracing::error!(status = status, message = %message, "SMS provider API error");
                (
                    StatusCode::BAD_GATEWAY,
                    "external_service_error",
                    format!("SMS provider error ({}): {}", status, message),
                )
            }
            SmsVerificationError::InvalidResponse(ref msg) => {
                tracing::error!(message = %msg, "Invalid response from SMS provider");
                (
                    StatusCode::BAD_GATEWAY,
                    "external_service_error",
                    format!("Invalid response from SMS provider: {}", msg),
                )
            }
            SmsVerificationError::DatabaseError(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "Database operation failed".to_string(),
                )
            }
            SmsVerificationError::HomeserverAdminError(ref msg) => {
                tracing::error!(message = %msg, "Homeserver admin API error");
                (
                    StatusCode::BAD_GATEWAY,
                    "homeserver_error",
                    format!("Failed to generate signup token: {}", msg),
                )
            }
        };

        let body = Json(json!({
            "error": error_type,
            "message": message,
        }));

        (status, body).into_response()
    }
}

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
        CreateVerificationRequest, CreateVerificationRequestRaw, CreateVerificationResponse,
        SendCodeRequest, SendCodeRequestRaw, SendCodeResponse,
    },
};
use crate::{
    EnvConfig,
    infrastructure::http::{HttpServerError, RequestOrigin, ValidatedJson},
    sms_verification::app_state::AppState,
};

pub async fn router(
    config: &EnvConfig,
    db: crate::infrastructure::database::SqlDb,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db);
    Ok(Router::new()
        .route("/send_code", post(send_code_handler))
        .route("/verify_code", post(verify_code_handler))
        .with_state(state))
}

#[cfg(test)]
pub async fn router_with_db(
    config: &EnvConfig,
    db: crate::infrastructure::database::SqlDb,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db);
    Ok(Router::new()
        .route("/send_code", post(send_code_handler))
        .route("/verify_code", post(verify_code_handler))
        .with_state(state))
}

async fn send_code_handler(
    State(state): State<AppState>,
    RequestOrigin(ip_address): RequestOrigin,
    request: ValidatedJson<
        CreateVerificationRequest,
        CreateVerificationRequestRaw,
        SmsVerificationError,
    >,
) -> Result<Json<CreateVerificationResponse>, SmsVerificationError> {
    let response = state
        .sms_verification
        .create_verification(request.into_inner(), ip_address)
        .await?;
    Ok(Json(response))
}

async fn verify_code_handler(
    State(state): State<AppState>,
    request: ValidatedJson<SendCodeRequest, SendCodeRequestRaw, SmsVerificationError>,
) -> Result<Json<SendCodeResponse>, SmsVerificationError> {
    let response = state
        .sms_verification
        .send_code(request.into_inner())
        .await?;
    Ok(Json(response))
}

impl IntoResponse for SmsVerificationError {
    fn into_response(self) -> Response {
        let (status, error_type, message, retry_after) = match self {
            SmsVerificationError::InvalidPhoneNumber(ref _phone) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_phone_number",
                self.to_string(),
                None,
            ),
            SmsVerificationError::InvalidCode(ref _code) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_code",
                self.to_string(),
                None,
            ),
            SmsVerificationError::WeeklyLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "weekly_limit_exceeded",
                self.to_string(),
                None,
            ),
            SmsVerificationError::AnnualLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "annual_limit_exceeded",
                self.to_string(),
                None,
            ),
            SmsVerificationError::NoActiveVerification(ref _phone) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_active_verification",
                self.to_string(),
                None,
            ),
            SmsVerificationError::RateLimited { retry_after } => (
                StatusCode::TOO_MANY_REQUESTS,
                "external_service_rate_limited",
                self.to_string(),
                retry_after,
            ),
            SmsVerificationError::RequestFailed(ref err) => {
                tracing::error!(error = %err, "Failed to communicate with SMS provider");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "external_service_error",
                    "Failed to communicate with external API".to_string(),
                    None,
                )
            }
            SmsVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "Database operation failed".to_string(),
                    None,
                )
            }
        };

        let mut body = json!({
            "error": error_type,
            "message": message,
        });

        if let Some(seconds) = retry_after {
            body["retry_after"] = json!(seconds);
        }

        (status, Json(body)).into_response()
    }
}

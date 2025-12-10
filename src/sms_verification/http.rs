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
        let (status, error_type, message) = match self {
            SmsVerificationError::InvalidPhoneNumber(ref _phone) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_phone_number",
                self.to_string(),
            ),
            SmsVerificationError::TooManyVerifiedSessions => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "too_many_verified_sessions",
                self.to_string(),
            ),
            SmsVerificationError::NoActiveVerification(ref _phone) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_active_verification",
                self.to_string(),
            ),
            SmsVerificationError::RequestFailed(ref err) => {
                tracing::error!(error = %err, "Failed to communicate with SMS provider");
                (
                    StatusCode::BAD_GATEWAY,
                    "external_service_error",
                    "Failed to communicate with external API".to_string(),
                )
            }
            SmsVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "Database operation failed".to_string(),
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

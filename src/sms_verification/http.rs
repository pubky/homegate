use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::json;

use crate::infrastructure::http::{AppState, RequestOrigin};
use crate::shared::HomeserverAdminApiTrait;
use crate::sms_verification::{
    error::SmsVerificationError,
    prelude_api::SmsVerificationProviderApi,
    types::{SendCodeRequest, SendCodeResponse, VerifyCodeRequest, VerifyCodeResponse},
};

/// Mount SMS verification routes
pub fn routes<T, S>() -> Router<AppState<T, S>>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    Router::new().nest(
        "/sms_verification",
        Router::new()
            .route("/send_code", post(send_code_handler))
            .route("/verify_code", post(verify_code_handler)),
    )
}

async fn send_code_handler<T, S>(
    State(state): State<AppState<T, S>>,
    RequestOrigin(ip_address): RequestOrigin,
    Json(request): Json<SendCodeRequest>,
) -> Result<Json<SendCodeResponse>, SmsVerificationError>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    let response = state
        .sms_verification
        .send_code(request, ip_address)
        .await?;
    Ok(Json(response))
}

async fn verify_code_handler<T, S>(
    State(state): State<AppState<T, S>>,
    Json(request): Json<VerifyCodeRequest>,
) -> Result<Json<VerifyCodeResponse>, SmsVerificationError>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    let response = state.sms_verification.verify_code(request).await?;
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

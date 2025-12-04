use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::sms_verification::SmsVerificationError;

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

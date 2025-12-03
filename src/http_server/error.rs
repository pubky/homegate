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
            SmsVerificationError::RequestFailed(_) => (
                StatusCode::BAD_GATEWAY,
                "external_service_error",
                "Failed to communicate with SMS provider".to_string(),
            ),
            SmsVerificationError::ApiError {
                status,
                ref message,
            } => (
                StatusCode::BAD_GATEWAY,
                "external_service_error",
                format!("SMS provider error ({}): {}", status, message),
            ),
            SmsVerificationError::InvalidResponse(ref msg) => (
                StatusCode::BAD_GATEWAY,
                "external_service_error",
                format!("Invalid response from SMS provider: {}", msg),
            ),
            SmsVerificationError::DatabaseError(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                "Database operation failed".to_string(),
            ),
            SmsVerificationError::HomeserverAdminError(_) => (
                StatusCode::BAD_GATEWAY,
                "homeserver_error",
                "Failed to generate signup token".to_string(),
            ),
        };

        let body = Json(json!({
            "error": error_type,
            "message": message,
        }));

        (status, body).into_response()
    }
}

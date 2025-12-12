use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};

use crate::sms_verification::{
    error::SmsVerificationError,
    types::{CreateVerificationRequest, ValidateCodeRequest, ValidateCodeResponse},
};
use crate::{
    EnvConfig,
    infrastructure::http::{HttpServerError, RequestOrigin},
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
    Json(request): Json<CreateVerificationRequest>,
) -> Result<StatusCode, SmsVerificationError> {
    state
        .sms_verification
        .create_verification(request, ip_address)
        .await?;
    Ok(StatusCode::OK)
}

async fn verify_code_handler(
    State(state): State<AppState>,
    Json(request): Json<ValidateCodeRequest>,
) -> Result<Json<ValidateCodeResponse>, SmsVerificationError> {
    let response = state.sms_verification.validate_code(request).await?;
    Ok(Json(response))
}

impl IntoResponse for SmsVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            SmsVerificationError::InvalidPhoneNumber(_) => StatusCode::UNPROCESSABLE_ENTITY,
            SmsVerificationError::InvalidCode(_) => StatusCode::UNPROCESSABLE_ENTITY,
            SmsVerificationError::Blocked => StatusCode::FORBIDDEN,
            SmsVerificationError::WeeklyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            SmsVerificationError::AnnualLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            SmsVerificationError::NoActiveVerification(_) => StatusCode::UNPROCESSABLE_ENTITY,
            SmsVerificationError::RateLimited { retry_after } => {
                let mut response =
                    (StatusCode::TOO_MANY_REQUESTS, self.to_string()).into_response();
                if let Some(seconds) = retry_after {
                    response
                        .headers_mut()
                        .insert("Retry-After", seconds.to_string().parse().unwrap());
                }
                return response;
            }
            SmsVerificationError::RequestFailed(ref err) => {
                tracing::error!(error = %err, "Failed to communicate with SMS provider");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            SmsVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // If it isnt clear, self.to_string() uses the Display impl for each SmsVerificationError as the Response body
        (status, self.to_string()).into_response()
    }
}

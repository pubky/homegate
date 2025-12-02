mod error;

use axum::{Json, Router, extract::State, routing::post};

use crate::{
    external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi},
    http_server::AppState,
    sms_verification::{
        SendCodeRequest, SendCodeResponse, SmsVerificationError, VerifyCodeRequest,
        VerifyCodeResponse,
    },
};

/// Mount SMS verification routes
pub fn routes<
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
>() -> Router<AppState<T, S>> {
    Router::new().nest(
        "/sms_verification",
        Router::new()
            .route("/send_code", post(sms_send_code_handler))
            .route("/verify_code", post(sms_verify_code_handler)),
    )
}

async fn sms_send_code_handler<
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
>(
    State(state): State<AppState<T, S>>,
    Json(request): Json<SendCodeRequest>,
) -> Result<Json<SendCodeResponse>, SmsVerificationError> {
    let response = state.sms_verification_service.send_code(request).await?;
    Ok(Json(response))
}

async fn sms_verify_code_handler<
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
>(
    State(state): State<AppState<T, S>>,
    Json(request): Json<VerifyCodeRequest>,
) -> Result<Json<VerifyCodeResponse>, SmsVerificationError> {
    let response = state.sms_verification_service.verify_code(request).await?;
    Ok(Json(response))
}

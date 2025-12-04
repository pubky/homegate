use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    routing::post,
};
use std::net::SocketAddr;

use crate::{
    app_state::AppState,
    external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi},
    http_server::ip_extraction::extract_client_ip,
    sms_verification::{
        SendCodeRequest, SendCodeResponse, SmsVerificationError, VerifyCodeRequest,
        VerifyCodeResponse,
    },
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
            .route("/send_code", post(sms_send_code_handler))
            .route("/verify_code", post(sms_verify_code_handler)),
    )
}

async fn sms_send_code_handler<T, S>(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState<T, S>>,
    Json(request): Json<SendCodeRequest>,
) -> Result<Json<SendCodeResponse>, SmsVerificationError>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    let ip_address = extract_client_ip(addr, &headers);
    let response = state
        .sms_verification_service
        .send_code(request, ip_address)
        .await?;
    Ok(Json(response))
}

async fn sms_verify_code_handler<T, S>(
    State(state): State<AppState<T, S>>,
    Json(request): Json<VerifyCodeRequest>,
) -> Result<Json<VerifyCodeResponse>, SmsVerificationError>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    let response = state.sms_verification_service.verify_code(request).await?;
    Ok(Json(response))
}

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
    sms_verification::{
        SendCodeRequest, SendCodeResponse, SmsVerificationError, VerifyCodeRequest,
        VerifyCodeResponse,
    },
};

/// Extract client IP address from request, checking proxy headers first
fn extract_client_ip(addr: SocketAddr, headers: &HeaderMap) -> String {
    // Check X-Forwarded-For header first (standard proxy header)
    if let Some(forwarded) = headers.get("x-forwarded-for")
        && let Ok(value) = forwarded.to_str()
    {
        // Take first IP in comma-separated list (original client)
        if let Some(ip) = value.split(',').next() {
            return ip.trim().to_string();
        }
    }

    // Check X-Real-IP header (alternative proxy header)
    if let Some(real_ip) = headers.get("x-real-ip")
        && let Ok(value) = real_ip.to_str()
    {
        return value.trim().to_string();
    }

    // Fallback to direct TCP connection IP
    addr.ip().to_string()
}

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

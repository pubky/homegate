use crate::sms_verification::PhoneNumber;
use crate::sms_verification::prelude_api::PreludeBlockedReason;
use serde_json::json;
use std::net::IpAddr;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub struct WiremockServers {
    pub prelude_server: MockServer,
    pub homeserver_server: MockServer,
}

impl WiremockServers {
    pub async fn start() -> Self {
        Self {
            prelude_server: MockServer::start().await,
            homeserver_server: MockServer::start().await,
        }
    }
}

/// Setup mock for Prelude API create_verification endpoint (POST /v2/verification)
pub fn setup_prelude_create_verification(
    phone_number: &PhoneNumber,
    ip_address: Option<IpAddr>,
    response_status: &str,
    reason: Option<PreludeBlockedReason>,
) -> Mock {
    use wiremock::matchers::body_partial_json;

    let mut body = json!({
        "target": {
            "type": "phone_number",
            "value": phone_number.as_str()
        }
    });

    if let Some(ip) = ip_address {
        body["signals"] = json!({
            "ip": ip.to_string()
        });
    }

    let response_body = if let Some(r) = reason {
        json!({
            "id": "verification-id-123",
            "status": response_status,
            "reason": r
        })
    } else {
        json!({
            "id": "verification-id-123",
            "status": response_status,
            "reason": null
        })
    };

    Mock::given(method("POST"))
        .and(path("/v2/verification"))
        .and(header("Authorization", "Bearer test-key"))
        .and(header("Content-Type", "application/json"))
        .and(body_partial_json(&body))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
}

/// Setup mock for Prelude API check_code endpoint (POST /v2/verification/check)
pub fn setup_prelude_check_code(
    phone_number: &PhoneNumber,
    code: &str,
    response_status: &str,
) -> Mock {
    Mock::given(method("POST"))
        .and(path("/v2/verification/check"))
        .and(header("Authorization", "Bearer test-key"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(json!({
            "target": {
                "type": "phone_number",
                "value": phone_number.as_str()
            },
            "code": code
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "verification-id-123",
            "status": response_status,
            "metadata": null,
            "request_id": null
        })))
}

/// Setup mock for Homeserver Admin API generate_signup_token endpoint (GET /generate_signup_token)
pub fn setup_homeserver_signup_token(token: &str) -> Mock {
    Mock::given(method("GET"))
        .and(path("/generate_signup_token"))
        .and(header("X-Admin-Password", "test-pass"))
        .respond_with(ResponseTemplate::new(200).set_body_string(token))
}

/// Setup mock for Homeserver Admin API generate_signup_token with quota (POST /generate_signup_token)
pub fn setup_homeserver_signup_token_with_quota(token: &str) -> Mock {
    Mock::given(method("POST"))
        .and(path("/generate_signup_token"))
        .and(header("X-Admin-Password", "test-pass"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(token))
}

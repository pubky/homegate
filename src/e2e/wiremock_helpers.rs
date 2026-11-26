use crate::sms_verification::PhoneNumber;
use crate::sms_verification::prelude_api::PreludeBlockedReason;
use serde_json::json;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
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

/// Setup mock for Homeserver Admin API generate_signup_token (GET /generate_signup_token)
/// that takes a while to answer and returns a different token for every request
/// (`signup-token-0`, `signup-token-1`, ...), so concurrent callers overlap and a
/// second token request is visible in the tokens returned.
pub fn setup_homeserver_slow_distinct_signup_tokens() -> Mock {
    let requests = AtomicUsize::new(0);
    Mock::given(method("GET"))
        .and(path("/generate_signup_token"))
        .and(header("X-Admin-Password", "test-pass"))
        .respond_with(move |_: &wiremock::Request| {
            let request_number = requests.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200)
                .set_body_string(format!("signup-token-{request_number}"))
                .set_delay(Duration::from_millis(300))
        })
}

/// Start a mock PhoenixD: invoice creation plus an invoice lookup reporting the
/// given payment state.
pub async fn start_phoenixd_mock(is_paid: bool) -> MockServer {
    const PAYMENT_HASH: &str = "bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289";

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/createinvoice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "amountSat": 1000,
            "paymentHash": PAYMENT_HASH,
            "serialized": "lnbc-test-invoice",
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/payments/incoming/{PAYMENT_HASH}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "paymentHash": PAYMENT_HASH,
            "preimage": "00",
            "externalId": null,
            "description": "Test Invoice",
            "invoice": "lnbc-test-invoice",
            "isPaid": is_paid,
            "receivedSat": if is_paid { 1000 } else { 0 },
            "requestedSat": 1000,
            "fees": 0,
            "completedAt": null,
            "createdAt": 1765463919000u64,
            "isExpired": false,
            "expiresAt": 1765464519000u64,
        })))
        .mount(&server)
        .await;
    server
}

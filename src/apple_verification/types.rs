#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleVerificationRequest {
    pub apple_id_token: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleVerificationResponse {
    pub signup_code: String,
    pub homeserver_pubky: String,
}

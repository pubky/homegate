#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleVerificationRequest {
    pub google_id_token: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleVerificationResponse {
    pub signup_code: String,
    pub homeserver_pubky: String,
}

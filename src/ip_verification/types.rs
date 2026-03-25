#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpVerificationResponse {
    pub signup_code: String,
    pub homeserver_pubky: String,
}

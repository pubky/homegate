use crate::sms_verification::prelude_api::{PreludeSendCodeStatus, PreludeVerifyCodeStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub phone_number: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendCodeResponse {
    // Return the Prelude Status directly back to caller
    pub status: PreludeSendCodeStatus,
    // There are a number of Prelude `reason` values, most of which are not blockers. Return them directly for now.
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyCodeRequest {
    pub phone_number: String,
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VerifyCodeResponse {
    // Return the Prelude Status directly back to caller
    pub status: PreludeVerifyCodeStatus,
    pub signup_code: Option<String>,
    pub homeserver_pubky: Option<String>,
}

use crate::sms_verification::phone_number::PhoneNumber;
use crate::sms_verification::prelude_api::PreludeBlockedReason;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateVerificationRequest {
    pub phone_number: PhoneNumber,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CreateVerificationResponse {
    Success,
    Retry,
    Blocked { reason: PreludeBlockedReason },
}

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub phone_number: PhoneNumber,
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SendCodeResponse {
    Success {
        signup_code: String,
        homeserver_pubky: String,
    },
    Failure,
    ExpiredOrNotFound,
}

use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::phone_number::PhoneNumber;
use crate::sms_verification::prelude_api::PreludeBlockedReason;
use serde::{Deserialize, Serialize};

/// Raw request from JSON
#[derive(Debug, Deserialize)]
pub struct CreateVerificationRequestRaw {
    pub phone_number: String,
}

/// Validated request
#[derive(Debug)]
pub struct CreateVerificationRequest {
    pub phone_number: PhoneNumber,
}

impl TryFrom<CreateVerificationRequestRaw> for CreateVerificationRequest {
    type Error = SmsVerificationError;

    fn try_from(raw: CreateVerificationRequestRaw) -> Result<Self, Self::Error> {
        let phone_number = PhoneNumber::new(&raw.phone_number)
            .map_err(|_| SmsVerificationError::InvalidPhoneNumber(raw.phone_number))?;
        Ok(Self { phone_number })
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CreateVerificationResponse {
    Success,
    Retry,
    Blocked { reason: PreludeBlockedReason },
}

/// Raw request from JSON
#[derive(Debug, Deserialize)]
pub struct SendCodeRequestRaw {
    pub phone_number: String,
    pub code: String,
}

/// Validated request
#[derive(Debug)]
pub struct SendCodeRequest {
    pub phone_number: PhoneNumber,
    pub code: String,
}

impl TryFrom<SendCodeRequestRaw> for SendCodeRequest {
    type Error = SmsVerificationError;

    fn try_from(raw: SendCodeRequestRaw) -> Result<Self, Self::Error> {
        let phone_number = PhoneNumber::new(&raw.phone_number)
            .map_err(|_| SmsVerificationError::InvalidPhoneNumber(raw.phone_number))?;
        Ok(Self {
            phone_number,
            code: raw.code,
        })
    }
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

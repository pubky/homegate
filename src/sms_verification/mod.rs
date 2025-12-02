mod error;
mod sms_verification_service;

#[cfg(test)]
mod tests;

pub use error::SmsVerificationError;
pub use sms_verification_service::{
    SendCodeRequest, SendCodeResponse, SmsVerificationService, VerifyCodeRequest,
    VerifyCodeResponse,
};

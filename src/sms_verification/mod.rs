mod error;
mod sms_verification_service;

#[cfg(test)]
mod e2e_tests;

pub use error::SmsVerificationError;
#[allow(unused_imports)]
pub use sms_verification_service::{DefaultSmsVerificationService, SmsVerificationService};

mod error;
mod prelude_api;
mod sms_verification_provider_api;
mod sms_verification_service;

#[cfg(test)]
mod e2e_tests;
#[cfg(test)]
mod mock_provider_api;

pub use error::SmsVerificationError;
pub use prelude_api::VerificationResponse;
#[allow(unused_imports)]
pub use prelude_api::{CheckCodeResponse, PreludeAPI};
#[allow(unused_imports)]
pub use sms_verification_provider_api::SmsVerificationProviderApi;
#[allow(unused_imports)]
pub use sms_verification_service::{DefaultSmsVerificationService, SmsVerificationService};

#[cfg(test)]
#[allow(unused_imports)]
pub use mock_provider_api::MockSmsVerificationProviderApi;

mod error;
pub mod http;
pub mod prelude_api;
pub mod repository;
pub mod service;
mod types;

#[cfg(test)]
mod tests;

// Public API
pub use error::SmsVerificationError;
pub use http::routes;
pub use repository::SmsVerificationRepository;
pub use service::SmsVerificationService;
pub use types::{SendCodeRequest, SendCodeResponse, VerifyCodeRequest, VerifyCodeResponse};

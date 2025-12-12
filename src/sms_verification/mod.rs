#[cfg(test)]
mod tests;

mod app_state;
mod error;
pub(crate) mod hasher_argon2id;
pub mod http;
pub mod prelude_api;
pub mod repository;
pub mod service;
mod types;

pub use error::SmsVerificationError;
pub use hasher_argon2id::HasherArgon2id;
pub use repository::SmsVerificationRepository;
pub use service::SmsVerificationService;
pub use types::{
    Code, CreateVerificationRequest, PhoneNumber, ValidateCodeRequest, ValidateCodeResponse,
};

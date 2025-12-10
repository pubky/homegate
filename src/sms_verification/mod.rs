#[cfg(test)]
mod tests;

mod app_state;
mod error;
pub(crate) mod hasher_argon2id;
pub mod http;
mod phone_number;
pub mod prelude_api;
pub mod repository;
pub mod service;
mod types;

pub use error::SmsVerificationError;
pub use hasher_argon2id::{HasherArgon2id, HasherArgon2idError};
pub use phone_number::PhoneNumber;
pub use repository::SmsVerificationRepository;
pub use service::SmsVerificationService;
pub use types::{
    CreateVerificationRequest, CreateVerificationResponse, SendCodeRequest, SendCodeResponse,
};

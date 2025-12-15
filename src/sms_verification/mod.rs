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

pub use hasher_argon2id::HasherArgon2id;
pub(crate) use repository::SmsVerificationRepository;
pub(crate) use types::{Code, PhoneNumber};

#[cfg(test)]
pub(crate) use types::{CreateVerificationRequest, ValidateCodeRequest, ValidateCodeResponse};

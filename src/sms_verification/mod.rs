#[cfg(test)]
mod tests;

mod app_state;
mod error;
pub mod http;
pub mod prelude_api;
pub mod repository;
pub mod service;
mod types;

pub(crate) use types::{Code, PhoneNumber};

#[cfg(test)]
pub(crate) use types::{CreateVerificationRequest, ValidateCodeRequest, ValidateCodeResponse};

//! Apple identity verification: issues homeserver signup codes for verified Sign in with Apple ID tokens.
//!
//! The endpoint accepts an Apple ID token, verifies it server-side, and rate-limits
//! invite issuance by a secret-peppered hash of the verified `iss` and `sub` claims.
//! Apple users may hide their email, so the identity is never keyed on email.

mod app_state;
mod apple_id_token_verifier;
mod error;
pub mod http;
mod service;
#[cfg(test)]
mod tests;
mod types;

pub use http::router;

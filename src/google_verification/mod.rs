//! Google identity verification: issues homeserver signup codes for verified Google ID tokens.
//!
//! The endpoint accepts a Google ID token, verifies it server-side, and rate-limits
//! invite issuance by a secret-peppered hash of the verified `iss` and `sub` claims.

mod app_state;
mod error;
mod google_id_token_verifier;
pub mod http;
mod repository;
mod service;
#[cfg(test)]
mod tests;
mod types;

pub use http::router;

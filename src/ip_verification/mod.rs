//! IP-based verification: issues homeserver signup codes rate-limited by client IP.
//!
//! This is a low-friction alternative to SMS/LN verification. A client POSTs to
//! the endpoint and receives a signup code if their IP has not exceeded the
//! configured weekly/annual limits.
//!
//! ## Security considerations
//!
//! IP-based rate limiting is inherently weak. The client IP is extracted from
//! `X-Forwarded-For` / `X-Real-IP` headers or the socket address, all of which
//! can be spoofed or shared:
//!
//! - **Header spoofing:** If the reverse proxy does not strip or overwrite
//!   `X-Forwarded-For`, a client can supply an arbitrary IP and bypass rate
//!   limits entirely. The proxy *must* set this header authoritatively.
//! - **Shared IPs:** Users behind carrier-grade NAT, corporate proxies, or VPNs
//!   share a single IP and will collectively exhaust the rate limit.
//! - **Rotating IPs:** An attacker with access to many IPs (botnets, cloud
//!   instances) can trivially circumvent per-IP limits.
//!
//! Because of these limitations, this endpoint should not be the sole line of
//! defence to a system vulnerable to spam.

mod app_state;
mod error;
pub mod http;
mod repository;
mod service;
#[cfg(test)]
mod tests;
mod types;

pub use http::router;

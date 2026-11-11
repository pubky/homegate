pub(crate) mod hasher_argon2id;
mod homeserver_admin_api;
pub(crate) mod jwks_cache;
pub(crate) mod metrics;
pub(crate) mod rate_limited_signup_issuer;

use std::time::Duration;

/// Time limit for each call to an external service: the homeserver admin
/// API, Prelude, the Google and Apple JWKS endpoints and the PhoenixD REST API. It is well
/// below the 30 s request timeout, so a stuck call fails as a normal error
/// and its transaction rolls back cleanly, instead of the request being cut
/// at an arbitrary point.
pub const DEPENDENCY_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub use hasher_argon2id::{HasherArgon2id, HasherBusy};
pub use homeserver_admin_api::HomeserverAdminAPI;
pub use jwks_cache::{JwksCache, jwks_url_from_env_or_default};
pub use metrics::{Dependency, FailureKind, Metrics, RejectionReason, SignupType};
pub use rate_limited_signup_issuer::{
    LimitEnforcement, RateLimitedSignupIssuer, SignupIssuanceError, VerificationTable,
};

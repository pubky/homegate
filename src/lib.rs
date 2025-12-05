pub mod infrastructure;
pub mod shared;
pub mod sms_verification;

#[cfg(test)]
mod e2e;

pub use infrastructure::{
    EnvConfig,
    database::SqlDb,
    http::{AppState, HttpServer},
};

pub use shared::{HomeserverAdminApi, HomeserverAdminApiError, MockHomeserverAdminApi};

pub use sms_verification::{
    SendCodeResponse, SmsVerificationError, SmsVerificationService, VerifyCodeResponse,
};

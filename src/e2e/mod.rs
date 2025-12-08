mod http;

#[cfg(test)]
mod sms_verification_service;

#[cfg(test)]
mod wiremock_helpers;

use sqlx::PgPool;
#[cfg(test)]
pub use wiremock_helpers::*;

use crate::{
    HomeserverAdminAPI, SmsVerificationService, SqlDb,
    sms_verification::{SmsVerificationRepository, prelude_api::PreludeAPI},
};

/// Helper to create service with wiremock for direct service layer testing
async fn create_service_with_mocked_apis(
    pool: PgPool,
    servers: &WiremockServers,
) -> SmsVerificationService {
    let config = servers.create_config();
    let db = SqlDb::test(pool.clone()).await;

    let prelude_api = PreludeAPI::from_config(&config);
    let homeserver_admin_api = HomeserverAdminAPI::from_config(&config);

    let repository = SmsVerificationRepository::new(db);
    SmsVerificationService::new(repository, prelude_api, homeserver_admin_api, 10)
}

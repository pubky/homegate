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
    sms_verification::{PhoneHasher, SmsVerificationRepository, prelude_api::PreludeAPI},
};

/// Helper to create service with wiremock for direct service layer testing
async fn create_service_with_mocked_apis(
    pool: PgPool,
    servers: &WiremockServers,
) -> SmsVerificationService {
    use crate::EnvConfig;

    let config = EnvConfig::for_test(
        servers.prelude_server.uri().parse().unwrap(),
        servers.homeserver_server.uri().parse().unwrap(),
    );
    let db = SqlDb::test(pool.clone()).await;

    let prelude_api = PreludeAPI::new(&config.prelude_api_url, &config.prelude_api_key);
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &config.homeserver_admin_api_url,
        &config.homeserver_admin_password,
        &config.homeserver_pubky,
    );

    let phone_hasher = PhoneHasher::new(config.phone_number_pepper.clone());
    let repository = SmsVerificationRepository::new(db, phone_hasher);
    SmsVerificationService::new(repository, prelude_api, homeserver_admin_api, 10)
}

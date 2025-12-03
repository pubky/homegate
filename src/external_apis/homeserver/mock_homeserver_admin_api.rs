use async_trait::async_trait;
use uuid::Uuid;

use crate::external_apis::homeserver::homeserver_admin_api::{
    HomeserverAdminApiError, HomeserverAdminApiTrait,
};

/// Mock signup token provider for testing
///
/// Always returns a new UUID as the signup token
#[derive(Clone, Default)]
pub struct MockHomeserverAdminApi;

impl MockHomeserverAdminApi {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl HomeserverAdminApiTrait for MockHomeserverAdminApi {
    async fn generate_signup_token(&self) -> Result<String, HomeserverAdminApiError> {
        Ok(Uuid::new_v4().to_string())
    }
}

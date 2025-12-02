mod homeserver_admin_api;

pub use homeserver_admin_api::{
    HomeserverAdminApi, HomeserverAdminApiError, HomeserverAdminApiTrait,
};

#[cfg(test)]
pub mod mock_homeserver_admin_api;

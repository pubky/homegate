use crate::{
    infrastructure::{config::EnvConfig, sql::SqlDb},
    invite::service::InviteService,
    shared::HomeserverAdminAPI,
};

#[derive(Clone, Debug)]
pub struct AppState {
    pub db: SqlDb,
    pub invite: InviteService,
}

impl AppState {
    pub fn new(config: &EnvConfig, db: SqlDb) -> Self {
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );
        let invite = InviteService::new(
            homeserver_admin_api,
            config.max_invite_friend_per_week,
            config.max_invite_friend_per_year,
        );
        Self { db, invite }
    }
}

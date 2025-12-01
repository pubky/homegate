use crate::persistence::db::Db;

#[derive(Clone, Debug)]
pub(crate) struct AppState {
    #[allow(dead_code)]
    pub(crate) db: Db,
}

use crate::persistence::sql::SqlDb;

#[derive(Clone, Debug)]
pub(crate) struct AppState {
    pub(crate) sql_db: SqlDb,
}

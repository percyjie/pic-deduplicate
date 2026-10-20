mod migrations;
pub mod queries;

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;

pub fn open_database(path: &Path) -> AppResult<Connection> {
    let mut conn = Connection::open(path)?;
    migrations::initialize(&mut conn)?;
    Ok(conn)
}

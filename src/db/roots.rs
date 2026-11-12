//! Typed queries on the `roots` table.

use std::path::PathBuf;

use rusqlite::Connection;

use super::DbError;
use crate::model::Root;

/// Every root, ordered by name.
pub fn all(conn: &Connection) -> Result<Vec<Root>, DbError> {
    let mut stmt = conn.prepare("SELECT id, name, path, is_default FROM roots ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(Root {
            id: row.get(0)?,
            name: row.get(1)?,
            path: PathBuf::from(row.get::<_, String>(2)?),
            is_default: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

//! Typed queries on the `roots` table.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{DbError, path_text, unique_violation};
use crate::model::Root;

const COLUMNS: &str = "id, name, path, is_default";

fn from_row(row: &Row) -> rusqlite::Result<Root> {
    Ok(Root {
        id: row.get(0)?,
        name: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        is_default: row.get(3)?,
    })
}

fn one(
    conn: &Connection,
    filter: &str,
    param: impl rusqlite::ToSql,
) -> Result<Option<Root>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM roots WHERE {filter}");
    Ok(conn.query_row(&sql, [param], from_row).optional()?)
}

/// Every root, ordered by name.
pub fn all(conn: &Connection) -> Result<Vec<Root>, DbError> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM roots ORDER BY name"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every root with the number of repositories under it, ordered by name.
pub fn with_repo_counts(conn: &Connection) -> Result<Vec<(Root, u32)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT ro.id, ro.name, ro.path, ro.is_default, COUNT(r.id)
         FROM roots ro LEFT JOIN repos r ON r.root_id = ro.id
         GROUP BY ro.id ORDER BY ro.name",
    )?;
    let rows = stmt.query_map([], |row| Ok((from_row(row)?, row.get(4)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The root with this name, compared case-insensitively.
pub fn by_name(conn: &Connection, name: &str) -> Result<Option<Root>, DbError> {
    one(conn, "name = ?1", name)
}

pub fn by_id(conn: &Connection, id: i64) -> Result<Option<Root>, DbError> {
    one(conn, "id = ?1", id)
}

/// The default root, if one is set.
pub fn default(conn: &Connection) -> Result<Option<Root>, DbError> {
    one(conn, "is_default = ?1", 1)
}

/// Register a root. `path` should already be absolute. The first root
/// becomes the default even when `make_default` is false.
pub fn add(
    conn: &Connection,
    name: &str,
    path: &Path,
    make_default: bool,
) -> Result<Root, DbError> {
    let text = path_text(path)?;
    let tx = conn.unchecked_transaction()?;
    let existing: u32 = tx.query_row("SELECT COUNT(*) FROM roots", [], |row| row.get(0))?;
    let is_default = make_default || existing == 0;
    if is_default {
        tx.execute("UPDATE roots SET is_default = 0 WHERE is_default = 1", [])?;
    }
    tx.execute(
        "INSERT INTO roots (name, path, is_default) VALUES (?1, ?2, ?3)",
        params![name, text, is_default],
    )
    .map_err(|err| match unique_violation(&err) {
        Some(msg) if msg.contains("roots.name") => {
            DbError::Conflict(format!("a root named '{name}' already exists"))
        }
        Some(_) => DbError::Conflict(format!("a root at {text} already exists")),
        None => err.into(),
    })?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(Root {
        id,
        name: name.to_owned(),
        path: path.to_path_buf(),
        is_default,
    })
}

/// Make this root the default and clear the flag on every other root.
pub fn set_default(conn: &Connection, id: i64) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE roots SET is_default = 0 WHERE is_default = 1", [])?;
    tx.execute("UPDATE roots SET is_default = 1 WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Point a root at a new location. Every repository under it follows.
pub fn set_path(conn: &Connection, id: i64, path: &Path) -> Result<(), DbError> {
    let text = path_text(path)?;
    conn.execute(
        "UPDATE roots SET path = ?1 WHERE id = ?2",
        params![text, id],
    )
    .map_err(|err| match unique_violation(&err) {
        Some(_) => DbError::Conflict(format!("a root at {text} already exists")),
        None => err.into(),
    })?;
    Ok(())
}

/// Remove a root that holds no repositories. Removing the default root
/// leaves no default.
pub fn remove(conn: &Connection, root: &Root) -> Result<(), DbError> {
    let repos: u32 = conn.query_row(
        "SELECT COUNT(*) FROM repos WHERE root_id = ?1",
        [root.id],
        |row| row.get(0),
    )?;
    if repos > 0 {
        return Err(DbError::RootNotEmpty {
            name: root.name.clone(),
            repos,
        });
    }
    conn.execute("DELETE FROM roots WHERE id = ?1", [root.id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::fresh;

    #[test]
    fn first_root_becomes_default() {
        let (_dir, conn) = fresh();
        let a = add(&conn, "src", Path::new("/src"), false).unwrap();
        let b = add(&conn, "work", Path::new("/work"), false).unwrap();
        assert!(a.is_default && !b.is_default);
        assert_eq!(default(&conn).unwrap().unwrap().name, "src");

        set_default(&conn, b.id).unwrap();
        assert_eq!(default(&conn).unwrap().unwrap().name, "work");
        assert!(!by_id(&conn, a.id).unwrap().unwrap().is_default);
    }

    #[test]
    fn names_are_case_insensitive_and_unique() {
        let (_dir, conn) = fresh();
        add(&conn, "src", Path::new("/src"), false).unwrap();
        assert_eq!(by_name(&conn, "SRC").unwrap().unwrap().name, "src");
        let err = add(&conn, "Src", Path::new("/other"), false).unwrap_err();
        assert_eq!(err.to_string(), "a root named 'Src' already exists");
        let err = add(&conn, "other", Path::new("/src"), false).unwrap_err();
        assert_eq!(err.to_string(), "a root at /src already exists");
    }

    #[test]
    fn move_and_remove() {
        let (_dir, conn) = fresh();
        let root = add(&conn, "src", Path::new("/src"), false).unwrap();
        set_path(&conn, root.id, Path::new("/mnt/src")).unwrap();
        assert_eq!(
            by_id(&conn, root.id).unwrap().unwrap().path,
            Path::new("/mnt/src")
        );
        assert_eq!(with_repo_counts(&conn).unwrap()[0].1, 0);
        remove(&conn, &root).unwrap();
        assert!(all(&conn).unwrap().is_empty());
    }
}

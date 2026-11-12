//! Typed queries on the `hooks` table.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;
use crate::model::{Hook, Platform, RunWhen};

const COLUMNS: &str = "h.id, h.repo_id, h.position, h.platform, h.run_when, h.command, h.enabled";

fn from_row(row: &Row) -> rusqlite::Result<Hook> {
    Ok(Hook {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        position: row.get(2)?,
        platform: row.get(3)?,
        run_when: row.get(4)?,
        command: row.get(5)?,
        enabled: row.get(6)?,
    })
}

/// Attach a hook to a repository, after its existing hooks.
pub fn add(
    conn: &Connection,
    repo_id: i64,
    command: &str,
    platform: Platform,
    run_when: RunWhen,
) -> Result<Hook, DbError> {
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM hooks WHERE repo_id = ?1",
        [repo_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO hooks (repo_id, position, platform, run_when, command) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![repo_id, position, platform, run_when, command],
    )?;
    Ok(Hook {
        id: conn.last_insert_rowid(),
        repo_id,
        position,
        platform,
        run_when,
        command: command.to_owned(),
        enabled: true,
    })
}

/// A repository's hooks in the order they run.
pub fn for_repo(conn: &Connection, repo_id: i64) -> Result<Vec<Hook>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM hooks h WHERE h.repo_id = ?1 ORDER BY h.position, h.id"
    ))?;
    let rows = stmt.query_map([repo_id], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every hook with its repository's name, ordered by repository then
/// position.
pub fn all(conn: &Connection) -> Result<Vec<(String, Hook)>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS}, r.name FROM hooks h JOIN repos r ON r.id = h.repo_id
         ORDER BY r.name, h.position, h.id"
    ))?;
    let rows = stmt.query_map([], |row| Ok((row.get(7)?, from_row(row)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn by_id(conn: &Connection, id: i64) -> Result<Option<Hook>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM hooks h WHERE h.id = ?1");
    Ok(conn.query_row(&sql, [id], from_row).optional()?)
}

/// Delete a hook. Returns whether it existed.
pub fn remove(conn: &Connection, id: i64) -> Result<bool, DbError> {
    Ok(conn.execute("DELETE FROM hooks WHERE id = ?1", [id])? > 0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::db::repos::tests::add_repo;
    use crate::db::tests::fresh;
    use crate::db::{repos, roots};

    #[test]
    fn hooks_keep_order_and_follow_their_repo() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let v = add_repo(&conn, root.id, "v", &[]);
        let zed = add_repo(&conn, root.id, "zed", &[]);
        let make = add(&conn, v.id, "make", Platform::Unix, RunWhen::Changed).unwrap();
        let bat = add(&conn, v.id, "make.bat", Platform::Windows, RunWhen::Changed).unwrap();
        add(&conn, zed.id, "cargo build", Platform::Any, RunWhen::Always).unwrap();

        assert_eq!((make.position, bat.position), (0, 1));
        assert_eq!(for_repo(&conn, v.id).unwrap(), [make.clone(), bat.clone()]);
        assert_eq!(by_id(&conn, bat.id).unwrap().unwrap(), bat);

        let listed: Vec<_> = all(&conn)
            .unwrap()
            .into_iter()
            .map(|(n, h)| (n, h.command))
            .collect();
        assert_eq!(
            listed,
            [
                ("v".into(), "make".into()),
                ("v".into(), "make.bat".into()),
                ("zed".into(), "cargo build".into())
            ]
        );

        assert!(remove(&conn, make.id).unwrap());
        assert!(!remove(&conn, make.id).unwrap());
        repos::remove(&conn, v.id).unwrap();
        assert!(by_id(&conn, bat.id).unwrap().is_none());
    }
}

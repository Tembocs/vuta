//! Opening and migrating the database. Only this module runs SQL.
//!
//! The submodules are typed queries, one per table group. Lookups by name
//! return `Option`; turning a missing name into a user-facing error is the
//! caller's job.

pub mod hooks;
pub mod repos;
pub mod roots;
pub mod runs;
pub mod settings;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, ErrorCode};

use crate::model::{
    FailReason, GlyphMode, HookOutcome, OutcomeKind, Platform, RunWhen, SkipKind, VcsKind,
};

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("cannot create directory {}: {source}", path.display())]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot open database {}: {source}", path.display())]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error(
        "the database is at schema version {found} but this vuta knows only {known}; upgrade vuta"
    )]
    SchemaTooNew { found: u32, known: u32 },
    /// A uniqueness rule was broken, for example a duplicate name.
    #[error("{0}")]
    Conflict(String),
    #[error("root '{name}' still holds {repos} repositories; remove or move them first")]
    RootNotEmpty { name: String, repos: u32 },
    #[error("path {} is not valid UTF-8", .0.display())]
    NonUtf8Path(PathBuf),
    #[error("unknown setting '{0}'; see: vuta config list")]
    UnknownSetting(String),
    #[error("invalid value '{value}' for {key}: {reason}")]
    InvalidSetting {
        key: String,
        value: String,
        reason: String,
    },
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// The schema version this binary migrates to.
pub fn known_version() -> u32 {
    MIGRATIONS.len() as u32
}

/// Open the database at `path`, creating it and its directory if needed,
/// and apply any pending migrations.
pub fn open(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|source| DbError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut conn = Connection::open(path).map_err(|source| DbError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Open an existing database for reading only: never create, migrate or
/// write it. Shell completion uses this on every TAB. A database at a newer
/// schema version than this binary knows is refused rather than guessed at;
/// one never migrated has no tables, so queries on it fail.
pub fn open_read_only(path: &Path) -> Result<Connection, DbError> {
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|source| DbError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    conn.busy_timeout(Duration::from_millis(200))?;
    let (found, known) = (schema_version(&conn)?, known_version());
    if found > known {
        return Err(DbError::SchemaTooNew { found, known });
    }
    Ok(conn)
}

/// The schema version recorded in the database.
pub fn schema_version(conn: &Connection) -> Result<u32, DbError> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Run SQLite's integrity check. `Ok(None)` means the database is sound;
/// otherwise the first problem SQLite reported.
pub fn integrity_problem(conn: &Connection) -> Result<Option<String>, DbError> {
    let result: String = conn.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    Ok((result != "ok").then_some(result))
}

fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let known = known_version();
    let found = schema_version(conn)?;
    if found > known {
        return Err(DbError::SchemaTooNew { found, known });
    }
    for version in found..known {
        let tx = conn.transaction()?;
        tx.execute_batch(MIGRATIONS[version as usize])?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Run `f` as one unit: everything it writes is kept, or nothing is.
///
/// Uses a SAVEPOINT, which starts a transaction when none is open and
/// nests inside one otherwise. So functions that use `atomic` compose: an
/// outer `atomic` around several of them makes the whole sequence atomic,
/// as `vuta import` does.
pub fn atomic<T, E: From<DbError>>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    conn.execute_batch("SAVEPOINT vuta_atomic")
        .map_err(DbError::from)?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE vuta_atomic")
                .map_err(DbError::from)?;
            Ok(value)
        }
        Err(err) => {
            // Undo this unit's writes, then close it. If the rollback itself
            // fails the connection is unusable anyway; report the cause.
            let _ = conn.execute_batch("ROLLBACK TO vuta_atomic; RELEASE vuta_atomic");
            Err(err)
        }
    }
}

/// The message of a UNIQUE constraint failure, such as
/// `UNIQUE constraint failed: repos.name`, or `None` for any other error.
fn unique_violation(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::SqliteFailure(e, msg)
            if e.code == ErrorCode::ConstraintViolation
                && e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Some(msg.as_deref().unwrap_or(""))
        }
        _ => None,
    }
}

/// A path as stored text. Non-UTF-8 paths are refused rather than mangled.
fn path_text(path: &Path) -> Result<&str, DbError> {
    path.to_str()
        .ok_or_else(|| DbError::NonUtf8Path(path.to_path_buf()))
}

/// Store and load the model's string enums as their `as_str` text.
macro_rules! sql_text_enum {
    ($($ty:ty),+) => {$(
        impl ToSql for $ty {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.as_str()))
            }
        }

        impl FromSql for $ty {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                value.as_str()?.parse().map_err(|e| FromSqlError::Other(Box::new(e)))
            }
        }
    )+};
}

sql_text_enum!(
    VcsKind,
    OutcomeKind,
    SkipKind,
    FailReason,
    HookOutcome,
    Platform,
    RunWhen,
    GlyphMode
);

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh, migrated database in a temporary directory.
    pub(crate) fn fresh() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("vuta.db")).unwrap();
        (dir, conn)
    }

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn migrates_a_new_database() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("nested").join("vuta.db")).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), known_version());
        assert_eq!(
            tables(&conn),
            [
                "hooks",
                "repo_tags",
                "repos",
                "roots",
                "run_results",
                "runs",
                "settings"
            ]
        );
        assert_eq!(integrity_problem(&conn).unwrap(), None);
    }

    #[test]
    fn reopening_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vuta.db");
        let before = tables(&open(&path).unwrap());
        let conn = open(&path).unwrap();
        assert_eq!(tables(&conn), before);
        assert_eq!(schema_version(&conn).unwrap(), known_version());
    }

    fn count_roots(conn: &Connection) -> u32 {
        conn.query_row("SELECT COUNT(*) FROM roots", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn atomic_units_nest_and_roll_back_together() {
        let (_dir, conn) = fresh();
        let insert = |conn: &Connection, name: &str| -> Result<(), DbError> {
            conn.execute("INSERT INTO roots (name, path) VALUES (?1, ?1)", [name])?;
            Ok(())
        };
        // An inner unit that succeeds is undone when the outer one fails.
        let result: Result<(), DbError> = atomic(&conn, |conn| {
            atomic(conn, |conn| insert(conn, "a"))?;
            insert(conn, "a") // duplicate name: fails
        });
        assert!(result.is_err());
        assert_eq!(count_roots(&conn), 0);

        // An inner unit that fails is undone on its own.
        atomic(&conn, |conn| {
            insert(conn, "b")?;
            assert!(atomic(conn, |conn| insert(conn, "b")).is_err());
            insert(conn, "c")
        })
        .unwrap();
        assert_eq!(count_roots(&conn), 2);
    }

    #[test]
    fn refuses_a_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vuta.db");
        open(&path)
            .unwrap()
            .pragma_update(None, "user_version", known_version() + 1)
            .unwrap();
        let err = open(&path).unwrap_err();
        assert!(matches!(err, DbError::SchemaTooNew { .. }), "{err}");
    }
}

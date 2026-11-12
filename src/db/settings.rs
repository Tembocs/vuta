//! Typed queries on the `settings` table.
//!
//! Only the keys in `KNOWN` can be read or written. A key with no row has
//! its default value.

use rusqlite::{Connection, OptionalExtension, params};

use super::DbError;
use crate::model::GlyphMode;

pub const SYNC_JOBS: &str = "sync.jobs";
pub const HISTORY_KEEP_RUNS: &str = "history.keep_runs";
pub const UI_GLYPHS: &str = "ui.glyphs";

/// A setting Vuta understands.
pub struct Known {
    pub key: &'static str,
    pub default: &'static str,
    pub help: &'static str,
    validate: fn(&str) -> Result<(), String>,
}

pub const KNOWN: &[Known] = &[
    Known {
        key: SYNC_JOBS,
        default: "4",
        help: "How many repositories sync at once",
        validate: |v| positive(v, 64),
    },
    Known {
        key: HISTORY_KEEP_RUNS,
        default: "200",
        help: "How many past sync runs to keep",
        validate: |v| positive(v, 1_000_000),
    },
    Known {
        key: UI_GLYPHS,
        default: "auto",
        help: "Status symbols: auto, unicode or ascii",
        validate: |v| v.parse::<GlyphMode>().map(drop).map_err(|e| e.to_string()),
    },
];

fn positive(value: &str, max: u32) -> Result<(), String> {
    match value.parse::<u32>() {
        Ok(n) if (1..=max).contains(&n) => Ok(()),
        _ => Err(format!("expected a whole number from 1 to {max}")),
    }
}

fn known(key: &str) -> Result<&'static Known, DbError> {
    KNOWN
        .iter()
        .find(|k| k.key == key)
        .ok_or_else(|| DbError::UnknownSetting(key.to_owned()))
}

/// A setting's current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub key: &'static str,
    pub value: String,
    pub is_default: bool,
}

/// The value of `key`, or its default when unset.
pub fn get(conn: &Connection, key: &str) -> Result<Setting, DbError> {
    let known = known(key)?;
    let stored: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(match stored {
        Some(value) => Setting {
            key: known.key,
            value,
            is_default: false,
        },
        None => Setting {
            key: known.key,
            value: known.default.to_owned(),
            is_default: true,
        },
    })
}

/// Validate and store a value.
pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    let known = known(key)?;
    (known.validate)(value).map_err(|reason| DbError::InvalidSetting {
        key: key.to_owned(),
        value: value.to_owned(),
        reason,
    })?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Return a setting to its default. Returns whether a value was stored.
pub fn unset(conn: &Connection, key: &str) -> Result<bool, DbError> {
    known(key)?;
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", [key])? > 0)
}

/// Every known setting with its current value, in `KNOWN` order.
pub fn list(conn: &Connection) -> Result<Vec<Setting>, DbError> {
    KNOWN.iter().map(|k| get(conn, k.key)).collect()
}

fn parsed<T: std::str::FromStr>(conn: &Connection, key: &str) -> Result<T, DbError> {
    let setting = get(conn, key)?;
    setting.value.parse().map_err(|_| DbError::InvalidSetting {
        key: key.to_owned(),
        value: setting.value.clone(),
        reason: "fix it with: vuta config set".to_owned(),
    })
}

pub fn sync_jobs(conn: &Connection) -> Result<u32, DbError> {
    parsed(conn, SYNC_JOBS)
}

pub fn history_keep_runs(conn: &Connection) -> Result<u32, DbError> {
    parsed(conn, HISTORY_KEEP_RUNS)
}

pub fn glyph_mode(conn: &Connection) -> Result<GlyphMode, DbError> {
    parsed(conn, UI_GLYPHS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::fresh;

    #[test]
    fn defaults_set_and_unset() {
        let (_dir, conn) = fresh();
        assert_eq!(sync_jobs(&conn).unwrap(), 4);
        assert_eq!(history_keep_runs(&conn).unwrap(), 200);
        assert_eq!(glyph_mode(&conn).unwrap(), GlyphMode::Auto);
        assert!(list(&conn).unwrap().iter().all(|s| s.is_default));

        set(&conn, SYNC_JOBS, "8").unwrap();
        set(&conn, UI_GLYPHS, "ascii").unwrap();
        assert_eq!(sync_jobs(&conn).unwrap(), 8);
        assert_eq!(glyph_mode(&conn).unwrap(), GlyphMode::Ascii);
        assert!(!get(&conn, SYNC_JOBS).unwrap().is_default);

        assert!(unset(&conn, SYNC_JOBS).unwrap());
        assert!(!unset(&conn, SYNC_JOBS).unwrap());
        assert_eq!(sync_jobs(&conn).unwrap(), 4);
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let (_dir, conn) = fresh();
        assert!(matches!(
            get(&conn, "nope"),
            Err(DbError::UnknownSetting(_))
        ));
        assert!(matches!(
            set(&conn, "nope", "1"),
            Err(DbError::UnknownSetting(_))
        ));
        for (key, value) in [(SYNC_JOBS, "0"), (SYNC_JOBS, "x"), (UI_GLYPHS, "emoji")] {
            let err = set(&conn, key, value).unwrap_err();
            assert!(matches!(err, DbError::InvalidSetting { .. }), "{err}");
        }
    }
}

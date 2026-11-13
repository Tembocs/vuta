//! Typed queries on the `repos` and `repo_tags` tables.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use super::{DbError, atomic, unique_violation};
use crate::model::{LastSync, Repo, RepoEntry, VcsKind};

/// A repository to register.
#[derive(Debug, Clone)]
pub struct NewRepo<'a> {
    pub name: &'a str,
    pub root_id: i64,
    /// Already normalised with `paths::normalize_rel_path`.
    pub rel_path: &'a str,
    pub vcs: VcsKind,
    pub url: &'a str,
    pub branch: Option<&'a str>,
    pub clone_filter: Option<&'a str>,
    pub clone_depth: Option<u32>,
    pub tags: &'a [String],
}

/// Which repositories a listing or sync covers. Each non-empty filter must
/// match; within one filter any value matches. Names and tags compare
/// case-insensitively.
#[derive(Debug, Clone, Default)]
pub struct Selection<'a> {
    pub names: &'a [String],
    pub tags: &'a [String],
    pub root_id: Option<i64>,
    pub include_disabled: bool,
}

/// Changes to a repository. `None` leaves a field as it is; for nullable
/// fields, `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub name: Option<String>,
    pub root_id: Option<i64>,
    pub rel_path: Option<String>,
    pub url: Option<String>,
    pub branch: Option<Option<String>>,
    pub clone_filter: Option<Option<String>>,
    pub clone_depth: Option<Option<u32>>,
    pub enabled: Option<bool>,
}

const COLUMNS: &str = "r.id, r.name, r.root_id, r.rel_path, r.vcs, r.url, r.branch, \
                       r.clone_filter, r.clone_depth, r.enabled";

fn from_row(row: &Row) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get(0)?,
        name: row.get(1)?,
        root_id: row.get(2)?,
        rel_path: row.get(3)?,
        vcs: row.get(4)?,
        url: row.get(5)?,
        branch: row.get(6)?,
        clone_filter: row.get(7)?,
        clone_depth: row.get(8)?,
        enabled: row.get(9)?,
    })
}

fn conflict(err: rusqlite::Error, name: &str, rel_path: &str) -> DbError {
    match unique_violation(&err) {
        Some(msg) if msg.contains("repos.name") => {
            DbError::Conflict(format!("a repository named '{name}' already exists"))
        }
        Some(_) => DbError::Conflict(format!(
            "another repository is already at {rel_path} in that root"
        )),
        None => err.into(),
    }
}

/// Register a repository with its tags.
pub fn insert(conn: &Connection, new: &NewRepo) -> Result<Repo, DbError> {
    let id = atomic::<_, DbError>(conn, |tx| {
        tx.execute(
        "INSERT INTO repos (name, root_id, rel_path, vcs, url, branch, clone_filter, clone_depth)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            new.name,
            new.root_id,
            new.rel_path,
            new.vcs,
            new.url,
            new.branch,
            new.clone_filter,
            new.clone_depth
        ],
    )
    .map_err(|err| conflict(err, new.name, new.rel_path))?;
        let id = tx.last_insert_rowid();
        add_tags(tx, id, new.tags)?;
        Ok(id)
    })?;
    Ok(Repo {
        id,
        name: new.name.to_owned(),
        root_id: new.root_id,
        rel_path: new.rel_path.to_owned(),
        vcs: new.vcs,
        url: new.url.to_owned(),
        branch: new.branch.map(str::to_owned),
        clone_filter: new.clone_filter.map(str::to_owned),
        clone_depth: new.clone_depth,
        enabled: true,
    })
}

/// The repository with this name, compared case-insensitively.
pub fn by_name(conn: &Connection, name: &str) -> Result<Option<Repo>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM repos r WHERE r.name = ?1");
    Ok(conn.query_row(&sql, [name], from_row).optional()?)
}

pub fn by_id(conn: &Connection, id: i64) -> Result<Option<Repo>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM repos r WHERE r.id = ?1");
    Ok(conn.query_row(&sql, [id], from_row).optional()?)
}

/// Repositories matching `selection`, ordered by name, with their root
/// name, tags and latest result.
pub fn select(conn: &Connection, selection: &Selection) -> Result<Vec<RepoEntry>, DbError> {
    let mut sql = format!(
        "SELECT {COLUMNS}, r.last_rev, r.last_outcome, r.last_synced_at, ro.name
         FROM repos r JOIN roots ro ON ro.id = r.root_id WHERE 1 = 1"
    );
    let mut values: Vec<Value> = Vec::new();
    if !selection.include_disabled {
        sql.push_str(" AND r.enabled = 1");
    }
    if !selection.names.is_empty() {
        sql.push_str(&format!(
            " AND r.name IN ({})",
            placeholders(selection.names.len())
        ));
        values.extend(selection.names.iter().cloned().map(Value::Text));
    }
    if let Some(root_id) = selection.root_id {
        sql.push_str(" AND r.root_id = ?");
        values.push(Value::Integer(root_id));
    }
    if !selection.tags.is_empty() {
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM repo_tags t WHERE t.repo_id = r.id AND t.tag IN ({}))",
            placeholders(selection.tags.len())
        ));
        values.extend(selection.tags.iter().cloned().map(Value::Text));
    }
    sql.push_str(" ORDER BY r.name");

    let mut tags = all_tags_by_repo(conn)?;
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |row| {
        Ok((
            from_row(row)?,
            LastSync {
                rev: row.get(10)?,
                outcome: row.get(11)?,
                at: row.get(12)?,
            },
            row.get::<_, String>(13)?,
        ))
    })?;
    rows.map(|row| {
        let (repo, last, root_name) = row?;
        Ok(RepoEntry {
            tags: tags.remove(&repo.id).unwrap_or_default(),
            repo,
            root_name,
            last,
        })
    })
    .collect()
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

fn all_tags_by_repo(conn: &Connection) -> Result<HashMap<i64, Vec<String>>, DbError> {
    let mut stmt = conn.prepare("SELECT repo_id, tag FROM repo_tags ORDER BY tag")?;
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    for row in rows {
        let (id, tag) = row?;
        map.entry(id).or_default().push(tag);
    }
    Ok(map)
}

/// Apply `changes` to a repository. Nothing happens when `changes` is empty.
pub fn update(conn: &Connection, id: i64, changes: &Changes) -> Result<(), DbError> {
    let mut sets: Vec<&str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let text = |s: &Option<String>| s.clone().map_or(Value::Null, Value::Text);
    if let Some(name) = &changes.name {
        sets.push("name = ?");
        values.push(Value::Text(name.clone()));
    }
    if let Some(root_id) = changes.root_id {
        sets.push("root_id = ?");
        values.push(Value::Integer(root_id));
    }
    if let Some(rel_path) = &changes.rel_path {
        sets.push("rel_path = ?");
        values.push(Value::Text(rel_path.clone()));
    }
    if let Some(url) = &changes.url {
        sets.push("url = ?");
        values.push(Value::Text(url.clone()));
    }
    if let Some(branch) = &changes.branch {
        sets.push("branch = ?");
        values.push(text(branch));
    }
    if let Some(filter) = &changes.clone_filter {
        sets.push("clone_filter = ?");
        values.push(text(filter));
    }
    if let Some(depth) = changes.clone_depth {
        sets.push("clone_depth = ?");
        values.push(depth.map_or(Value::Null, |d| Value::Integer(d.into())));
    }
    if let Some(enabled) = changes.enabled {
        sets.push("enabled = ?");
        values.push(Value::Integer(enabled.into()));
    }
    if sets.is_empty() {
        return Ok(());
    }
    values.push(Value::Integer(id));
    let sql = format!("UPDATE repos SET {} WHERE id = ?", sets.join(", "));
    conn.execute(&sql, params_from_iter(values))
        .map_err(|err| {
            conflict(
                err,
                changes.name.as_deref().unwrap_or("?"),
                changes.rel_path.as_deref().unwrap_or("that path"),
            )
        })?;
    Ok(())
}

/// Forget a repository. Its tags, hooks and results go with it. Returns
/// whether a row was deleted.
pub fn remove(conn: &Connection, id: i64) -> Result<bool, DbError> {
    Ok(conn.execute("DELETE FROM repos WHERE id = ?1", [id])? > 0)
}

/// Every repository's name and `root:rel_path`, by name.
pub fn names_and_locations(conn: &Connection) -> Result<Vec<(String, String)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT repos.name, roots.name || ':' || repos.rel_path
         FROM repos JOIN roots ON roots.id = repos.root_id
         ORDER BY repos.name",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every tag in use, with how many repositories have it, sorted.
pub fn all_tags(conn: &Connection) -> Result<Vec<(String, u32)>, DbError> {
    let mut stmt = conn.prepare("SELECT tag, count(*) FROM repo_tags GROUP BY tag ORDER BY tag")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A repository's tags, sorted.
pub fn tags(conn: &Connection, id: i64) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT tag FROM repo_tags WHERE repo_id = ?1 ORDER BY tag")?;
    let rows = stmt.query_map([id], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Add tags. Tags the repository already has are ignored.
pub fn add_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<(), DbError> {
    let mut stmt =
        conn.prepare("INSERT OR IGNORE INTO repo_tags (repo_id, tag) VALUES (?1, ?2)")?;
    for tag in tags {
        stmt.execute(params![id, tag])?;
    }
    Ok(())
}

/// Remove tags. Tags the repository does not have are ignored.
pub fn remove_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<(), DbError> {
    let mut stmt = conn.prepare("DELETE FROM repo_tags WHERE repo_id = ?1 AND tag = ?2")?;
    for tag in tags {
        stmt.execute(params![id, tag])?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::db::roots;
    use crate::db::tests::fresh;

    pub(crate) fn add_repo(conn: &Connection, root_id: i64, name: &str, tags: &[&str]) -> Repo {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        let rel_path = format!("x/{name}");
        let url = format!("https://example.com/{name}.git");
        insert(
            conn,
            &NewRepo {
                name,
                root_id,
                rel_path: &rel_path,
                vcs: VcsKind::Git,
                url: &url,
                branch: None,
                clone_filter: None,
                clone_depth: None,
                tags: &tags,
            },
        )
        .unwrap()
    }

    fn names(entries: &[RepoEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.repo.name.as_str()).collect()
    }

    #[test]
    fn insert_and_look_up() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let repo = add_repo(&conn, root.id, "zed", &["rust", "editor"]);
        assert_eq!(by_name(&conn, "ZED").unwrap().unwrap(), repo);
        assert_eq!(by_id(&conn, repo.id).unwrap().unwrap(), repo);
        assert_eq!(tags(&conn, repo.id).unwrap(), ["editor", "rust"]);
        assert!(by_name(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn duplicates_are_conflicts() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        add_repo(&conn, root.id, "zed", &[]);
        let err = insert(
            &conn,
            &NewRepo {
                name: "Zed",
                root_id: root.id,
                rel_path: "other",
                vcs: VcsKind::Git,
                url: "u",
                branch: None,
                clone_filter: None,
                clone_depth: None,
                tags: &[],
            },
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "a repository named 'Zed' already exists");
    }

    #[test]
    fn selection_filters_combine() {
        let (_dir, conn) = fresh();
        let src = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let work = roots::add(&conn, "work", Path::new("/work"), false).unwrap();
        add_repo(&conn, src.id, "zed", &["rust"]);
        add_repo(&conn, src.id, "v", &["v"]);
        let edit = add_repo(&conn, work.id, "edit", &["Rust"]);
        update(
            &conn,
            edit.id,
            &Changes {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .unwrap();

        let all = Selection {
            include_disabled: true,
            ..Default::default()
        };
        assert_eq!(names(&select(&conn, &all).unwrap()), ["edit", "v", "zed"]);
        assert_eq!(
            names(&select(&conn, &Selection::default()).unwrap()),
            ["v", "zed"]
        );

        let rust = ["RUST".to_owned()];
        let by_tag = Selection {
            tags: &rust,
            include_disabled: true,
            ..Default::default()
        };
        assert_eq!(names(&select(&conn, &by_tag).unwrap()), ["edit", "zed"]);

        let by_root = Selection {
            root_id: Some(src.id),
            tags: &rust,
            ..Default::default()
        };
        assert_eq!(names(&select(&conn, &by_root).unwrap()), ["zed"]);

        let picked = ["V".to_owned(), "zed".to_owned()];
        let by_name = Selection {
            names: &picked,
            ..Default::default()
        };
        let entries = select(&conn, &by_name).unwrap();
        assert_eq!(names(&entries), ["v", "zed"]);
        assert_eq!(entries[1].tags, ["rust"]);
        assert_eq!(entries[1].root_name, "src");
        assert_eq!(entries[1].last, LastSync::default());
    }

    #[test]
    fn update_tags_and_remove() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let repo = add_repo(&conn, root.id, "modular", &["mojo"]);
        update(
            &conn,
            repo.id,
            &Changes {
                name: Some("mojo".into()),
                branch: Some(Some("main".into())),
                clone_depth: Some(Some(1)),
                ..Default::default()
            },
        )
        .unwrap();
        let repo = by_id(&conn, repo.id).unwrap().unwrap();
        assert_eq!(
            (repo.name.as_str(), repo.branch.as_deref(), repo.clone_depth),
            ("mojo", Some("main"), Some(1))
        );

        update(
            &conn,
            repo.id,
            &Changes {
                branch: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_id(&conn, repo.id).unwrap().unwrap().branch, None);

        add_tags(&conn, repo.id, &["a".into(), "MOJO".into()]).unwrap();
        remove_tags(&conn, repo.id, &["mojo".into(), "missing".into()]).unwrap();
        assert_eq!(tags(&conn, repo.id).unwrap(), ["a"]);

        assert!(remove(&conn, repo.id).unwrap());
        assert!(!remove(&conn, repo.id).unwrap());
        assert!(tags(&conn, repo.id).unwrap().is_empty());
    }

    #[test]
    fn a_root_with_repos_cannot_be_removed() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        add_repo(&conn, root.id, "zed", &[]);
        let err = roots::remove(&conn, &root).unwrap_err();
        assert!(
            matches!(err, DbError::RootNotEmpty { repos: 1, .. }),
            "{err}"
        );
    }
}

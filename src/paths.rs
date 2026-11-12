//! The data directory, path normalisation, and joining a root with a
//! repository's `rel_path`.

use std::path::{Path, PathBuf};

use crate::Error;

/// The platform default database file, in the per-user local data directory.
pub fn default_db_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "vuta")?;
    Some(dirs.data_local_dir().join("vuta.db"))
}

/// Resolve the database path: `--db` (which clap also fills from `VUTA_DB`),
/// then the platform default. The result is absolute.
pub fn resolve_db_path(flag: Option<&Path>) -> Result<PathBuf, Error> {
    let path = match flag {
        Some(path) => path.to_path_buf(),
        None => default_db_path().ok_or(Error::NoDataDir)?,
    };
    std::path::absolute(&path)
        .map_err(|err| anyhow::anyhow!("cannot make {} absolute: {err}", path.display()).into())
}

/// The sync lock file, `vuta.lock` beside the database.
pub fn lock_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name("vuta.lock")
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelPathError {
    #[error("path is empty")]
    Empty,
    #[error("path '{0}' must be relative to its root")]
    NotRelative(String),
    #[error("path '{0}' must not contain '.' or '..' segments")]
    DotSegment(String),
}

/// Turn a user-supplied relative path into the stored form: `/` separators,
/// no empty, `.` or `..` segments, and not absolute.
pub fn normalize_rel_path(input: &str) -> Result<String, RelPathError> {
    let unified = input.replace('\\', "/");
    if unified.starts_with('/') || unified.contains(':') {
        return Err(RelPathError::NotRelative(input.to_owned()));
    }
    let mut segments = Vec::new();
    for segment in unified.split('/') {
        match segment {
            "" => continue,
            "." | ".." => return Err(RelPathError::DotSegment(input.to_owned())),
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return Err(RelPathError::Empty);
    }
    Ok(segments.join("/"))
}

/// The checkout location: `root` joined with a stored `rel_path`, using the
/// platform's separator.
pub fn join_rel_path(root: &Path, rel_path: &str) -> Result<PathBuf, RelPathError> {
    let normalized = normalize_rel_path(rel_path)?;
    let mut path = root.to_path_buf();
    path.extend(normalized.split('/'));
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_trailing_slashes() {
        assert_eq!(normalize_rel_path("rust/zed").unwrap(), "rust/zed");
        assert_eq!(normalize_rel_path("rust\\zed\\").unwrap(), "rust/zed");
        assert_eq!(normalize_rel_path("rust//zed").unwrap(), "rust/zed");
    }

    #[test]
    fn rejects_absolute_and_dot_segments() {
        assert!(matches!(
            normalize_rel_path("/abs"),
            Err(RelPathError::NotRelative(_))
        ));
        assert!(matches!(
            normalize_rel_path("C:\\src"),
            Err(RelPathError::NotRelative(_))
        ));
        assert!(matches!(
            normalize_rel_path("../up"),
            Err(RelPathError::DotSegment(_))
        ));
        assert!(matches!(
            normalize_rel_path("a/./b"),
            Err(RelPathError::DotSegment(_))
        ));
        assert_eq!(normalize_rel_path(""), Err(RelPathError::Empty));
        assert_eq!(
            normalize_rel_path("/"),
            Err(RelPathError::NotRelative("/".into()))
        );
    }

    #[test]
    fn joins_with_platform_separator() {
        let root = Path::new("base");
        let joined = join_rel_path(root, "rust/zed").unwrap();
        assert_eq!(joined, root.join("rust").join("zed"));
    }

    #[test]
    fn lock_sits_beside_the_database() {
        let db = Path::new("data").join("vuta.db");
        assert_eq!(lock_path(&db), Path::new("data").join("vuta.lock"));
    }

    #[test]
    fn resolves_relative_db_flag_to_absolute() {
        let path = resolve_db_path(Some(Path::new("x.db"))).unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("x.db"));
    }
}

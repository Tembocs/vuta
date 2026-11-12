//! Remote URL normalisation and repository name inference.
//!
//! Handles the three shapes Git accepts: URLs (`https://`, `ssh://`,
//! `git://`, `file://`), scp-like `[user@]host:path`, and local paths.

/// A remote reduced to what identifies the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Remote {
    /// Host lowercased, without user or port; path without leading `/`.
    Network { host: String, path: String },
    /// A local path with `/` separators.
    Local { path: String },
}

fn parse(url: &str) -> Remote {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return Remote::Local {
                path: clean_path(rest),
            };
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        return Remote::Network {
            host: host_of(authority),
            path: clean_path(path).trim_start_matches('/').to_owned(),
        };
    }
    // scp-like: a colon before any slash, and not a Windows drive letter.
    if let Some((before, path)) = url.split_once(':') {
        let is_drive = before.len() == 1 && before.chars().all(|c| c.is_ascii_alphabetic());
        if !before.is_empty() && !before.contains(['/', '\\']) && !is_drive {
            return Remote::Network {
                host: host_of(before),
                path: clean_path(path).trim_start_matches('/').to_owned(),
            };
        }
    }
    Remote::Local {
        path: clean_path(url),
    }
}

/// The host part of `[user@]host[:port]`, lowercased.
fn host_of(authority: &str) -> String {
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or(bracketed)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host.to_ascii_lowercase()
}

/// `/` separators, without trailing slashes or a `.git` suffix.
fn clean_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    path.trim_end_matches('/').to_owned()
}

/// The default repository name for a remote: the last path segment without
/// `.git`. `https://github.com/zed-industries/zed.git` gives `zed`.
pub fn infer_name(url: &str) -> Option<String> {
    let path = match parse(url) {
        Remote::Network { path, .. } | Remote::Local { path } => path,
    };
    path.rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_owned)
}

/// Whether two remotes name the same repository: host and path match after
/// lowercasing the host and dropping the scheme, user, port, `.git` suffix
/// and trailing slash.
pub fn same_repo(a: &str, b: &str) -> bool {
    parse(a) == parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_names_from_every_shape() {
        for (url, name) in [
            ("https://github.com/zed-industries/zed.git", "zed"),
            ("https://github.com/vlang/v", "v"),
            ("https://github.com/vlang/v/", "v"),
            ("git@github.com:vlang/v.git", "v"),
            ("ssh://git@github.com:22/modular/modular.git", "modular"),
            ("file:///srv/git/turso.git", "turso"),
            ("/srv/git/edit.git/", "edit"),
            ("C:\\repos\\openvpn.git", "openvpn"),
            ("../relative/surrealdb", "surrealdb"),
        ] {
            assert_eq!(infer_name(url).as_deref(), Some(name), "{url}");
        }
        assert_eq!(infer_name("https://github.com"), None);
        assert_eq!(infer_name(""), None);
    }

    #[test]
    fn matches_equivalent_remotes() {
        for (a, b) in [
            ("git@github.com:vlang/v.git", "https://github.com/vlang/v"),
            (
                "https://GitHub.com/vlang/v/",
                "ssh://git@github.com/vlang/v.git",
            ),
            ("ssh://git@github.com:22/vlang/v", "git@github.com:vlang/v"),
            ("/srv/git/v.git", "file:///srv/git/v"),
            ("C:\\repos\\v.git", "C:/repos/v"),
        ] {
            assert!(same_repo(a, b), "{a} vs {b}");
        }
    }

    #[test]
    fn distinguishes_different_remotes() {
        for (a, b) in [
            ("https://github.com/vlang/v", "https://github.com/vlang/vls"),
            ("https://github.com/vlang/v", "https://gitlab.com/vlang/v"),
            ("https://github.com/vlang/v", "https://github.com/VLang/v"),
            ("/srv/git/v", "https://srv/git/v"),
        ] {
            assert!(!same_repo(a, b), "{a} vs {b}");
        }
    }
}

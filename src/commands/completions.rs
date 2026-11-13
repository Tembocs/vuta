//! Shell completion through clap_complete's dynamic engine.
//!
//! `vuta completions <shell>` prints a short script that registers `vuta`
//! with the shell. On each TAB the shell runs
//! `COMPLETE=<shell> vuta -- <words of the line>`, and `main` hands that to
//! [`handle_request`] before parsing anything. The engine answers from
//! `cli.rs` (subcommands, flags, fixed values, paths) and from the
//! completers below, which read repository, root and tag names, hook ids and
//! setting keys from the registry.
//!
//! A completer must never get in the way: it opens the database that the
//! line names (`--db`, then `VUTA_DB`, then the platform default)
//! read-only and only if the file exists, so it never creates or migrates
//! one, and it returns nothing on any error.
//!
//! The engine is behind clap_complete's `unstable-dynamic` feature, so
//! `Cargo.toml` pins clap_complete to one exact version.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::anyhow;
use clap::CommandFactory;
use clap_complete::CompleteEnv;
pub use clap_complete::Shell;
use clap_complete::engine::CompletionCandidate;
use clap_complete::env::Shells;
use rusqlite::Connection;

use crate::Error;
use crate::cli::Cli;
use crate::db::{self, DbError};
use crate::ui::Ui;

/// The binary name the scripts complete, and the command they call back.
const BIN: &str = "vuta";
/// The environment variable that turns a run into a completion request.
pub const VAR: &str = "COMPLETE";

/// Answer a completion request and exit, if this run is one; otherwise
/// return and let the program run normally. Call it before printing
/// anything.
pub fn handle_request() {
    CompleteEnv::with_factory(Cli::command).var(VAR).complete();
}

pub fn run(ui: &Ui, shell: Shell) -> Result<ExitCode, Error> {
    ui.verbatim(&script(shell))
        .map_err(|err| anyhow!("cannot write the completion script: {err}"))?;
    Ok(ExitCode::SUCCESS)
}

/// The registration script for `shell`. It calls `vuta` through PATH, so
/// it suits a package's completion directory. Packaging calls this too.
pub fn script(shell: Shell) -> String {
    let shells = Shells::builtins();
    let completer = shells
        .completer(&shell.to_string())
        .expect("clap_complete registers every shell it can generate for");
    let mut buf = Vec::new();
    completer
        .write_registration(VAR, BIN, BIN, BIN, &mut buf)
        .expect("writing to memory cannot fail");
    let mut text = String::from_utf8(buf).expect("clap_complete writes UTF-8");
    if shell == Shell::Zsh {
        // Installed in zsh's fpath, the file is autoloaded as the body of
        // `_vuta` on the first TAB. The script only defines and registers
        // its function, so call it then too, or the first TAB offers
        // nothing. Sourced, `_vuta` is not running and this does nothing.
        text.push_str(ZSH_AUTOLOAD);
    }
    text
}

/// Appended to the zsh script; see `script`.
const ZSH_AUTOLOAD: &str = r#"
if [ "$funcstack[1]" = "_vuta" ]; then
    _clap_dynamic_completer_vuta "$@"
fi
"#;

/// Repository names, each described by where its checkout lives.
pub fn repo_names() -> Vec<CompletionCandidate> {
    from_registry(|conn| {
        Ok(db::repos::names_and_locations(conn)?
            .into_iter()
            .map(|(name, location)| CompletionCandidate::new(name).help(Some(location.into())))
            .collect())
    })
}

/// Root names, each described by its path.
pub fn root_names() -> Vec<CompletionCandidate> {
    from_registry(|conn| {
        Ok(db::roots::all(conn)?
            .into_iter()
            .map(|root| {
                let path = root.path.display().to_string();
                CompletionCandidate::new(root.name).help(Some(path.into()))
            })
            .collect())
    })
}

/// Tags in use, each described by how many repositories have it.
pub fn tag_names() -> Vec<CompletionCandidate> {
    from_registry(|conn| {
        Ok(db::repos::all_tags(conn)?
            .into_iter()
            .map(|(tag, n)| {
                let help = super::repositories(n as usize);
                CompletionCandidate::new(tag).help(Some(help.into()))
            })
            .collect())
    })
}

/// The arguments of `vuta tag <name>`: `+tag`, `-tag` or a bare tag,
/// keeping the sign already typed.
pub fn tag_changes(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let (sign, typed) = match current.chars().next() {
        Some(sign @ ('+' | '-')) => (sign.to_string(), &current[1..]),
        _ => (String::new(), current),
    };
    tag_names()
        .into_iter()
        .filter(|c| c.get_value().to_str().is_some_and(|t| t.starts_with(typed)))
        .map(|c| c.add_prefix(sign.as_str()))
        .collect()
}

/// Hook ids, each described by its repository and command.
pub fn hook_ids() -> Vec<CompletionCandidate> {
    from_registry(|conn| {
        Ok(db::hooks::all(conn)?
            .into_iter()
            .map(|(repo, hook)| {
                let help = format!("{repo}: {}", hook.command);
                CompletionCandidate::new(hook.id.to_string()).help(Some(help.into()))
            })
            .collect())
    })
}

/// Setting keys, each described by what it does. They need no database.
pub fn setting_keys() -> Vec<CompletionCandidate> {
    db::settings::KNOWN
        .iter()
        .map(|known| CompletionCandidate::new(known.key).help(Some(known.help.into())))
        .collect()
}

/// Candidates read by `f` from the registry, or none when there is no
/// database or anything goes wrong.
fn from_registry(
    f: impl FnOnce(&Connection) -> Result<Vec<CompletionCandidate>, DbError>,
) -> Vec<CompletionCandidate> {
    db_path()
        .filter(|path| path.is_file())
        .and_then(|path| db::open_read_only(&path).ok())
        .and_then(|conn| f(&conn).ok())
        .unwrap_or_default()
}

/// The database the line being completed would use: its `--db`, then
/// `VUTA_DB`, then the platform default.
fn db_path() -> Option<PathBuf> {
    let args: Vec<OsString> = std::env::args_os().collect();
    // The process runs as `vuta -- <words>`; the words start after `--`.
    let words = args.iter().skip_while(|a| *a != "--").skip(1);
    db_flag(words.map(OsString::as_os_str))
        .or_else(|| {
            std::env::var_os("VUTA_DB")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(crate::paths::default_db_path)
}

/// The value of the last `--db PATH` or `--db=PATH` among `words`, before
/// any `--` that ends the options.
fn db_flag<'a>(words: impl Iterator<Item = &'a OsStr>) -> Option<PathBuf> {
    let mut words = words;
    let mut found = None;
    while let Some(word) = words.next() {
        if word == "--" {
            break;
        } else if word == "--db" {
            if let Some(value) = words.next() {
                found = Some(PathBuf::from(value));
            }
        } else if let Some(value) = word.to_str().and_then(|w| w.strip_prefix("--db=")) {
            found = Some(PathBuf::from(value));
        }
    }
    found.filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn every_shell_gets_a_script_that_calls_back_into_vuta() {
        for shell in Shell::value_variants() {
            let text = script(*shell);
            assert!(text.contains("COMPLETE"), "{shell}: {text}");
            assert!(text.contains("vuta"), "{shell}: {text}");
        }
        // zsh finds the script in its fpath by this first line, and the
        // function the appended lines call is the one the script defines.
        let zsh = script(Shell::Zsh);
        assert!(zsh.starts_with("#compdef vuta\n"));
        assert!(zsh.contains("function _clap_dynamic_completer_vuta()"));
        assert!(zsh.ends_with(ZSH_AUTOLOAD));
    }

    #[test]
    fn the_db_comes_from_the_line() {
        let flag = |words: &[&str]| db_flag(words.iter().map(OsStr::new));
        assert_eq!(flag(&["vuta", "sync", "z"]), None);
        assert_eq!(
            flag(&["vuta", "--db", "/x/a.db", "sync", "z"]),
            Some("/x/a.db".into())
        );
        assert_eq!(
            flag(&["vuta", "sync", "--db=b.db", "z"]),
            Some("b.db".into())
        );
        // The last one wins, as when clap parses the line.
        assert_eq!(
            flag(&["vuta", "--db", "a", "--db", "b", "show", ""]),
            Some("b".into())
        );
        // `--db` still being typed, or after `--`, names nothing.
        assert_eq!(flag(&["vuta", "--db"]), None);
        assert_eq!(flag(&["vuta", "--db="]), None);
        assert_eq!(flag(&["vuta", "tag", "zed", "--", "--db", "x"]), None);
    }
}

//! One module per subcommand: read arguments, call the core, render.
//!
//! The helpers here turn names into rows (with the `Error` variants that
//! map to exit code 3) and render values several commands share.

pub mod add;
pub mod completions;
pub mod config;
pub mod db;
pub mod doctor;
pub mod export;
pub mod hook;
pub mod import;
pub mod list;
pub mod log;
pub mod remove;
pub mod root;
pub mod scan;
pub mod set;
pub mod show;
pub mod status;
pub mod sync;
pub mod tag;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use rusqlite::Connection;

use crate::Error;
use crate::cli::{Cli, Command, DbCommand};
use crate::model::{FailReason, HookOutcome, Outcome, OutcomeKind, Repo, Root, RunResult};
use crate::paths;
use crate::proc::{Captured, Tracer};
use crate::ui::table::Cell;
use crate::ui::theme::{Glyphs, THEME};
use crate::ui::{Styled, Ui, Verbosity, report};

/// What every command gets: the terminal and the resolved database path.
/// The database is opened only by commands that need it.
pub struct Ctx<'a> {
    pub ui: &'a mut Ui,
    pub db_path: PathBuf,
}

impl Ctx<'_> {
    /// Open the database and apply the `ui.glyphs` setting.
    pub fn open_db(&mut self) -> Result<Connection, Error> {
        let conn = crate::db::open(&self.db_path)?;
        // `config set` validates the value, so this only fails on a
        // database error; the platform default is fine then.
        if let Ok(mode) = crate::db::settings::glyph_mode(&conn) {
            self.ui.set_glyph_mode(mode);
        }
        Ok(conn)
    }
}

pub fn run(cli: &Cli, ui: &mut Ui) -> Result<ExitCode, Error> {
    // Completions need no database, not even its path, so a shell's init
    // file can run them anywhere.
    if let Command::Completions { shell } = &cli.command {
        return completions::run(ui, *shell);
    }
    let mut ctx = Ctx {
        ui,
        db_path: paths::resolve_db_path(cli.db.as_deref())?,
    };
    let ctx = &mut ctx;
    match &cli.command {
        Command::Root(cmd) => root::run(ctx, cmd),
        Command::Add(args) => add::run(ctx, args),
        Command::Scan(args) => scan::run(ctx, args),
        Command::Sync(args) => sync::run(ctx, args),
        Command::List(args) => list::run(ctx, args),
        Command::Status(args) => status::run(ctx, args),
        Command::Show(args) => show::run(ctx, args),
        Command::Set(args) => set::run(ctx, args),
        Command::Tag(args) => tag::run(ctx, args),
        Command::Remove(args) => remove::run(ctx, args),
        Command::Hook(cmd) => hook::run(ctx, cmd),
        Command::Log(args) => log::run(ctx, args),
        Command::Config(cmd) => config::run(ctx, cmd),
        Command::Export(args) => export::run(ctx, args),
        Command::Import(args) => import::run(ctx, args),
        Command::Completions { .. } => unreachable!("handled above"),
        Command::Db(DbCommand::Path) => db::path(ctx),
        Command::Doctor => doctor::run(ctx),
    }
}

/// The repository with this name, or `Error::UnknownRepo`.
pub fn repo_by_name(conn: &Connection, name: &str) -> Result<Repo, Error> {
    crate::db::repos::by_name(conn, name)?.ok_or_else(|| Error::UnknownRepo(name.to_owned()))
}

/// The root with this name, or `Error::UnknownRoot`.
pub fn root_by_name(conn: &Connection, name: &str) -> Result<Root, Error> {
    crate::db::roots::by_name(conn, name)?.ok_or_else(|| Error::UnknownRoot(name.to_owned()))
}

/// The root named by `--root`, or the default root.
pub fn root_or_default(conn: &Connection, name: Option<&str>) -> Result<Root, Error> {
    match name {
        Some(name) => root_by_name(conn, name),
        None => crate::db::roots::default(conn)?.ok_or_else(|| {
            anyhow!("no default root; add one with: vuta root add <name> <path>").into()
        }),
    }
}

/// The root a repository lives under.
pub fn root_of(conn: &Connection, repo: &Repo) -> Result<Root, Error> {
    // The foreign key guarantees the row; a miss means a damaged database.
    crate::db::roots::by_id(conn, repo.root_id)?
        .ok_or_else(|| anyhow!("repository '{}' has no root", repo.name).into())
}

/// Where a repository's checkout lives.
pub fn checkout_path(root: &Root, repo: &Repo) -> Result<PathBuf, Error> {
    paths::join_rel_path(&root.path, &repo.rel_path)
        .map_err(|err| anyhow!("repository '{}': {err}", repo.name).into())
}

/// A user-typed path relative to a root, in stored form.
pub fn rel_path(input: &str) -> Result<String, Error> {
    paths::normalize_rel_path(input).map_err(|err| anyhow::Error::from(err).into())
}

/// A user-typed directory made absolute against the current directory.
pub fn absolute(path: &Path) -> Result<PathBuf, Error> {
    std::path::absolute(path)
        .map_err(|err| anyhow!("cannot make {} absolute: {err}", path.display()).into())
}

/// Refuse names and tags that would be hard to type or to tell apart:
/// empty, containing whitespace or control characters, or starting with
/// `+` or `-` (which `vuta tag` and flags give meaning to).
pub fn check_label(kind: &str, label: &str) -> Result<(), Error> {
    let bad = label.is_empty()
        || label.starts_with(['+', '-'])
        || label.chars().any(|c| c.is_whitespace() || c.is_control());
    if bad {
        return Err(anyhow!(
            "invalid {kind} '{label}': use letters, digits and punctuation, \
             without spaces, and do not start with '+' or '-'"
        )
        .into());
    }
    Ok(())
}

/// A remote as stored: a local path that exists is made absolute, so the
/// repository syncs from any directory. Anything else is kept as typed.
pub fn stored_url(url: &str) -> Result<String, Error> {
    let path = Path::new(url);
    if !path.is_absolute() && path.exists() {
        let abs = absolute(path)?;
        return abs
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("path {} is not valid UTF-8", abs.display()).into());
    }
    Ok(url.to_owned())
}

/// Print one JSON value on stdout.
pub fn print_json(ui: &Ui, value: &serde_json::Value) -> Result<(), Error> {
    ui.json(value)
        .map_err(|err| anyhow!("cannot write the JSON output: {err}").into())
}

/// Every root by id, for turning repositories into checkout paths.
pub fn roots_by_id(conn: &Connection) -> Result<std::collections::HashMap<i64, Root>, Error> {
    Ok(crate::db::roots::all(conn)?
        .into_iter()
        .map(|root| (root.id, root))
        .collect())
}

/// A tracer that prints each git command with `--verbose`, or `None`.
pub fn tracer(ui: &Ui) -> Option<Tracer> {
    (ui.verbosity() == Verbosity::Verbose).then(|| {
        let trace_ui = Ui::new(Verbosity::Verbose);
        Arc::new(move |captured: &Captured| trace_ui.trace(captured)) as Tracer
    })
}

/// `1 repository`, `9 repositories`.
fn repositories(n: usize) -> String {
    if n == 1 {
        "1 repository".to_owned()
    } else {
        format!("{n} repositories")
    }
}

/// The current time in Unix seconds.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `root:rel/path`, how a checkout's location is shown in one line.
pub fn location(root: &Root, repo: &Repo) -> String {
    format!("{}:{}", root.name, repo.rel_path)
}

/// The glyph and word of a stored result, in its status colour.
pub fn stored_status(result: &RunResult, glyphs: &Glyphs) -> Cell {
    let (style, glyph, word) = report::status(result.outcome, glyphs);
    Cell::new(format!("{glyph} {word}"), style)
}

/// The words a result line showed for a stored result. Skips store their
/// description as the message; the other outcomes are rebuilt so the text
/// matches the live result line.
pub fn stored_detail(result: &RunResult, glyphs: &Glyphs) -> Styled {
    let rev = || result.new_rev.clone().unwrap_or_default();
    let outcome = match result.outcome {
        OutcomeKind::Cloned => Outcome::Cloned { rev: rev() },
        OutcomeKind::Updated => Outcome::Updated {
            old: result.old_rev.clone().unwrap_or_default(),
            new: rev(),
            commits: result.commits.unwrap_or(0),
        },
        OutcomeKind::UpToDate => Outcome::UpToDate {
            rev: rev(),
            ahead: 0,
        },
        OutcomeKind::Skipped => {
            let mut out = Styled::new();
            out.plain(result.message.as_deref().unwrap_or("skipped"));
            return out;
        }
        OutcomeKind::Failed => Outcome::Failed {
            reason: result
                .reason
                .as_deref()
                .and_then(|r| r.parse().ok())
                .unwrap_or(FailReason::Fetch),
            message: result.message.clone().unwrap_or_default(),
        },
    };
    report::detail(&outcome, glyphs)
}

/// How a stored result's hooks went, for tables. Empty when it had none.
pub fn stored_hooks(result: &RunResult) -> Cell {
    match result.hook_outcome {
        Some(HookOutcome::Ok) => Cell::new("hooks ok", THEME.ok),
        Some(HookOutcome::Failed) => Cell::new("hook failed", THEME.err),
        Some(HookOutcome::NotRun) => Cell::new("hooks not run", THEME.muted),
        None => Cell::plain(""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        for good in ["zed", "rust-tools", "v.2", "Ünïcode"] {
            assert!(check_label("tag", good).is_ok(), "{good}");
        }
        for bad in ["", "+rust", "-rust", "two words", "tab\there"] {
            assert!(check_label("tag", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn remote_urls_keep_their_form_unless_local() {
        assert_eq!(
            stored_url("https://github.com/vlang/v").unwrap(),
            "https://github.com/vlang/v"
        );
        assert_eq!(
            stored_url("git@github.com:vlang/v.git").unwrap(),
            "git@github.com:vlang/v.git"
        );
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_str().unwrap();
        assert_eq!(stored_url(abs).unwrap(), abs);
    }
}

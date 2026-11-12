//! `vuta status [name]...`: the local state of each checkout, without
//! touching the network. Ahead and behind counts compare with the
//! remote-tracking branch as of the last fetch.

use std::path::Path;
use std::process::ExitCode;

use anstyle::Style;
use serde_json::{Value, json};

use super::{Ctx, checkout_path, print_json, repo_by_name, root_by_name, roots_by_id, tracer};
use crate::Error;
use crate::cli::{Format, StatusArgs};
use crate::db;
use crate::db::repos::Selection;
use crate::model::{Repo, count, short_rev};
use crate::ui::json::path_text;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::vcs::{self, Head, Vcs, remote};

/// What kind of checkout the path holds, in the order a sync checks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    NotCloned,
    NotACheckout,
    NoOrigin,
    RemoteMismatch {
        found: String,
    },
    Unborn,
    Detached,
    /// On a branch; the details are in `Inspection`.
    Branch,
    /// Git could not answer.
    Error {
        message: String,
    },
}

impl State {
    fn as_str(&self) -> &'static str {
        match self {
            State::NotCloned => "not_cloned",
            State::NotACheckout => "not_a_checkout",
            State::NoOrigin => "no_origin",
            State::RemoteMismatch { .. } => "remote_mismatch",
            State::Unborn => "unborn",
            State::Detached => "detached",
            State::Branch => "branch",
            State::Error { .. } => "error",
        }
    }
}

/// One checkout's local state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inspection {
    state: State,
    branch: Option<String>,
    rev: Option<String>,
    /// Tracked files modified. Known only on a branch.
    dirty: Option<bool>,
    /// `None` when the branch tracks no remote branch.
    ahead: Option<u32>,
    behind: Option<u32>,
}

impl Inspection {
    fn only(state: State) -> Inspection {
        Inspection {
            state,
            branch: None,
            rev: None,
            dirty: None,
            ahead: None,
            behind: None,
        }
    }
}

pub fn run(ctx: &mut Ctx, args: &StatusArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    for name in &args.names {
        repo_by_name(&conn, name)?;
    }
    let root_id = match &args.root {
        Some(name) => Some(root_by_name(&conn, name)?.id),
        None => None,
    };
    let entries = db::repos::select(
        &conn,
        &Selection {
            names: &args.names,
            tags: &args.tags,
            root_id,
            include_disabled: true,
        },
    )?;

    let roots = roots_by_id(&conn)?;
    let vcs = vcs::backend(vcs::VcsKind::Git, tracer(ui));
    let mut inspected = Vec::with_capacity(entries.len());
    for entry in &entries {
        let repo = &entry.repo;
        let path = checkout_path(&roots[&repo.root_id], repo)?;
        let inspection = inspect(vcs.as_ref(), repo, &path);
        inspected.push((repo, path, inspection));
    }

    if args.format == Format::Json {
        let items = inspected
            .iter()
            .map(|(repo, path, inspection)| to_json(repo, path, inspection))
            .collect();
        print_json(ui, &Value::Array(items))?;
        return Ok(ExitCode::SUCCESS);
    }
    if inspected.is_empty() {
        ui.progress("no repositories match");
        return Ok(ExitCode::SUCCESS);
    }
    let mut table = Table::new(["NAME", "BRANCH", "REV", "STATE"]);
    for (repo, _, inspection) in &inspected {
        table.push(row(repo, inspection));
    }
    ui.table(&table);
    Ok(ExitCode::SUCCESS)
}

/// Read a checkout's state with the same checks, in the same order, as a
/// sync would make, but without fetching or changing anything.
fn inspect(vcs: &dyn Vcs, repo: &Repo, path: &Path) -> Inspection {
    let error = |err: anyhow::Error| {
        Inspection::only(State::Error {
            message: format!("{err:#}"),
        })
    };
    if !path.exists() {
        return Inspection::only(State::NotCloned);
    }
    if !vcs.is_checkout(path) {
        return Inspection::only(State::NotACheckout);
    }
    match vcs.remote_url(path) {
        Ok(Some(found)) if remote::same_repo(&found, &repo.url) => {}
        Ok(Some(found)) => return Inspection::only(State::RemoteMismatch { found }),
        Ok(None) => return Inspection::only(State::NoOrigin),
        Err(err) => return error(err),
    }
    let (branch, rev) = match vcs.head(path) {
        Ok(Head::Branch { name, rev }) => (name, rev),
        Ok(Head::Detached { rev }) => {
            return Inspection {
                rev: Some(rev),
                ..Inspection::only(State::Detached)
            };
        }
        Ok(Head::Unborn) => return Inspection::only(State::Unborn),
        Err(err) => return error(err),
    };
    let dirty = match vcs.is_dirty(path) {
        Ok(dirty) => dirty,
        Err(err) => return error(err),
    };
    let divergence = match vcs.divergence(path) {
        Ok(d) => d,
        Err(err) => return error(err),
    };
    Inspection {
        state: State::Branch,
        branch: Some(branch),
        rev: Some(rev),
        dirty: Some(dirty),
        ahead: divergence.map(|d| d.ahead),
        behind: divergence.map(|d| d.behind),
    }
}

/// The table row: name, branch, short revision and a state in words.
fn row(repo: &Repo, inspection: &Inspection) -> Vec<Cell> {
    let (text, style) = describe(inspection);
    let (text, style) = if repo.enabled {
        (text, style)
    } else {
        ("disabled".to_owned(), THEME.muted)
    };
    let branch = match (&inspection.state, &inspection.branch) {
        (State::Detached, _) => Cell::new("(detached)", THEME.warn),
        (_, Some(branch)) => Cell::new(branch, THEME.name),
        _ => Cell::plain(""),
    };
    vec![
        Cell::new(
            &repo.name,
            if repo.enabled {
                THEME.name
            } else {
                THEME.muted
            },
        ),
        branch,
        Cell::new(
            inspection.rev.as_deref().map(short_rev).unwrap_or(""),
            THEME.rev,
        ),
        Cell::new(text, style),
    ]
}

/// The state in words, yellow when it needs attention.
fn describe(inspection: &Inspection) -> (String, Style) {
    match &inspection.state {
        State::NotCloned => ("not cloned".into(), THEME.warn),
        State::NotACheckout => ("not a Git checkout".into(), THEME.err),
        State::NoOrigin => ("no origin remote".into(), THEME.warn),
        State::RemoteMismatch { found } => (format!("origin is {found}"), THEME.warn),
        State::Unborn => ("no commits yet".into(), THEME.warn),
        State::Detached => ("HEAD is detached".into(), THEME.warn),
        State::Error { message } => (message.clone(), THEME.err),
        State::Branch => {
            let mut words = Vec::new();
            let mut style = THEME.muted;
            if inspection.dirty == Some(true) {
                words.push("local changes".to_owned());
                style = THEME.warn;
            }
            match (inspection.ahead, inspection.behind) {
                (Some(ahead), Some(behind)) => {
                    if ahead > 0 {
                        words.push(count(ahead, "local commit"));
                    }
                    if behind > 0 {
                        words.push(format!("{} behind", count(behind, "commit")));
                        style = THEME.warn;
                    }
                }
                _ => {
                    words.push("no upstream".to_owned());
                    style = THEME.warn;
                }
            }
            if words.is_empty() {
                ("clean".into(), style)
            } else {
                (words.join("; "), style)
            }
        }
    }
}

fn to_json(repo: &Repo, path: &Path, inspection: &Inspection) -> Value {
    let (origin, message) = match &inspection.state {
        State::RemoteMismatch { found } => (Some(found.as_str()), None),
        State::Error { message } => (None, Some(message.as_str())),
        _ => (None, None),
    };
    json!({
        "name": repo.name,
        "enabled": repo.enabled,
        "path": path_text(path),
        "state": inspection.state.as_str(),
        "branch": inspection.branch,
        "rev": inspection.rev,
        "dirty": inspection.dirty,
        "ahead": inspection.ahead,
        "behind": inspection.behind,
        "has_upstream": inspection.ahead.is_some(),
        "origin": origin,
        "message": message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_branch(dirty: bool, ahead: Option<u32>, behind: Option<u32>) -> Inspection {
        Inspection {
            state: State::Branch,
            branch: Some("main".into()),
            rev: Some("0123456789".into()),
            dirty: Some(dirty),
            ahead,
            behind,
        }
    }

    #[test]
    fn descriptions() {
        let text = |i: &Inspection| describe(i).0;
        assert_eq!(text(&on_branch(false, Some(0), Some(0))), "clean");
        assert_eq!(
            text(&on_branch(true, Some(2), Some(1))),
            "local changes; 2 local commits; 1 commit behind"
        );
        assert_eq!(text(&on_branch(false, None, None)), "no upstream");
        assert_eq!(text(&Inspection::only(State::NotCloned)), "not cloned");
    }
}

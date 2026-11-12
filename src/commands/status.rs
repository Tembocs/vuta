//! `vuta status [name]...`: the local state of each checkout, without
//! touching the network. Ahead and behind counts compare with the
//! remote-tracking branch as of the last fetch.

use std::process::ExitCode;

use anstyle::Style;

use super::{Ctx, checkout_path, repo_by_name, root_by_name, tracer};
use crate::Error;
use crate::cli::StatusArgs;
use crate::db;
use crate::db::repos::Selection;
use crate::model::{Repo, count, short_rev};
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::vcs::{self, Head, Vcs, remote};

/// One checkout's state, as table cells.
struct State {
    branch: Cell,
    rev: Cell,
    state: Cell,
}

impl State {
    fn only(text: impl Into<String>, style: Style) -> State {
        State {
            branch: Cell::plain(""),
            rev: Cell::plain(""),
            state: Cell::new(text, style),
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
    if entries.is_empty() {
        ui.progress("no repositories match");
        return Ok(ExitCode::SUCCESS);
    }

    let roots = db::roots::all(&conn)?;
    let vcs = vcs::backend(vcs::VcsKind::Git, tracer(ui));
    let mut table = Table::new(["NAME", "BRANCH", "REV", "STATE"]);
    for entry in entries {
        let repo = &entry.repo;
        let root = roots
            .iter()
            .find(|r| r.id == repo.root_id)
            .expect("every repository has a root");
        let path = checkout_path(root, repo)?;
        let state = inspect(vcs.as_ref(), repo, &path);
        let name_style = if repo.enabled {
            THEME.name
        } else {
            THEME.muted
        };
        let mut state_cell = state.state;
        if !repo.enabled {
            state_cell = Cell::new("disabled", THEME.muted);
        }
        table.push(vec![
            Cell::new(&repo.name, name_style),
            state.branch,
            state.rev,
            state_cell,
        ]);
    }
    ui.table(&table);
    Ok(ExitCode::SUCCESS)
}

/// Read a checkout's state with the same checks, in the same order, as a
/// sync would make, but without fetching or changing anything.
fn inspect(vcs: &dyn Vcs, repo: &Repo, path: &std::path::Path) -> State {
    if !path.exists() {
        return State::only("not cloned", THEME.warn);
    }
    if !vcs.is_checkout(path) {
        return State::only("not a Git checkout", THEME.err);
    }
    match vcs.remote_url(path) {
        Ok(Some(found)) if remote::same_repo(&found, &repo.url) => {}
        Ok(Some(found)) => return State::only(format!("origin is {found}"), THEME.warn),
        Ok(None) => return State::only("no origin remote", THEME.warn),
        Err(err) => return State::only(format!("{err:#}"), THEME.err),
    }
    let (branch, rev) = match vcs.head(path) {
        Ok(Head::Branch { name, rev }) => (Cell::new(name, THEME.name), rev),
        Ok(Head::Detached { rev }) => {
            return State {
                branch: Cell::new("(detached)", THEME.warn),
                rev: Cell::new(short_rev(&rev), THEME.rev),
                state: Cell::new("HEAD is detached", THEME.warn),
            };
        }
        Ok(Head::Unborn) => return State::only("no commits yet", THEME.warn),
        Err(err) => return State::only(format!("{err:#}"), THEME.err),
    };
    let rev_cell = Cell::new(short_rev(&rev), THEME.rev);
    let mut words = Vec::new();
    let mut style = THEME.muted;
    match vcs.is_dirty(path) {
        Ok(true) => {
            words.push("local changes".to_owned());
            style = THEME.warn;
        }
        Ok(false) => {}
        Err(err) => return State::only(format!("{err:#}"), THEME.err),
    }
    match vcs.divergence(path) {
        Ok(Some(d)) => {
            if d.ahead > 0 {
                words.push(count(d.ahead, "local commit"));
            }
            if d.behind > 0 {
                words.push(format!("{} behind", count(d.behind, "commit")));
                style = THEME.warn;
            }
        }
        Ok(None) => {
            words.push("no upstream".to_owned());
            style = THEME.warn;
        }
        Err(err) => return State::only(format!("{err:#}"), THEME.err),
    }
    let text = if words.is_empty() {
        "clean".to_owned()
    } else {
        words.join("; ")
    };
    State {
        branch,
        rev: rev_cell,
        state: Cell::new(text, style),
    }
}

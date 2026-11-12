//! `vuta add <url> [path]`: register a repository and clone it. The clone
//! is an ordinary one-repository run, so it is recorded in the history.

use std::process::ExitCode;

use anyhow::anyhow;

use super::sync::{HookMode, Plan};
use super::{Ctx, check_label, location, rel_path, root_or_default, stored_url};
use crate::Error;
use crate::cli::{AddArgs, Format};
use crate::db;
use crate::db::repos::{NewRepo, Selection};
use crate::model::VcsKind;
use crate::paths;
use crate::sync;
use crate::ui::Styled;
use crate::ui::theme::THEME;
use crate::vcs::remote;

pub fn run(ctx: &mut Ctx, args: &AddArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let root = root_or_default(&conn, args.root.as_deref())?;
    let name = match &args.name {
        Some(name) => name.clone(),
        None => remote::infer_name(&args.url)
            .ok_or_else(|| anyhow!("cannot infer a name from {}; pass --name", args.url))?,
    };
    check_label("repository name", &name)?;
    for tag in &args.tags {
        check_label("tag", tag)?;
    }
    if let Some(existing) = db::repos::by_name(&conn, &name)? {
        return Err(anyhow!(
            "a repository named '{}' already exists; choose another name with --name",
            existing.name
        )
        .into());
    }
    let rel = rel_path(args.path.as_deref().unwrap_or(&name))?;
    let url = stored_url(&args.url)?;

    // Take the lock before registering, so a running sync cannot make
    // this add fail half-way through.
    let lock = if args.no_clone {
        None
    } else {
        Some(sync::acquire_lock(&paths::lock_path(&ctx.db_path))?)
    };
    let repo = db::repos::insert(
        &conn,
        &NewRepo {
            name: &name,
            root_id: root.id,
            rel_path: &rel,
            vcs: VcsKind::Git,
            url: &url,
            branch: args.branch.as_deref(),
            clone_filter: args.filter.as_deref(),
            clone_depth: args.depth,
            tags: &args.tags,
        },
    )?;

    let mut line = Styled::new();
    line.push("added ", THEME.new)
        .push(&repo.name, THEME.name)
        .plain("  ")
        .push(&location(&root, &repo), THEME.muted);
    ui.result(line);
    if lock.is_none() {
        ui.progress(format_args!(
            "not cloned yet; the next sync clones it: vuta sync {}",
            repo.name
        ));
        return Ok(ExitCode::SUCCESS);
    }

    let names = [repo.name.clone()];
    let entries = db::repos::select(
        &conn,
        &Selection {
            names: &names,
            ..Default::default()
        },
    )?;
    super::sync::execute(
        ui,
        &conn,
        Plan {
            entries,
            workers: 1,
            fetch_only: false,
            hooks: HookMode::Normal,
            strict: false,
            framed: false,
            format: Format::Text,
        },
    )
}

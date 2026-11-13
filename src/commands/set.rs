//! `vuta set <name> [options]`: change a repository's record. Files on
//! disk are never moved.

use std::process::ExitCode;

use super::{
    Ctx, check_label, checkout_path, location, rel_path, repo_by_name, root_by_name, root_of,
    stored_url,
};
use crate::Error;
use crate::cli::SetArgs;
use crate::db;
use crate::db::repos::Changes;
use crate::ui::Styled;
use crate::ui::theme::THEME;

pub fn run(ctx: &mut Ctx, args: &SetArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let repo = repo_by_name(&conn, &args.name)?;
    let old_root = root_of(&conn, &repo)?;
    let old_path = checkout_path(&old_root, &repo)?;

    let mut changes = Changes::default();
    if let Some(name) = &args.new_name {
        check_label("repository name", name)?;
        changes.name = Some(name.clone());
    }
    if let Some(url) = &args.url {
        changes.url = Some(stored_url(url)?);
    }
    if let Some(branch) = &args.branch {
        changes.branch = Some(Some(branch.clone()));
    } else if args.default_branch {
        changes.branch = Some(None);
    }
    let new_root = match &args.root {
        Some(name) => {
            let root = root_by_name(&conn, name)?;
            changes.root_id = Some(root.id);
            root
        }
        None => old_root.clone(),
    };
    if let Some(path) = &args.path {
        changes.rel_path = Some(rel_path(path)?);
    }
    if let Some(filter) = &args.filter {
        changes.clone_filter = Some(Some(filter.clone()));
    } else if args.no_filter {
        changes.clone_filter = Some(None);
    }
    if let Some(depth) = args.depth {
        changes.clone_depth = Some(Some(depth));
    } else if args.no_depth {
        changes.clone_depth = Some(None);
    }
    if args.enable {
        changes.enabled = Some(true);
    } else if args.disable {
        changes.enabled = Some(false);
    }
    if args.allow_dirty {
        changes.allow_dirty = Some(true);
    } else if args.no_allow_dirty {
        changes.allow_dirty = Some(false);
    }
    if args.submodules {
        changes.submodules = Some(true);
    } else if args.no_submodules {
        changes.submodules = Some(false);
    }
    changes.strategy = args.strategy;

    db::repos::update(&conn, repo.id, &changes)?;
    let updated = repo_by_name(&conn, changes.name.as_deref().unwrap_or(&repo.name))?;

    let mut line = Styled::new();
    line.plain("updated ").push(&updated.name, THEME.name);
    if updated.name != repo.name {
        line.plain(" (was ").push(&repo.name, THEME.name).plain(")");
    }
    ui.result(line);

    let new_path = checkout_path(&new_root, &updated)?;
    if new_path != old_path {
        let mut line = Styled::new();
        line.plain("  location ")
            .push(&location(&old_root, &repo), THEME.muted)
            .plain(&format!(" {} ", ui.glyphs().arrow))
            .push(&location(&new_root, &updated), THEME.muted);
        ui.result(line);
        if old_path.exists() && !new_path.exists() {
            ui.warn(format_args!(
                "no files were moved; move {} to {} yourself, or the next sync clones a new copy",
                old_path.display(),
                new_path.display()
            ));
        }
    }
    Ok(ExitCode::SUCCESS)
}

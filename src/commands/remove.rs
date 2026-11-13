//! `vuta remove <name>` (alias `rm`): forget a repository, and with
//! `--delete-files` also delete its checkout after a confirmation.

use std::fs;
use std::process::ExitCode;

use anyhow::anyhow;

use super::{Ctx, checkout_path, repo_by_name, root_of, tracer};
use crate::Error;
use crate::cli::RemoveArgs;
use crate::db;
use crate::error::exit;
use crate::ui::Styled;
use crate::ui::theme::THEME;
use crate::vcs;

pub fn run(ctx: &mut Ctx, args: &RemoveArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let repo = repo_by_name(&conn, &args.name)?;
    let root = root_of(&conn, &repo)?;
    let path = checkout_path(&root, &repo)?;
    let delete = args.delete_files && path.exists();

    if delete {
        let vcs = vcs::backend(repo.vcs, tracer(ui));
        // Only ever delete what is recognisably this repository's checkout.
        if !vcs.is_checkout(&path) {
            return Err(anyhow!(
                "{} is not a Git checkout; not deleting it. \
                 Remove it yourself, or drop --delete-files to only forget the repository",
                path.display()
            )
            .into());
        }
        if !args.yes {
            let dirty = vcs.is_dirty(&path, false).unwrap_or(false);
            let question = format!(
                "Delete {}{}?",
                path.display(),
                if dirty {
                    ", which has local changes,"
                } else {
                    ""
                }
            );
            let confirmed = ui
                .confirm(question)
                .map_err(|err| anyhow!("cannot read the answer: {err}"))?;
            if !confirmed {
                ui.progress("cancelled; nothing was removed");
                return Ok(ExitCode::from(exit::FAILED));
            }
        }
        fs::remove_dir_all(&path)
            .map_err(|err| anyhow!("cannot delete {}: {err}", path.display()))?;
    }

    db::repos::remove(&conn, repo.id)?;
    let mut line = Styled::new();
    line.plain("removed ").push(&repo.name, THEME.name);
    if delete {
        line.plain(" and deleted ")
            .push(&path.display().to_string(), THEME.muted);
    } else if path.exists() {
        line.plain("; its checkout stays at ")
            .push(&path.display().to_string(), THEME.muted);
    }
    ui.result(line);
    Ok(ExitCode::SUCCESS)
}

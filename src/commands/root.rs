//! `vuta root add|list|default|move|remove`.

use std::process::ExitCode;

use super::{Ctx, absolute, check_label, root_by_name};
use crate::Error;
use crate::cli::RootCommand;
use crate::db;
use crate::ui::Styled;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;

pub fn run(ctx: &mut Ctx, cmd: &RootCommand) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;
    match cmd {
        RootCommand::Add {
            name,
            path,
            default,
        } => {
            check_label("root name", name)?;
            let path = absolute(path)?;
            let root = db::roots::add(&conn, name, &path, *default)?;
            let mut line = Styled::new();
            line.plain("added root ")
                .push(&root.name, THEME.name)
                .plain(" at ")
                .push(&root.path.display().to_string(), THEME.muted);
            if root.is_default {
                line.plain(" (default)");
            }
            ui.result(line);
            if !root.path.is_dir() {
                ui.warn(format_args!(
                    "{} does not exist yet; the first clone creates it",
                    root.path.display()
                ));
            }
        }
        RootCommand::List => {
            let roots = db::roots::with_repo_counts(&conn)?;
            if roots.is_empty() {
                ui.progress("no roots registered; add one with: vuta root add <name> <path>");
                return Ok(ExitCode::SUCCESS);
            }
            let mut table = Table::new(["NAME", "REPOS", "PATH", ""]);
            for (root, repos) in roots {
                let path = root.path.display().to_string();
                let path = if root.path.is_dir() {
                    Cell::new(path, THEME.muted)
                } else {
                    Cell::new(format!("{path} (missing)"), THEME.warn)
                };
                table.push(vec![
                    Cell::new(root.name, THEME.name),
                    Cell::plain(repos.to_string()),
                    path,
                    Cell::plain(if root.is_default { "default" } else { "" }),
                ]);
            }
            ui.table(&table);
        }
        RootCommand::Default { name } => {
            let root = root_by_name(&conn, name)?;
            db::roots::set_default(&conn, root.id)?;
            let mut line = Styled::new();
            line.push(&root.name, THEME.name)
                .plain(" is now the default root");
            ui.result(line);
        }
        RootCommand::Move { name, new_path } => {
            let root = root_by_name(&conn, name)?;
            let path = absolute(new_path)?;
            db::roots::set_path(&conn, root.id, &path)?;
            let mut line = Styled::new();
            line.plain("root ")
                .push(&root.name, THEME.name)
                .plain(" now points at ")
                .push(&path.display().to_string(), THEME.muted);
            ui.result(line);
            if !path.is_dir() {
                ui.warn(format_args!(
                    "{} does not exist; no files were moved",
                    path.display()
                ));
            }
        }
        RootCommand::Remove { name } => {
            let root = root_by_name(&conn, name)?;
            db::roots::remove(&conn, &root)?;
            let mut line = Styled::new();
            line.plain("removed root ").push(&root.name, THEME.name);
            ui.result(line);
            if root.is_default && !db::roots::all(&conn)?.is_empty() {
                ui.warn("there is no default root now; set one with: vuta root default <name>");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

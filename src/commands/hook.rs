//! `vuta hook add|list|remove`: commands that run after a repository syncs.
//! Hooks enter the database only through `vuta hook add`.

use std::process::ExitCode;

use anyhow::anyhow;

use super::{Ctx, print_json, repo_by_name};
use crate::Error;
use crate::cli::{Format, HookCommand};
use crate::db;
use crate::model::RunWhen;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, json};

pub fn run(ctx: &mut Ctx, cmd: &HookCommand) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;
    match cmd {
        HookCommand::Add {
            repo,
            command,
            platform,
            always,
        } => {
            let repo = repo_by_name(&conn, repo)?;
            if command.trim().is_empty() {
                return Err(anyhow!("the hook command is empty").into());
            }
            let run_when = if *always {
                RunWhen::Always
            } else {
                RunWhen::Changed
            };
            let hook = db::hooks::add(&conn, repo.id, command, *platform, run_when)?;
            let mut line = Styled::new();
            line.plain("added hook ")
                .push(&hook.id.to_string(), THEME.name)
                .plain(" to ")
                .push(&repo.name, THEME.name)
                .plain(": ")
                .plain(&hook.command);
            ui.result(line);
            if !platform.matches_current() {
                ui.warn(format_args!(
                    "this hook runs only on {}, so not on this machine",
                    platform
                ));
            }
        }
        HookCommand::List { repo, format } => {
            let hooks = match repo {
                Some(name) => {
                    let repo = repo_by_name(&conn, name)?;
                    db::hooks::for_repo(&conn, repo.id)?
                        .into_iter()
                        .map(|hook| (repo.name.clone(), hook))
                        .collect()
                }
                None => db::hooks::all(&conn)?,
            };
            if *format == Format::Json {
                let items = hooks.iter().map(|(name, hook)| json::hook(name, hook));
                print_json(ui, &serde_json::Value::Array(items.collect()))?;
                return Ok(ExitCode::SUCCESS);
            }
            if hooks.is_empty() {
                ui.progress("no hooks; add one with: vuta hook add <repo> <command>");
                return Ok(ExitCode::SUCCESS);
            }
            let mut table = Table::new(["ID", "REPO", "PLATFORM", "WHEN", "COMMAND"]);
            for (name, hook) in hooks {
                // Hooks that cannot run on this machine are dim.
                let style = if hook.platform.matches_current() && hook.enabled {
                    anstyle::Style::new()
                } else {
                    THEME.muted
                };
                table.push(vec![
                    Cell::new(hook.id.to_string(), THEME.muted),
                    Cell::new(name, THEME.name),
                    Cell::new(hook.platform.as_str(), style),
                    Cell::new(hook.run_when.as_str(), style),
                    Cell::new(hook.command, style),
                ]);
            }
            ui.table(&table);
        }
        HookCommand::Remove { id } => {
            let hook = db::hooks::by_id(&conn, *id)?
                .ok_or_else(|| anyhow!("no hook with id {id}; see: vuta hook list"))?;
            db::hooks::remove(&conn, hook.id)?;
            let mut line = Styled::new();
            line.plain("removed hook ")
                .push(&hook.id.to_string(), THEME.name)
                .plain(": ")
                .plain(&hook.command);
            ui.result(line);
        }
    }
    Ok(ExitCode::SUCCESS)
}

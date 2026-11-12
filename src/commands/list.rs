//! `vuta list` (alias `ls`): one line per repository.

use std::process::ExitCode;

use anstyle::Style;

use super::{Ctx, now_unix, root_by_name};
use crate::Error;
use crate::cli::ListArgs;
use crate::db;
use crate::db::repos::Selection;
use crate::model::{OutcomeKind, short_rev};
use crate::ui::fmt_age;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;

pub fn run(ctx: &mut Ctx, args: &ListArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let root_id = match &args.root {
        Some(name) => Some(root_by_name(&conn, name)?.id),
        None => None,
    };
    let entries = db::repos::select(
        &conn,
        &Selection {
            tags: &args.tags,
            root_id,
            include_disabled: true,
            ..Default::default()
        },
    )?;
    if entries.is_empty() {
        if args.tags.is_empty() && args.root.is_none() {
            ui.progress("no repositories registered; add one with: vuta add <url>");
        } else {
            ui.progress("no repositories match");
        }
        return Ok(ExitCode::SUCCESS);
    }

    // Name the root only when there is more than one to tell apart.
    let show_root = db::roots::all(&conn)?.len() > 1;
    let mut headers = vec!["NAME", "TAGS"];
    if show_root {
        headers.push("ROOT");
    }
    headers.extend(["PATH", "REV", "SYNCED"]);
    let mut table = Table::new(headers);
    let now = now_unix();

    for entry in entries {
        let enabled = entry.repo.enabled;
        // A disabled repository is dim across the whole row.
        let style = |s: Style| if enabled { s } else { THEME.muted };
        let synced = match entry.last.at {
            Some(at) => fmt_age(now - at),
            None => "never".to_owned(),
        };
        let synced_style = match entry.last.outcome {
            Some(OutcomeKind::Skipped) => THEME.warn,
            Some(OutcomeKind::Failed) => THEME.err,
            _ => THEME.muted,
        };
        let mut row = vec![
            Cell::new(entry.repo.name, style(THEME.name)),
            Cell::new(entry.tags.join(","), style(Style::new())),
        ];
        if show_root {
            row.push(Cell::new(entry.root_name, style(Style::new())));
        }
        row.extend([
            Cell::new(entry.repo.rel_path, THEME.muted),
            Cell::new(
                entry.last.rev.as_deref().map(short_rev).unwrap_or("-"),
                style(THEME.rev),
            ),
            Cell::new(
                if enabled {
                    synced
                } else {
                    format!("{synced} (disabled)")
                },
                style(synced_style),
            ),
        ]);
        table.push(row);
    }
    ui.table(&table);
    Ok(ExitCode::SUCCESS)
}

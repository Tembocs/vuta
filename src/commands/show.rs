//! `vuta show <name>`: everything about one repository.

use std::process::ExitCode;

use super::{
    Ctx, checkout_path, now_unix, print_json, repo_by_name, root_of, stored_detail, stored_hooks,
    stored_status, sync_options,
};
use crate::Error;
use crate::cli::{Format, ShowArgs};
use crate::db;
use crate::db::repos::Selection;
use crate::model::{OutcomeKind, SubmoduleOutcome, short_rev};
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, fmt_age, json, pad, report};

/// How many recent results to show.
const RECENT: u32 = 10;
/// Width of the label column.
const LABEL: usize = 9;

pub fn run(ctx: &mut Ctx, args: &ShowArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;
    let glyphs = ui.glyphs();

    let repo = repo_by_name(&conn, &args.name)?;
    let root = root_of(&conn, &repo)?;
    let path = checkout_path(&root, &repo)?;
    let names = [repo.name.clone()];
    let entry = db::repos::select(
        &conn,
        &Selection {
            names: &names,
            include_disabled: true,
            ..Default::default()
        },
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::UnknownRepo(args.name.clone()))?;
    if args.format == Format::Json {
        // The fields of `vuta list`, then the hooks and recent results.
        let mut value = json::repo(&entry, &path);
        let hooks = db::hooks::for_repo(&conn, repo.id)?;
        value["hooks"] = hooks.iter().map(|h| json::hook(&repo.name, h)).collect();
        let results = db::runs::results_for_repo(&conn, repo.id, RECENT)?;
        value["results"] = results.iter().map(json::result).collect();
        print_json(ui, &value)?;
        return Ok(ExitCode::SUCCESS);
    }
    let field = |label: &str, value: &Styled| {
        let mut line = Styled::new();
        line.push(&pad(label, LABEL), THEME.muted).append(value);
        ui.result(line);
    };
    let text = |text: &str, style| {
        let mut value = Styled::new();
        value.push(text, style);
        value
    };

    let mut title = Styled::new();
    title.push(&repo.name, THEME.name);
    ui.result(title);

    let mut value = Styled::new();
    value
        .push(&root.name, THEME.name)
        .plain("  ")
        .push(&root.path.display().to_string(), THEME.muted);
    field("root", &value);

    let mut value = text(&path.display().to_string(), anstyle::Style::new());
    if !path.exists() {
        value.push("  (not cloned)", THEME.warn);
    }
    field("path", &value);
    field("url", &text(&repo.url, anstyle::Style::new()));
    field(
        "branch",
        &match &repo.branch {
            Some(branch) => text(branch, THEME.name),
            None => text("the remote's default", THEME.muted),
        },
    );
    let mut clone = Vec::new();
    if let Some(filter) = &repo.clone_filter {
        clone.push(format!("filter {filter}"));
    }
    if let Some(depth) = repo.clone_depth {
        clone.push(format!("depth {depth}"));
    }
    if !clone.is_empty() {
        field("clone", &text(&clone.join(", "), anstyle::Style::new()));
    }
    field(
        "sync",
        &text(
            &sync_options(repo.allow_dirty, repo.submodules, repo.strategy).join(", "),
            anstyle::Style::new(),
        ),
    );
    field(
        "enabled",
        &if repo.enabled {
            text("yes", anstyle::Style::new())
        } else {
            text("no", THEME.warn)
        },
    );
    field(
        "tags",
        &if entry.tags.is_empty() {
            text("none", THEME.muted)
        } else {
            text(&entry.tags.join(", "), anstyle::Style::new())
        },
    );
    let mut value = Styled::new();
    match (entry.last.outcome, entry.last.at) {
        (Some(outcome), Some(at)) => {
            let (style, glyph, word) = report::status(outcome, glyphs);
            value
                .push(&format!("{glyph} {word}"), style)
                .plain("  ")
                .push(&fmt_age(now_unix() - at), THEME.muted);
        }
        _ => {
            value.push("never", THEME.muted);
        }
    }
    if let Some(rev) = &entry.last.rev {
        value.plain("  ").push(short_rev(rev), THEME.rev);
    }
    field("synced", &value);

    let hooks = db::hooks::for_repo(&conn, repo.id)?;
    if !hooks.is_empty() {
        ui.blank();
        ui.result(text("Hooks", THEME.name));
        let mut table = Table::new(["ID", "PLATFORM", "WHEN", "COMMAND"]);
        for hook in hooks {
            let style = if hook.platform.matches_current() && hook.enabled {
                anstyle::Style::new()
            } else {
                THEME.muted
            };
            table.push(vec![
                Cell::new(hook.id.to_string(), THEME.muted),
                Cell::new(hook.platform.as_str(), style),
                Cell::new(hook.run_when.as_str(), style),
                Cell::new(hook.command, style),
            ]);
        }
        ui.table(&table);
    }

    let results = db::runs::results_for_repo(&conn, repo.id, RECENT)?;
    if !results.is_empty() {
        ui.blank();
        ui.result(text("Recent results", THEME.name));
        let now = now_unix();
        let mut table = Table::new(["WHEN", "RESULT", "HOOKS", "DETAIL"]);
        for result in &results {
            table.push(vec![
                Cell::new(fmt_age(now - result.run_started_at), THEME.muted),
                stored_status(result, glyphs),
                stored_hooks(result),
                Cell::styled(stored_detail(result, glyphs)),
            ]);
        }
        ui.table(&table);

        // The result line shows only the last line of git's output; the
        // newest failure, of the repository or of its submodules, is shown
        // in full.
        let failure = results
            .iter()
            .find(|r| {
                r.outcome == OutcomeKind::Failed || r.submodules == Some(SubmoduleOutcome::Failed)
            })
            .and_then(|r| r.message.as_deref().map(|m| (r, m.trim())))
            .filter(|(_, message)| !message.is_empty());
        if let Some((result, message)) = failure {
            ui.blank();
            let mut heading = text("Last failure", THEME.name);
            heading
                .plain("  ")
                .push(&fmt_age(now - result.run_started_at), THEME.muted);
            ui.result(heading);
            for line in message.lines() {
                ui.result(text(&format!("  {line}"), THEME.muted));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

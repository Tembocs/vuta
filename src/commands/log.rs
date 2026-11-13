//! `vuta log [name]`: recent sync runs and their results.
//!
//! Without a name, each run gets a line with its time, exit code and
//! command, its summary, and a line per repository that did not end up
//! to date. With a name, that repository's results are listed instead.
//! With `--format json`, runs carry all their results.

use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

use super::{Ctx, now_unix, print_json, repo_by_name, stored_detail, stored_hooks, stored_status};
use crate::Error;
use crate::cli::{Format, LogArgs};
use crate::db;
use crate::error::exit;
use crate::model::{HookOutcome, OutcomeKind, Run};
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, fmt_age, json, pad, report, text_width};

pub fn run(ctx: &mut Ctx, args: &LogArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;
    let glyphs = ui.glyphs();
    let now = now_unix();

    if let Some(name) = &args.name {
        let repo = repo_by_name(&conn, name)?;
        let results = db::runs::results_for_repo(&conn, repo.id, args.limit)?;
        if args.format == Format::Json {
            print_json(
                ui,
                &Value::Array(results.iter().map(json::result).collect()),
            )?;
            return Ok(ExitCode::SUCCESS);
        }
        if results.is_empty() {
            ui.progress(format_args!("{} has not been synced yet", repo.name));
            return Ok(ExitCode::SUCCESS);
        }
        let mut table = Table::new(["RUN", "WHEN", "RESULT", "HOOKS", "DETAIL"]);
        for result in &results {
            table.push(vec![
                Cell::new(result.run_id.to_string(), THEME.muted),
                Cell::new(fmt_age(now - result.run_started_at), THEME.muted),
                stored_status(result, glyphs),
                stored_hooks(result),
                Cell::styled(stored_detail(result, glyphs)),
            ]);
        }
        ui.table(&table);
        return Ok(ExitCode::SUCCESS);
    }

    let runs = db::runs::recent(&conn, args.limit)?;
    if args.format == Format::Json {
        let mut items = Vec::with_capacity(runs.len());
        for summary in &runs {
            let mut run = json::run(&summary.run);
            run["tally"] = json::tally(&summary.tally);
            run["results"] = db::runs::results_for_run(&conn, summary.run.id)?
                .iter()
                .map(json::result)
                .collect();
            items.push(run);
        }
        print_json(ui, &Value::Array(items))?;
        return Ok(ExitCode::SUCCESS);
    }
    if runs.is_empty() {
        ui.progress("no sync runs yet; start one with: vuta sync");
        return Ok(ExitCode::SUCCESS);
    }
    for (i, summary) in runs.iter().enumerate() {
        if i > 0 {
            ui.blank();
        }
        let run = &summary.run;
        ui.result(run_line(run, now));
        let mut tally = Styled::new();
        tally.plain("  ");
        match run.finished_at {
            Some(finished) => {
                let elapsed =
                    Duration::from_secs(u64::try_from(finished - run.started_at).unwrap_or(0));
                tally.plain(&report::summary_line(&summary.tally, elapsed, glyphs));
            }
            None => {
                tally.push("did not finish; the process was killed", THEME.warn);
            }
        }
        ui.result(tally);

        let notable: Vec<_> = db::runs::results_for_run(&conn, run.id)?
            .into_iter()
            .filter(|r| {
                r.outcome != OutcomeKind::UpToDate
                    || r.hook_outcome == Some(HookOutcome::Failed)
                    || r.submodules.is_some()
            })
            .collect();
        let name_width = notable
            .iter()
            .map(|r| text_width(&r.repo_name))
            .max()
            .unwrap_or(0);
        for result in &notable {
            let (style, glyph, word) = report::status(result.outcome, glyphs);
            let mut line = Styled::new();
            line.plain("  ")
                .push(&pad(glyph, glyphs.status_width()), style)
                .plain(" ")
                .push(&pad(word, 10), style)
                .plain("  ")
                .push(&pad(&result.repo_name, name_width), THEME.name)
                .plain("   ")
                .append(&stored_detail(result, glyphs));
            if result.hook_outcome == Some(HookOutcome::Failed) {
                line.plain("   ").push("hook failed", THEME.err);
            }
            ui.result(line);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `run 12   2 hours ago   exit 1   sync --tag rust`
fn run_line(run: &Run, now: i64) -> Styled {
    let mut line = Styled::new();
    line.push(&format!("run {}", run.id), THEME.name)
        .plain("   ")
        .push(&fmt_age(now - run.started_at), THEME.muted)
        .plain("   ");
    match run.exit_code {
        Some(code) => {
            let style = match u8::try_from(code) {
                Ok(exit::OK) => THEME.ok,
                Ok(exit::INTERRUPTED) => THEME.warn,
                _ => THEME.err,
            };
            line.push(&format!("exit {code}"), style);
        }
        None => {
            line.push("unfinished", THEME.warn);
        }
    }
    line.plain("   ").push(&run.args, THEME.muted);
    line
}

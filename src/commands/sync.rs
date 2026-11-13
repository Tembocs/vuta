//! `vuta sync [name]...`: clone what is missing and update the rest.
//!
//! A run goes through the phases of docs/ARCHITECTURE.md section 7:
//! select, lock, record, sync, hooks, finish. `vuta add` reuses `execute`
//! for the clone of the repository it just registered.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::json;

use super::{
    Ctx, checkout_path, location, print_json, repo_by_name, repositories, root_by_name, roots_by_id,
};
use crate::Error;
use crate::cli::{Format, SyncArgs};
use crate::db;
use crate::db::repos::Selection;
use crate::model::{HookOutcome, Outcome, RepoEntry, Tally};
use crate::paths;
use crate::sync::hooks::{self, HookContext, HookEvent};
use crate::sync::{self, Event, Job};
use crate::ui::live::Live;
use crate::ui::report::ResultLine;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Ui, Verbosity, json as ui_json};

/// How often live progress redraws its elapsed times.
const LIVE_TICK: Duration = Duration::from_millis(500);

/// Which hooks a run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    /// After a clone or an update, and `always` hooks after every sync.
    Normal,
    /// `--no-hooks`.
    Skip,
    /// `--force-hooks`: also after an up-to-date sync.
    Force,
}

/// What one run syncs and how. The caller already holds the sync lock.
#[derive(Debug)]
pub struct Plan {
    pub entries: Vec<RepoEntry>,
    pub workers: usize,
    pub fetch_only: bool,
    pub hooks: HookMode,
    pub strict: bool,
    /// Print the "Syncing N repositories" header and the summary line.
    /// `vuta add` prints only the result line.
    pub framed: bool,
    /// With JSON, stdout gets one object when the run ends and nothing
    /// else; hook output goes to stderr.
    pub format: Format,
}

pub fn run(ctx: &mut Ctx, args: &SyncArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    // Select. Named repositories must exist; a disabled one that is named
    // is reported as skipped rather than silently left out.
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
            include_disabled: !args.names.is_empty(),
        },
    )?;
    if entries.is_empty() {
        ui.progress("nothing to sync");
        if args.format == Format::Json {
            print_json(
                ui,
                &json!({
                    "run": null,
                    "interrupted": false,
                    "elapsed_ms": 0,
                    "tally": ui_json::tally(&Tally::default()),
                    "results": [],
                }),
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let workers = match args.jobs {
        Some(jobs) => jobs,
        None => db::settings::sync_jobs(&conn)?,
    };
    let workers = usize::try_from(workers)
        .unwrap_or(1)
        .clamp(1, entries.len());

    if args.dry_run {
        return dry_run(ui, &conn, &entries, workers, args.fetch_only, args.format);
    }

    let _lock = sync::acquire_lock(&paths::lock_path(&ctx.db_path))?;
    let hooks = if args.no_hooks {
        HookMode::Skip
    } else if args.force_hooks {
        HookMode::Force
    } else {
        HookMode::Normal
    };
    execute(
        ui,
        &conn,
        Plan {
            entries,
            workers,
            fetch_only: args.fetch_only,
            hooks,
            strict: args.strict,
            framed: true,
            format: args.format,
        },
    )
}

/// `--dry-run`: what each repository would get, judged from the disk
/// alone. Nothing is locked, recorded or run.
fn dry_run(
    ui: &Ui,
    conn: &Connection,
    entries: &[RepoEntry],
    workers: usize,
    fetch_only: bool,
    format: Format,
) -> Result<ExitCode, Error> {
    let roots = roots_by_id(conn)?;
    let action = |enabled: bool, path: &std::path::Path| {
        if !enabled {
            "skip"
        } else if path.exists() {
            "fetch"
        } else {
            "clone"
        }
    };
    if format == Format::Json {
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            let repo = &entry.repo;
            let path = checkout_path(&roots[&repo.root_id], repo)?;
            items.push(json!({
                "name": repo.name,
                "action": action(repo.enabled, &path),
                "path": ui_json::path_text(&path),
                "url": repo.url,
            }));
        }
        print_json(ui, &serde_json::Value::Array(items))?;
        return Ok(ExitCode::SUCCESS);
    }
    ui.progress(format_args!(
        "Would sync {}, {workers} at a time{}",
        repositories(entries.len()),
        if fetch_only { ", fetch only" } else { "" }
    ));
    ui.progress("");
    let mut table = Table::new(["ACTION", "NAME", "PATH", "FROM"]);
    for entry in entries {
        let repo = &entry.repo;
        let root = &roots[&repo.root_id];
        let path = checkout_path(root, repo)?;
        let action = action(repo.enabled, &path);
        let style = match action {
            "skip" => THEME.warn,
            "fetch" => THEME.ok,
            _ => THEME.new,
        };
        table.push(vec![
            Cell::new(action, style),
            Cell::new(&repo.name, THEME.name),
            Cell::new(location(root, repo), THEME.muted),
            Cell::new(
                if repo.enabled { &repo.url } else { "disabled" },
                THEME.muted,
            ),
        ]);
    }
    ui.table(&table);
    Ok(ExitCode::SUCCESS)
}

/// One repository that finished, kept for the hooks phase.
struct Finished {
    repo_id: i64,
    name: String,
    path: PathBuf,
    result_id: i64,
    outcome: Outcome,
}

/// Record, sync, hooks and finish for an already selected and locked plan.
pub fn execute(ui: &Ui, conn: &Connection, plan: Plan) -> Result<ExitCode, Error> {
    let stop = sync::interrupt_flag()?;
    let started = Instant::now();
    let text = plan.format == Format::Text;
    let roots = roots_by_id(conn)?;
    let mut names = HashMap::new();
    let mut paths = HashMap::new();
    let mut jobs = Vec::with_capacity(plan.entries.len());
    for entry in plan.entries {
        let repo = entry.repo;
        let path = checkout_path(&roots[&repo.root_id], &repo)?;
        names.insert(repo.id, repo.name.clone());
        paths.insert(repo.id, path.clone());
        jobs.push(Job { repo, path });
    }
    let total = jobs.len();
    let workers = plan.workers.clamp(1, total.max(1));
    let name_width = names
        .values()
        .map(|n| crate::ui::text_width(n))
        .max()
        .unwrap_or(0);

    if plan.framed {
        ui.progress(format_args!(
            "Syncing {}, {workers} at a time",
            repositories(total)
        ));
        if text {
            ui.blank();
        }
    }

    // Record.
    let run_id = db::runs::start(conn, env!("CARGO_PKG_VERSION"), &invocation())?;

    // Sync. This thread is the only one that touches SQLite and the
    // terminal; workers report through events.
    let mut tally = Tally::default();
    let mut finished: Vec<Finished> = Vec::with_capacity(total);
    let mut db_error = None;
    // On a terminal, a live line per busy worker; otherwise only the dim
    // "cloning" line, since clones can take minutes.
    let mut live = (text && ui.can_redraw()).then(|| Live::new(ui, total, name_width));
    let options = sync::Options {
        workers,
        fetch_only: plan.fetch_only,
        trace: ui.verbosity() == Verbosity::Verbose,
        tick: live.is_some().then_some(LIVE_TICK),
    };
    sync::run(jobs, &options, stop, |event| match event {
        Event::Started { repo_id } => {
            if let Some(live) = &mut live {
                live.started(repo_id, &names[&repo_id]);
            }
        }
        Event::Cloning { repo_id } => match &mut live {
            Some(live) => live.cloning(repo_id),
            None => ui.clone_started(&names[&repo_id]),
        },
        Event::Tick => {
            if let Some(live) = &mut live {
                live.tick();
            }
        }
        Event::Trace(captured) => ui.trace(&captured),
        Event::Finished {
            repo_id,
            outcome,
            duration,
        } => {
            tally.add(outcome.kind());
            if text {
                let line = ResultLine {
                    index: tally.total() as usize,
                    total,
                    name: &names[&repo_id],
                    name_width,
                    outcome: &outcome,
                    duration,
                };
                match &mut live {
                    Some(live) => live.finished(repo_id, &line),
                    None => ui.sync_result(&line),
                }
            }
            match db::runs::record_result(conn, run_id, repo_id, &outcome, duration) {
                Ok(result_id) => finished.push(Finished {
                    repo_id,
                    name: names[&repo_id].clone(),
                    path: paths[&repo_id].clone(),
                    result_id,
                    outcome,
                }),
                Err(err) => {
                    db_error.get_or_insert(err);
                }
            }
        }
    });
    // Every repository finished, so the block is empty already; this makes
    // sure nothing is left before hooks stream their output.
    drop(live);
    if let Some(err) = db_error {
        return Err(err.into());
    }

    // Hooks, one repository at a time in name order.
    finished.sort_by_key(|f| f.name.to_lowercase());
    let mut heading = false;
    for done in &finished {
        let hooks = db::hooks::for_repo(conn, done.repo_id)?;
        if hooks.is_empty() {
            continue;
        }
        let outcome = if plan.hooks == HookMode::Skip || stop.load(Ordering::Relaxed) {
            HookOutcome::NotRun
        } else {
            let force = plan.hooks == HookMode::Force;
            if !heading
                && hooks
                    .iter()
                    .any(|h| hooks::should_run(h, &done.outcome, force))
            {
                heading = true;
                if text {
                    ui.blank();
                    ui.hooks_heading();
                }
            }
            let ctx = HookContext {
                repo_name: &done.name,
                path: &done.path,
                outcome: &done.outcome,
                stdout_to_stderr: !text,
            };
            hooks::run(&ctx, &hooks, force, stop, |event| match event {
                HookEvent::Started { hook } if text => ui.hook_started(&done.name, &hook.command),
                HookEvent::Finished { end, duration, .. } if text => {
                    ui.hook_finished(&done.name, end, duration)
                }
                _ => {}
            })
        };
        if outcome == HookOutcome::Failed {
            tally.hooks_failed += 1;
        }
        db::runs::set_hook_outcome(conn, done.result_id, outcome)?;
    }

    // Finish.
    let interrupted = stop.load(Ordering::Relaxed);
    let code = sync::exit_code(&tally, plan.strict, interrupted);
    db::runs::finish(conn, run_id, code.into())?;
    db::runs::prune(conn, db::settings::history_keep_runs(conn)?)?;
    if plan.framed && text {
        ui.blank();
        ui.summary(&tally, started.elapsed());
    }
    if !text {
        let run = db::runs::by_id(conn, run_id)?.map(|run| ui_json::run(&run));
        let results: Vec<_> = db::runs::results_for_run(conn, run_id)?
            .iter()
            .map(ui_json::result)
            .collect();
        print_json(
            ui,
            &json!({
                "run": run,
                "interrupted": interrupted,
                "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "tally": ui_json::tally(&tally),
                "results": results,
            }),
        )?;
    }
    if interrupted {
        let skipped = total - finished.len();
        if skipped > 0 {
            ui.warn(format_args!(
                "interrupted; {} not started",
                repositories(skipped)
            ));
        } else {
            ui.warn("interrupted");
        }
    }
    Ok(ExitCode::from(code))
}

/// The command line as stored in `runs.args`, without the program name.
fn invocation() -> String {
    std::env::args_os()
        .skip(1)
        .map(|arg| {
            let arg = arg.to_string_lossy();
            if arg.is_empty() || arg.contains(char::is_whitespace) {
                format!("\"{arg}\"")
            } else {
                arg.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

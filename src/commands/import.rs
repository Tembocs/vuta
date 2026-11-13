//! `vuta import <file>`: add the roots, repositories, tags, hooks and
//! settings of a file written by `vuta export`.
//!
//! Import only adds. A repository whose name is already registered is
//! skipped, a local setting is never overwritten, and nothing is cloned;
//! `vuta sync` does that afterwards. The whole import is one atomic unit.
//!
//! Each of the file's roots is resolved to a root on this machine:
//!
//! 1. `--map-root NAME=PATH` says where NAME lives here. A local root with
//!    that path is used as it is; otherwise NAME is created at PATH.
//! 2. Without a mapping, a local root with the same name is used, then a
//!    local root with the same path.
//! 3. Otherwise the root is created at its exported path, which must be
//!    absolute on this machine (a Windows path is not, on Linux).
//!
//! Hooks are commands that run on every sync, so every hook the import
//! adds is listed, and `--no-hooks` leaves them out.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Ctx, absolute, check_label, print_json, rel_path, repositories, sync_options};
use crate::Error;
use crate::cli::{Format, ImportArgs};
use crate::db;
use crate::db::repos::{Changes, NewRepo, Selection};
use crate::model::{Platform, Root, RunWhen, Strategy, VcsKind, count};
use crate::transfer::{Registry, RepoRecord};
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, json as ui_json};
use crate::vcs::remote;

/// Where one of the file's roots goes.
#[derive(Debug, Clone)]
enum RootTarget {
    /// A root that already exists here.
    Existing(Root),
    /// A root to create.
    Create { name: String, path: PathBuf },
}

impl RootTarget {
    /// Identifies the target, so two repositories of the file that land at
    /// the same place can be told apart.
    fn key(&self) -> String {
        match self {
            RootTarget::Existing(root) => format!("id:{}", root.id),
            RootTarget::Create { name, .. } => format!("new:{}", name.to_lowercase()),
        }
    }

    fn name(&self) -> &str {
        match self {
            RootTarget::Existing(root) => &root.name,
            RootTarget::Create { name, .. } => name,
        }
    }
}

/// A hook, validated.
#[derive(Debug, Clone)]
struct HookPlan {
    command: String,
    platform: Platform,
    run_when: RunWhen,
}

#[derive(Debug, Clone)]
enum RepoAction {
    Add { hooks: Vec<HookPlan> },
    Skip(String),
}

#[derive(Debug, Clone)]
struct RepoPlan {
    record: RepoRecord,
    /// The file's root name, lowercased; a key into `Plan::roots`.
    root: String,
    rel_path: String,
    strategy: Strategy,
    action: RepoAction,
}

#[derive(Debug, Clone)]
enum SettingAction {
    Set,
    /// The local value differs and is kept.
    Keep(String),
    /// This Vuta does not know the key.
    Unknown,
}

#[derive(Debug, Clone)]
struct Plan {
    /// The file's roots by lowercased name, in file order.
    roots: Vec<(String, RootTarget)>,
    /// Make this file root the default: set when no local root is default.
    make_default: Option<String>,
    repos: Vec<RepoPlan>,
    settings: Vec<(String, String, SettingAction)>,
    /// Disabled hooks of added repositories, which are not imported.
    disabled_hooks: usize,
}

pub fn run(ctx: &mut Ctx, args: &ImportArgs) -> Result<ExitCode, Error> {
    let registry = read(&args.file)?;
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let mappings = parse_mappings(&args.map_roots)?;
    let plan = plan(&conn, &registry, &mappings, !args.no_hooks)?;
    let text = args.format == Format::Text;
    if text {
        show(ui, &plan);
    }
    if !args.dry_run {
        db::atomic(&conn, |conn| apply(conn, &plan))?;
    }
    if text {
        summarize(ui, &plan, args.dry_run);
    } else {
        print_json(ui, &plan_json(&plan, args.dry_run))?;
    }
    notes(ui, &plan, args.dry_run);
    Ok(ExitCode::SUCCESS)
}

fn read(file: &Path) -> Result<Registry, Error> {
    let mut bytes = Vec::new();
    if file == Path::new("-") {
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|err| anyhow!("cannot read stdin: {err}"))?;
    } else {
        bytes =
            std::fs::read(file).map_err(|err| anyhow!("cannot read {}: {err}", file.display()))?;
    }
    Registry::parse(&bytes).map_err(|err| anyhow!("{}: {err:#}", file.display()).into())
}

/// `NAME=PATH` pairs, with PATH made absolute here.
fn parse_mappings(pairs: &[String]) -> Result<Vec<(String, PathBuf)>, Error> {
    pairs
        .iter()
        .map(|pair| {
            let (name, path) = pair
                .split_once('=')
                .filter(|(n, p)| !n.is_empty() && !p.is_empty())
                .ok_or_else(|| anyhow!("--map-root takes NAME=PATH, not '{pair}'"))?;
            Ok((name.to_owned(), absolute(Path::new(path))?))
        })
        .collect()
}

/// Validate the file and decide what to do with each part of it.
fn plan(
    conn: &Connection,
    registry: &Registry,
    mappings: &[(String, PathBuf)],
    with_hooks: bool,
) -> Result<Plan, Error> {
    // Roots.
    let local_roots = db::roots::all(conn)?;
    let mut seen = HashSet::new();
    for root in &registry.roots {
        check_label("root name", &root.name)?;
        if !seen.insert(root.name.to_lowercase()) {
            return Err(anyhow!("the file names root '{}' twice", root.name).into());
        }
    }
    for (name, _) in mappings {
        if !seen.contains(&name.to_lowercase()) {
            return Err(anyhow!("--map-root {name}: the file has no root named '{name}'").into());
        }
    }
    let mut roots = Vec::new();
    let mut created_paths: HashMap<PathBuf, String> = HashMap::new();
    for root in &registry.roots {
        let mapped = mappings
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(&root.name))
            .map(|(_, p)| p.clone());
        let by_name = local_roots
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(&root.name));
        let target = match mapped {
            Some(path) => match local_roots.iter().find(|r| r.path == path) {
                Some(local) => RootTarget::Existing(local.clone()),
                None => match by_name {
                    Some(local) => {
                        return Err(anyhow!(
                            "--map-root {}: a root named '{}' already exists here at {}; \
                             map to that path, or move it with: vuta root move {} {}",
                            root.name,
                            local.name,
                            local.path.display(),
                            local.name,
                            path.display()
                        )
                        .into());
                    }
                    None => RootTarget::Create {
                        name: root.name.clone(),
                        path,
                    },
                },
            },
            None => {
                let exported = PathBuf::from(&root.path);
                if let Some(local) = by_name {
                    RootTarget::Existing(local.clone())
                } else if let Some(local) = local_roots.iter().find(|r| r.path == exported) {
                    RootTarget::Existing(local.clone())
                } else if exported.is_absolute() {
                    RootTarget::Create {
                        name: root.name.clone(),
                        path: exported,
                    }
                } else {
                    return Err(anyhow!(
                        "root '{}' is at {} on the machine that exported it, which is not an \
                         absolute path here; say where it goes with: --map-root {}=<path>",
                        root.name,
                        root.path,
                        root.name
                    )
                    .into());
                }
            }
        };
        if let RootTarget::Create { name, path } = &target
            && let Some(other) = created_paths.insert(path.clone(), name.clone())
        {
            return Err(anyhow!(
                "roots '{other}' and '{name}' would both be created at {}; map one elsewhere \
                 with --map-root",
                path.display()
            )
            .into());
        }
        roots.push((root.name.to_lowercase(), target));
    }
    let make_default = if db::roots::default(conn)?.is_none() {
        registry
            .roots
            .iter()
            .find(|r| r.default)
            .map(|r| r.name.to_lowercase())
    } else {
        None
    };

    // Repositories.
    let local = db::repos::select(
        conn,
        &Selection {
            include_disabled: true,
            ..Default::default()
        },
    )?;
    let local_by_name: HashMap<String, &crate::model::RepoEntry> = local
        .iter()
        .map(|e| (e.repo.name.to_lowercase(), e))
        .collect();
    // Who occupies each (root target, path), locally or earlier in the file.
    let mut taken: HashMap<(String, String), String> = local
        .iter()
        .map(|e| {
            (
                (format!("id:{}", e.repo.root_id), e.repo.rel_path.clone()),
                e.repo.name.clone(),
            )
        })
        .collect();
    let mut names = HashSet::new();
    let mut repos = Vec::new();
    let mut disabled_hooks = 0;
    for record in &registry.repos {
        check_label("repository name", &record.name)?;
        if !names.insert(record.name.to_lowercase()) {
            return Err(anyhow!("the file names repository '{}' twice", record.name).into());
        }
        let root_key = record.root.to_lowercase();
        let Some((_, target)) = roots.iter().find(|(k, _)| *k == root_key) else {
            return Err(anyhow!(
                "repository '{}' is under root '{}', which the file does not list",
                record.name,
                record.root
            )
            .into());
        };
        let rel =
            rel_path(&record.path).map_err(|err| anyhow!("repository '{}': {err}", record.name))?;
        for tag in &record.tags {
            check_label("tag", tag)
                .map_err(|err| anyhow!("repository '{}': {err}", record.name))?;
        }
        if record.clone_depth == Some(0) {
            return Err(anyhow!(
                "repository '{}': clone_depth must be at least 1",
                record.name
            )
            .into());
        }
        let strategy: Strategy = record
            .strategy
            .parse()
            .map_err(|err| anyhow!("repository '{}': {err}", record.name))?;
        let mut hooks = Vec::new();
        for hook in &record.hooks {
            let parsed =
                parse_hook(hook).map_err(|err| anyhow!("repository '{}': {err}", record.name))?;
            if !hook.enabled {
                disabled_hooks += 1;
            } else if with_hooks {
                hooks.push(parsed);
            }
        }

        let location = (target.key(), rel.clone());
        let action = if let Some(existing) = local_by_name.get(&record.name.to_lowercase()) {
            if remote::same_repo(&existing.repo.url, &record.url) {
                RepoAction::Skip("already registered".to_owned())
            } else {
                RepoAction::Skip(format!(
                    "already registered, with another URL: {}",
                    existing.repo.url
                ))
            }
        } else if let Some(other) = taken.get(&location) {
            RepoAction::Skip(format!("'{other}' is already at {}:{rel}", target.name()))
        } else {
            taken.insert(location, record.name.clone());
            RepoAction::Add { hooks }
        };
        repos.push(RepoPlan {
            record: record.clone(),
            root: root_key,
            rel_path: rel,
            strategy,
            action,
        });
    }

    // Settings: never overwrite a local value.
    let mut settings = Vec::new();
    for (key, value) in &registry.settings {
        let action = match db::settings::get(conn, key) {
            Ok(local) if local.is_default => SettingAction::Set,
            Ok(local) if local.value == *value => continue,
            Ok(local) => SettingAction::Keep(local.value),
            Err(db::DbError::UnknownSetting(_)) => SettingAction::Unknown,
            Err(err) => return Err(err.into()),
        };
        settings.push((key.clone(), value.clone(), action));
    }

    Ok(Plan {
        roots,
        make_default,
        repos,
        settings,
        disabled_hooks,
    })
}

fn parse_hook(hook: &crate::transfer::HookRecord) -> anyhow::Result<HookPlan> {
    if hook.command.trim().is_empty() {
        bail!("a hook has an empty command");
    }
    Ok(HookPlan {
        command: hook.command.clone(),
        platform: hook.platform.parse()?,
        run_when: hook.when.parse()?,
    })
}

/// Make the plan's changes. The caller wraps this in `db::atomic`.
fn apply(conn: &Connection, plan: &Plan) -> Result<(), Error> {
    let mut root_ids = HashMap::new();
    for (key, target) in &plan.roots {
        let id = match target {
            RootTarget::Existing(root) => root.id,
            RootTarget::Create { name, path } => db::roots::add(conn, name, path, false)?.id,
        };
        root_ids.insert(key.clone(), id);
    }
    if let Some(key) = &plan.make_default {
        db::roots::set_default(conn, root_ids[key])?;
    }
    for repo in &plan.repos {
        let RepoAction::Add { hooks } = &repo.action else {
            continue;
        };
        let record = &repo.record;
        let added = db::repos::insert(
            conn,
            &NewRepo {
                name: &record.name,
                root_id: root_ids[&repo.root],
                rel_path: &repo.rel_path,
                vcs: VcsKind::Git,
                url: &record.url,
                branch: record.branch.as_deref(),
                clone_filter: record.clone_filter.as_deref(),
                clone_depth: record.clone_depth,
                allow_dirty: record.allow_dirty,
                submodules: record.submodules,
                strategy: repo.strategy,
                tags: &record.tags,
            },
        )?;
        if !record.enabled {
            db::repos::update(
                conn,
                added.id,
                &Changes {
                    enabled: Some(false),
                    ..Default::default()
                },
            )?;
        }
        for hook in hooks {
            db::hooks::add(conn, added.id, &hook.command, hook.platform, hook.run_when)?;
        }
    }
    for (key, value, action) in &plan.settings {
        if matches!(action, SettingAction::Set) {
            db::settings::set(conn, key, value)?;
        }
    }
    Ok(())
}

/// One table row per root, repository, hook and setting.
fn show(ui: &crate::ui::Ui, plan: &Plan) {
    let mut table = Table::new(["KIND", "NAME", "ACTION", "DETAIL"]);
    let row = |kind: &str, name: &str, action: &str, style, detail: String| {
        vec![
            Cell::new(kind, THEME.muted),
            Cell::new(name, THEME.name),
            Cell::new(action, style),
            Cell::plain(detail),
        ]
    };
    for (key, target) in &plan.roots {
        let default = if plan.make_default.as_ref() == Some(key) {
            ", default"
        } else {
            ""
        };
        table.push(match target {
            RootTarget::Existing(root) => row(
                "root",
                &root.name,
                "use",
                THEME.muted,
                format!("{}{default}", root.path.display()),
            ),
            RootTarget::Create { name, path } => {
                let missing = if path.is_dir() {
                    ""
                } else {
                    " (does not exist yet)"
                };
                row(
                    "root",
                    name,
                    "create",
                    THEME.new,
                    format!("{}{missing}{default}", path.display()),
                )
            }
        });
    }
    for repo in &plan.repos {
        let record = &repo.record;
        match &repo.action {
            RepoAction::Add { hooks } => {
                let target = plan
                    .roots
                    .iter()
                    .find(|(k, _)| *k == repo.root)
                    .map_or("?", |(_, t)| t.name());
                let mut detail = format!("{target}:{}", repo.rel_path);
                if !record.tags.is_empty() {
                    detail.push_str(&format!("  tags: {}", record.tags.join(",")));
                }
                if !record.enabled {
                    detail.push_str("  (disabled)");
                }
                let options = sync_options(record.allow_dirty, record.submodules, repo.strategy);
                if options != [Strategy::FfOnly.as_str()] {
                    detail.push_str(&format!("  sync: {}", options.join(", ")));
                }
                table.push(row("repo", &record.name, "add", THEME.new, detail));
                for hook in hooks {
                    table.push(row(
                        "hook",
                        &record.name,
                        "add",
                        THEME.warn,
                        format!("[{}, {}] {}", hook.platform, hook.run_when, hook.command),
                    ));
                }
            }
            RepoAction::Skip(reason) => {
                table.push(row(
                    "repo",
                    &record.name,
                    "skip",
                    THEME.warn,
                    reason.clone(),
                ));
            }
        }
    }
    for (key, value, action) in &plan.settings {
        table.push(match action {
            SettingAction::Set => row("setting", key, "set", THEME.ok, value.clone()),
            SettingAction::Keep(local) => row(
                "setting",
                key,
                "keep",
                THEME.muted,
                format!("local value {local} kept; the file has {value}"),
            ),
            SettingAction::Unknown => row(
                "setting",
                key,
                "skip",
                THEME.warn,
                "unknown to this vuta".to_owned(),
            ),
        });
    }
    if !table.is_empty() {
        ui.table(&table);
        ui.blank();
    }
}

fn summarize(ui: &crate::ui::Ui, plan: &Plan, dry_run: bool) {
    let added: Vec<&RepoPlan> = plan
        .repos
        .iter()
        .filter(|r| matches!(r.action, RepoAction::Add { .. }))
        .collect();
    let hooks: usize = added
        .iter()
        .map(|r| match &r.action {
            RepoAction::Add { hooks } => hooks.len(),
            RepoAction::Skip(_) => 0,
        })
        .sum();
    let created = plan
        .roots
        .iter()
        .filter(|(_, t)| matches!(t, RootTarget::Create { .. }))
        .count();
    let skipped = plan.repos.len() - added.len();
    let separator = format!(" {} ", ui.glyphs().separator);
    let mut line = Styled::new();
    if dry_run {
        line.push("dry run, nothing changed: ", THEME.warn);
    }
    line.push(
        &format!("{} added", repositories(added.len())),
        if added.is_empty() {
            THEME.muted
        } else {
            THEME.new
        },
    )
    .plain(&separator)
    .push(
        &format!("{skipped} skipped"),
        if skipped == 0 {
            THEME.muted
        } else {
            THEME.warn
        },
    )
    .plain(&separator)
    .push(&count(hooks as u32, "hook"), THEME.muted)
    .plain(&separator)
    .push(
        &format!("{} created", count(created as u32, "root")),
        THEME.muted,
    );
    ui.result(line);
}

/// Warnings and hints on stderr, after the plan in either format.
fn notes(ui: &crate::ui::Ui, plan: &Plan, dry_run: bool) {
    if plan.disabled_hooks > 0 {
        ui.warn(format_args!(
            "{} left out: disabled hooks are not imported",
            count(plan.disabled_hooks as u32, "disabled hook")
        ));
    }
    let added = plan
        .repos
        .iter()
        .any(|r| matches!(r.action, RepoAction::Add { .. }));
    if added && !dry_run {
        ui.progress("nothing was cloned; clone the new repositories with: vuta sync");
    }
}

/// The plan as `--format json` prints it: every root, repository and
/// setting of the file with what the import does with it.
fn plan_json(plan: &Plan, dry_run: bool) -> Value {
    let target_name = |key: &str| {
        plan.roots
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, t)| t.name())
    };
    let roots: Vec<Value> = plan
        .roots
        .iter()
        .map(|(key, target)| {
            let (action, path) = match target {
                RootTarget::Existing(root) => ("use", &root.path),
                RootTarget::Create { path, .. } => ("create", path),
            };
            json!({
                "name": target.name(),
                "path": ui_json::path_text(path),
                "action": action,
                "becomes_default": plan.make_default.as_deref() == Some(key.as_str()),
            })
        })
        .collect();
    let repos: Vec<Value> = plan
        .repos
        .iter()
        .map(|repo| {
            let record = &repo.record;
            let (action, reason, hooks) = match &repo.action {
                RepoAction::Add { hooks } => ("add", None, hooks.as_slice()),
                RepoAction::Skip(reason) => ("skip", Some(reason), &[][..]),
            };
            let hooks: Vec<Value> = hooks
                .iter()
                .map(|hook| {
                    json!({
                        "command": hook.command,
                        "platform": hook.platform.as_str(),
                        "when": hook.run_when.as_str(),
                    })
                })
                .collect();
            json!({
                "name": record.name,
                "root": target_name(&repo.root),
                "rel_path": repo.rel_path,
                "url": record.url,
                "branch": record.branch,
                "clone_filter": record.clone_filter,
                "clone_depth": record.clone_depth,
                "enabled": record.enabled,
                "allow_dirty": record.allow_dirty,
                "submodules": record.submodules,
                "strategy": repo.strategy.as_str(),
                "tags": record.tags,
                "action": action,
                "reason": reason,
                "hooks": hooks,
            })
        })
        .collect();
    let settings: Vec<Value> = plan
        .settings
        .iter()
        .map(|(key, value, action)| {
            let (action, local) = match action {
                SettingAction::Set => ("set", None),
                SettingAction::Keep(local) => ("keep", Some(local)),
                SettingAction::Unknown => ("skip", None),
            };
            json!({ "key": key, "value": value, "action": action, "local_value": local })
        })
        .collect();
    json!({
        "dry_run": dry_run,
        "roots": roots,
        "repos": repos,
        "settings": settings,
        "disabled_hooks": plan.disabled_hooks,
    })
}

//! `vuta export [-o FILE]`: the registry as a file `vuta import` reads.

use std::collections::HashMap;
use std::fs;
use std::process::ExitCode;

use anyhow::anyhow;

use super::{Ctx, now_unix, repositories};
use crate::Error;
use crate::cli::ExportArgs;
use crate::db;
use crate::db::repos::Selection;
use crate::model::{Hook, count};
use crate::transfer::{FORMAT, HookRecord, Registry, RepoRecord, RootRecord, VERSION};

pub fn run(ctx: &mut Ctx, args: &ExportArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let roots = db::roots::all(&conn)?;
    let entries = db::repos::select(
        &conn,
        &Selection {
            include_disabled: true,
            ..Default::default()
        },
    )?;
    let mut hooks: HashMap<String, Vec<Hook>> = HashMap::new();
    for (repo, hook) in db::hooks::all(&conn)? {
        hooks.entry(repo).or_default().push(hook);
    }
    let settings = db::settings::list(&conn)?
        .into_iter()
        .filter(|s| !s.is_default)
        .map(|s| (s.key.to_owned(), s.value))
        .collect();

    let hook_count: usize = hooks.values().map(Vec::len).sum();
    let registry = Registry {
        format: FORMAT.to_owned(),
        version: VERSION,
        vuta_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        exported_at: Some(now_unix()),
        roots: roots
            .iter()
            .map(|root| RootRecord {
                name: root.name.clone(),
                path: root.path.to_string_lossy().into_owned(),
                default: root.is_default,
            })
            .collect(),
        settings,
        repos: entries
            .into_iter()
            .map(|entry| {
                let repo = entry.repo;
                RepoRecord {
                    hooks: hooks
                        .remove(&repo.name)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|hook| HookRecord {
                            command: hook.command,
                            platform: hook.platform.as_str().to_owned(),
                            when: hook.run_when.as_str().to_owned(),
                            enabled: hook.enabled,
                        })
                        .collect(),
                    name: repo.name,
                    root: entry.root_name,
                    path: repo.rel_path,
                    url: repo.url,
                    branch: repo.branch,
                    clone_filter: repo.clone_filter,
                    clone_depth: repo.clone_depth,
                    enabled: repo.enabled,
                    tags: entry.tags,
                }
            })
            .collect(),
    };

    let text = registry.to_json();
    match &args.output {
        Some(path) => {
            fs::write(path, &text)
                .map_err(|err| anyhow!("cannot write {}: {err}", path.display()))?;
            ui.progress(format_args!(
                "exported {}, {} and {} to {}",
                repositories(registry.repos.len()),
                count(hook_count as u32, "hook"),
                count(registry.roots.len() as u32, "root"),
                path.display()
            ));
        }
        None => ui
            .verbatim(&text)
            .map_err(|err| anyhow!("cannot write the registry: {err}"))?,
    }
    Ok(ExitCode::SUCCESS)
}

//! `vuta scan [dir]`: register checkouts that already exist on disk.
//!
//! The walk stops at each checkout it finds, so submodules and vendored
//! repositories inside one are not registered separately. Hooks are never
//! read from a repository's contents.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::anyhow;
use serde_json::{Value, json};

use super::{
    Ctx, absolute, check_label, current_dir, offer_first_root, print_json, root_by_name,
    root_or_default, root_or_first, tracer,
};
use crate::Error;
use crate::cli::{Format, ScanArgs};
use crate::db;
use crate::db::repos::{NewRepo, Selection};
use crate::model::{Root, Strategy, VcsKind};
use crate::paths;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, json as ui_json};
use crate::vcs::{self, Vcs, remote};

/// What the scan decided for one checkout.
enum Found {
    Add {
        name: String,
        root: Root,
        rel_path: String,
        url: String,
        tags: Vec<String>,
    },
    Known,
    Skip {
        /// The checkout's absolute path.
        path: PathBuf,
        /// How the text output names it: `root:rel/path` when known.
        what: String,
        reason: String,
    },
}

pub fn run(ctx: &mut Ctx, args: &ScanArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let only_root = match &args.root {
        Some(name) => Some(root_by_name(&conn, name)?),
        None => None,
    };
    // With no roots yet, a person at a terminal may make the directory
    // being scanned the first one. A dry run changes nothing, so it only
    // says how to add one.
    let dir = match (&args.dir, &only_root) {
        (Some(dir), _) => absolute(dir)?,
        (None, Some(root)) => root.path.clone(),
        (None, None) if args.dry_run => root_or_default(&conn, None)?.path,
        (None, None) => root_or_first(ui, &conn, None, current_dir)?.path,
    };
    if !dir.is_dir() {
        return Err(anyhow!("{} is not a directory", dir.display()).into());
    }
    if only_root.is_none() && args.dir.is_some() && !args.dry_run {
        offer_first_root(ui, &conn, || Ok(dir.clone()))?;
    }
    let roots = match &only_root {
        Some(root) => vec![root.clone()],
        None => db::roots::all(&conn)?,
    };

    let vcs = vcs::backend(VcsKind::Git, tracer(ui));
    let mut checkouts = Vec::new();
    let mut unreadable = Vec::new();
    walk(
        vcs.as_ref(),
        &dir,
        args.max_depth,
        &mut checkouts,
        &mut unreadable,
    );

    // What is registered already: locations, and names (case-insensitive).
    let known = db::repos::select(
        &conn,
        &Selection {
            include_disabled: true,
            ..Default::default()
        },
    )?;
    let known_at: HashSet<(i64, String)> = known
        .iter()
        .map(|e| (e.repo.root_id, e.repo.rel_path.clone()))
        .collect();
    let mut taken: HashMap<String, String> = known
        .iter()
        .map(|e| (e.repo.name.to_lowercase(), e.repo.name.clone()))
        .collect();

    let mut table = Table::new(["RESULT", "NAME", "PATH", "DETAIL"]);
    let (mut added_json, mut skipped_json) = (Vec::new(), Vec::new());
    let (mut added, mut skipped, mut already) = (0, 0, 0);
    let skip_json =
        |path: &Path, reason: &str| json!({ "path": ui_json::path_text(path), "reason": reason });
    for (path, reason) in unreadable {
        skipped += 1;
        table.push(skip_row(&path.display().to_string(), &reason));
        skipped_json.push(skip_json(&path, &reason));
    }
    for path in checkouts {
        let found = classify(
            vcs.as_ref(),
            &path,
            &roots,
            only_root.is_some(),
            args.tag_from_parent,
            &known_at,
            &taken,
        );
        match found {
            Found::Known => already += 1,
            Found::Skip { path, what, reason } => {
                skipped += 1;
                table.push(skip_row(&what, &reason));
                skipped_json.push(skip_json(&path, &reason));
            }
            Found::Add {
                name,
                root,
                rel_path,
                url,
                tags,
            } => {
                if !args.dry_run {
                    db::repos::insert(
                        &conn,
                        &NewRepo {
                            name: &name,
                            root_id: root.id,
                            rel_path: &rel_path,
                            vcs: VcsKind::Git,
                            url: &url,
                            branch: None,
                            clone_filter: None,
                            clone_depth: None,
                            allow_dirty: false,
                            submodules: false,
                            strategy: Strategy::FfOnly,
                            tags: &tags,
                        },
                    )?;
                }
                added += 1;
                taken.insert(name.to_lowercase(), name.clone());
                added_json.push(json!({
                    "name": name,
                    "root": root.name,
                    "rel_path": rel_path,
                    "path": ui_json::path_text(&path),
                    "url": url,
                    "tags": tags,
                }));
                let mut detail = url;
                if !tags.is_empty() {
                    detail = format!("{detail}  tags: {}", tags.join(","));
                }
                table.push(vec![
                    Cell::new(if args.dry_run { "would add" } else { "added" }, THEME.new),
                    Cell::new(name, THEME.name),
                    Cell::new(format!("{}:{rel_path}", root.name), THEME.muted),
                    Cell::new(detail, THEME.muted),
                ]);
            }
        }
    }

    if args.format == Format::Json {
        print_json(
            ui,
            &json!({
                "dir": ui_json::path_text(&dir),
                "dry_run": args.dry_run,
                "added": Value::Array(added_json),
                "already_registered": already,
                "skipped": Value::Array(skipped_json),
            }),
        )?;
        return Ok(ExitCode::SUCCESS);
    }
    if !table.is_empty() {
        ui.table(&table);
        ui.blank();
    }
    let mut summary = Styled::new();
    let added_word = if args.dry_run {
        "would be added"
    } else {
        "added"
    };
    summary
        .push(
            &format!("{added} {added_word}"),
            if added > 0 { THEME.new } else { THEME.muted },
        )
        .plain(&format!(" {} ", ui.glyphs().separator))
        .push(&format!("{already} already registered"), THEME.muted)
        .plain(&format!(" {} ", ui.glyphs().separator))
        .push(
            &format!("{skipped} skipped"),
            if skipped > 0 { THEME.warn } else { THEME.muted },
        )
        .plain("   ")
        .push(&format!("in {}", dir.display()), THEME.muted);
    ui.result(summary);
    Ok(ExitCode::SUCCESS)
}

fn skip_row(what: &str, reason: &str) -> Vec<Cell> {
    vec![
        Cell::new("skipped", THEME.warn),
        Cell::plain("-"),
        Cell::new(what, THEME.muted),
        Cell::plain(reason),
    ]
}

/// Collect checkouts under `dir`, at most `depth` levels below it, in name
/// order. A checkout is not searched further. Unreadable directories are
/// reported rather than fatal.
fn walk(
    vcs: &dyn Vcs,
    dir: &Path,
    depth: u32,
    checkouts: &mut Vec<PathBuf>,
    unreadable: &mut Vec<(PathBuf, String)>,
) {
    if looks_like_checkout(dir) && vcs.is_checkout(dir) {
        checkouts.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            unreadable.push((dir.to_path_buf(), format!("cannot read: {err}")));
            return;
        }
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        // `file_type` does not follow symlinks, so links cannot loop.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name != ".git" && !name.ends_with(".vuta-partial")
        })
        .collect();
    children.sort();
    for child in children {
        walk(vcs, &child, depth - 1, checkouts, unreadable);
    }
}

/// A cheap test before asking git: a `.git` directory, or the `.git` file
/// of a worktree or submodule.
fn looks_like_checkout(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// The root whose path contains `path`, preferring the deepest, and the
/// path relative to it in stored form.
fn locate<'a>(path: &Path, roots: &'a [Root]) -> Option<(&'a Root, Result<String, String>)> {
    roots
        .iter()
        .filter_map(|root| path.strip_prefix(&root.path).ok().map(|rel| (root, rel)))
        .max_by_key(|(root, _)| root.path.components().count())
        .map(|(root, rel)| {
            let mut segments = Vec::new();
            for component in rel.components() {
                match component {
                    Component::Normal(s) => match s.to_str() {
                        Some(s) => segments.push(s),
                        None => return (root, Err("path is not valid UTF-8".to_owned())),
                    },
                    _ => return (root, Err("unexpected path component".to_owned())),
                }
            }
            if segments.is_empty() {
                return (root, Err("this is the root directory itself".to_owned()));
            }
            let rel = paths::normalize_rel_path(&segments.join("/")).map_err(|e| e.to_string());
            (root, rel)
        })
}

#[allow(clippy::too_many_arguments)]
fn classify(
    vcs: &dyn Vcs,
    path: &Path,
    roots: &[Root],
    one_root: bool,
    tag_from_parent: bool,
    known_at: &HashSet<(i64, String)>,
    taken: &HashMap<String, String>,
) -> Found {
    let what = path.display().to_string();
    let skip = |reason: String| Found::Skip {
        path: path.to_path_buf(),
        what: what.clone(),
        reason,
    };
    let (root, rel_path) = match locate(path, roots) {
        Some((root, Ok(rel))) => (root, rel),
        Some((_, Err(reason))) => return skip(reason),
        None if one_root => return skip(format!("outside root {}", roots[0].name)),
        None => return skip("outside every root".to_owned()),
    };
    if known_at.contains(&(root.id, rel_path.clone())) {
        return Found::Known;
    }
    let what = format!("{}:{rel_path}", root.name);
    let skip = |reason: String| Found::Skip {
        path: path.to_path_buf(),
        what: what.clone(),
        reason,
    };
    let url = match vcs.remote_url(path) {
        Ok(Some(url)) => url,
        Ok(None) => return skip("no origin remote".to_owned()),
        Err(err) => return skip(format!("cannot read origin: {err:#}")),
    };
    let name = remote::infer_name(&url)
        .or_else(|| rel_path.rsplit('/').next().map(str::to_owned))
        .unwrap_or_default();
    if check_label("name", &name).is_err() {
        return skip(format!(
            "cannot use '{name}' as a name; register it with: vuta add {url} {rel_path} --name <name>"
        ));
    }
    if let Some(existing) = taken.get(&name.to_lowercase()) {
        return skip(format!(
            "the name '{existing}' is taken; register it with: \
             vuta add {url} {rel_path} --root {} --name <other> --no-clone",
            root.name
        ));
    }
    let mut tags = Vec::new();
    if tag_from_parent && let Some((parent, _)) = rel_path.rsplit_once('/') {
        let tag = parent.rsplit('/').next().unwrap_or(parent);
        if check_label("tag", tag).is_ok() {
            tags.push(tag.to_owned());
        }
    }
    Found::Add {
        name,
        root: root.clone(),
        rel_path,
        url,
        tags,
    }
}

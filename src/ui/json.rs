//! The JSON shapes of `--format json`, defined in one place.
//!
//! Every command prints exactly one JSON value on stdout: an array for
//! listings, an object for `show`, `sync`, `scan` and `import`. The shapes
//! that only one command prints are built in that command. Fields follow
//! these rules, which
//! docs/ARCHITECTURE.md section 9 repeats for script authors:
//!
//! - Keys are `snake_case`. A field that has no value is `null`, never
//!   missing.
//! - Revisions are full hashes. Times are Unix seconds, durations
//!   milliseconds.
//! - Enumerations use the strings stored in the database, such as
//!   `up_to_date` or `fast_forward`.
//! - New fields may appear; existing ones keep their name and meaning.

use std::path::Path;

use serde_json::{Value, json};

use crate::db::settings::{Known, Setting};
use crate::model::{Hook, RepoEntry, Root, Run, RunResult, Tally};

/// A repository as `vuta list` shows it.
pub fn repo(entry: &RepoEntry, path: &Path) -> Value {
    let repo = &entry.repo;
    json!({
        "name": repo.name,
        "root": entry.root_name,
        "rel_path": repo.rel_path,
        "path": path_text(path),
        "vcs": repo.vcs.as_str(),
        "url": repo.url,
        "branch": repo.branch,
        "clone_filter": repo.clone_filter,
        "clone_depth": repo.clone_depth,
        "enabled": repo.enabled,
        "tags": entry.tags,
        "last_rev": entry.last.rev,
        "last_outcome": entry.last.outcome.map(|o| o.as_str()),
        "last_synced_at": entry.last.at,
    })
}

/// Outcome counts, as in the summary line.
pub fn tally(tally: &Tally) -> Value {
    json!({
        "cloned": tally.cloned,
        "updated": tally.updated,
        "up_to_date": tally.up_to_date,
        "skipped": tally.skipped,
        "failed": tally.failed,
        "hooks_failed": tally.hooks_failed,
    })
}

/// One sync run, without its results.
pub fn run(run: &Run) -> Value {
    json!({
        "id": run.id,
        "started_at": run.started_at,
        "finished_at": run.finished_at,
        "exit_code": run.exit_code,
        "args": run.args,
        "vuta_version": run.vuta_version,
    })
}

/// One repository's result in a run. `commits` counts the commits brought
/// in (updated) or not applied (skipped); `message` holds a skip's
/// description or git's full error output.
pub fn result(result: &RunResult) -> Value {
    json!({
        "run_id": result.run_id,
        "repo": result.repo_name,
        "outcome": result.outcome.as_str(),
        "reason": result.reason,
        "old_rev": result.old_rev,
        "new_rev": result.new_rev,
        "commits": result.commits,
        "duration_ms": result.duration_ms,
        "hook_outcome": result.hook_outcome.map(|h| h.as_str()),
        "message": result.message,
    })
}

/// A root as `vuta root list` shows it. `exists` says whether its
/// directory is there.
pub fn root(root: &Root, repos: u32) -> Value {
    json!({
        "name": root.name,
        "path": path_text(&root.path),
        "default": root.is_default,
        "repos": repos,
        "exists": root.path.is_dir(),
    })
}

/// A hook of the repository named `repo`. `platform` and `when` use the
/// same strings as `vuta export`.
pub fn hook(repo: &str, hook: &Hook) -> Value {
    json!({
        "id": hook.id,
        "repo": repo,
        "command": hook.command,
        "platform": hook.platform.as_str(),
        "when": hook.run_when.as_str(),
        "enabled": hook.enabled,
    })
}

/// A setting with its current value and its default.
pub fn setting(setting: &Setting, known: &Known) -> Value {
    json!({
        "key": setting.key,
        "value": setting.value,
        "default": known.default,
        "is_default": setting.is_default,
        "description": known.help,
    })
}

pub fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HookOutcome, LastSync, OutcomeKind, Repo, VcsKind};

    #[test]
    fn missing_values_are_null_not_absent() {
        let entry = RepoEntry {
            repo: Repo {
                id: 1,
                name: "zed".into(),
                root_id: 1,
                rel_path: "rust/zed".into(),
                vcs: VcsKind::Git,
                url: "https://example.com/zed.git".into(),
                branch: None,
                clone_filter: None,
                clone_depth: None,
                enabled: true,
            },
            root_name: "src".into(),
            tags: vec!["rust".into()],
            last: LastSync::default(),
        };
        let value = repo(&entry, Path::new("/src/rust/zed"));
        assert_eq!(value["branch"], Value::Null);
        assert_eq!(value["last_outcome"], Value::Null);
        assert_eq!(value["tags"], json!(["rust"]));
        assert!(value.as_object().unwrap().contains_key("last_synced_at"));
    }

    #[test]
    fn results_use_stored_strings() {
        let r = RunResult {
            id: 9,
            run_id: 3,
            run_started_at: 100,
            repo_id: 1,
            repo_name: "zed".into(),
            outcome: OutcomeKind::UpToDate,
            reason: None,
            old_rev: None,
            new_rev: Some("abc".into()),
            commits: None,
            duration_ms: 1200,
            hook_outcome: Some(HookOutcome::NotRun),
            message: None,
        };
        let value = result(&r);
        assert_eq!(value["outcome"], "up_to_date");
        assert_eq!(value["hook_outcome"], "not_run");
        assert_eq!(value["duration_ms"], 1200);
    }
}

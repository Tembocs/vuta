//! M2 and M4: `vuta sync` and the clone in `vuta add`, against local bare
//! upstreams. Every scenario in docs/ARCHITECTURE.md section 11 that
//! concerns syncing has a test here.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Env, Fixture, git, line_for, stderr, stdout};
use vuta::db;
use vuta::model::{OutcomeKind, Run, RunResult};

/// Register `name` at `<root>/<name>` without cloning.
fn register(env: &Env, fixture: &Fixture, name: &str) -> PathBuf {
    env.ok(&["add", &fixture.url(), name, "--name", name, "--no-clone"]);
    env.checkout(name)
}

/// Register and clone `name` with plain `vuta add`.
fn cloned(env: &Env, fixture: &Fixture, name: &str) -> PathBuf {
    env.ok(&["add", &fixture.url(), name, "--name", name]);
    env.checkout(name)
}

fn head(path: &std::path::Path) -> String {
    git(path, &["rev-parse", "HEAD"])
}

fn runs(env: &Env) -> Vec<Run> {
    db::runs::recent(&env.conn(), 100)
        .unwrap()
        .into_iter()
        .map(|s| s.run)
        .collect()
}

/// The results of the newest run.
fn last_results(env: &Env) -> Vec<RunResult> {
    let conn = env.conn();
    let run = db::runs::recent(&conn, 1).unwrap().remove(0).run;
    db::runs::results_for_run(&conn, run.id).unwrap()
}

/// Make a local commit in a checkout.
fn local_commit(checkout: &std::path::Path) {
    fs::write(checkout.join("local.txt"), "local").unwrap();
    git(checkout, &["add", "local.txt"]);
    git(checkout, &["commit", "-q", "-m", "local"]);
}

#[test]
fn missing_path_is_cloned() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    assert!(!path.exists());

    let out = env.code(0, &["sync"]);
    let text = stdout(&out);
    assert!(line_for(&text, "alpha").contains("cloned"), "{text}");
    assert!(text.contains("1 cloned"), "{text}");
    assert!(
        stderr(&out).contains("Syncing 1 repository"),
        "{}",
        stderr(&out)
    );
    assert_eq!(head(&path), fixture.upstream_rev());

    let runs = runs(&env);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, Some(0));
    assert!(runs[0].finished_at.is_some());
    assert!(runs[0].args.contains("sync"), "{}", runs[0].args);
    let results = last_results(&env);
    assert_eq!(results[0].outcome, OutcomeKind::Cloned);
    assert_eq!(results[0].new_rev, Some(fixture.upstream_rev()));
}

#[test]
fn add_clones_and_records_a_run() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let out = env.ok(&[
        "add",
        &fixture.url(),
        "nested/dir/alpha",
        "--name",
        "alpha",
        "-t",
        "x",
    ]);
    assert!(out.contains("added alpha"), "{out}");
    assert!(line_for(&out, "alpha").contains("added"), "{out}");
    assert!(out.contains("cloned"), "{out}");
    assert_eq!(
        head(&env.checkout("nested/dir/alpha")),
        fixture.upstream_rev()
    );

    let list = env.ok(&["list"]);
    let short = &fixture.upstream_rev()[..7];
    assert!(line_for(&list, "alpha").contains(short), "{list}");
    assert_eq!(runs(&env)[0].exit_code, Some(0));
}

#[test]
fn add_clones_a_branch_and_shallow_depth() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    fixture.push_commit();
    git(&fixture.path("seed"), &["push", "-q", "origin", "HEAD:dev"]);
    // Git ignores --depth for plain local paths, so use a file:// URL.
    let path = fixture.url().replace('\\', "/");
    let url = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    env.ok(&[
        "add", &url, "--name", "alpha", "--branch", "dev", "--depth", "1",
    ]);
    let path = env.checkout("alpha");
    assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]), "dev");
    assert_eq!(git(&path, &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn upstream_commits_are_fast_forwarded() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    let old = head(&path);
    fixture.push_commit();
    let new = fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("updated"), "{text}");
    assert!(line.contains("2 commits"), "{text}");
    assert!(
        line.contains(&old[..7]) && line.contains(&new[..7]),
        "{text}"
    );
    assert_eq!(head(&path), new);

    let result = &last_results(&env)[0];
    assert_eq!(result.outcome, OutcomeKind::Updated);
    assert_eq!(result.commits, Some(2));
    assert_eq!(result.old_rev.as_deref(), Some(old.as_str()));
}

#[test]
fn nothing_new_is_up_to_date() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    cloned(&env, &fixture, "alpha");
    let text = env.ok(&["sync"]);
    assert!(line_for(&text, "alpha").contains("up to date"), "{text}");
    assert!(text.contains("1 up to date"), "{text}");
}

#[test]
fn dirty_checkout_is_skipped_and_strict_fails() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    let before = head(&path);
    fs::write(path.join("file.txt"), "local edit").unwrap();
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("skipped"), "{text}");
    assert!(line.contains("local changes; 1 commit behind"), "{text}");
    assert_eq!(head(&path), before);
    assert_eq!(
        fs::read_to_string(path.join("file.txt")).unwrap(),
        "local edit"
    );
    let result = &last_results(&env)[0];
    assert_eq!(result.reason.as_deref(), Some("dirty"));

    env.code(1, &["sync", "--strict"]);
    assert_eq!(runs(&env)[0].exit_code, Some(1));
    assert_eq!(head(&path), before);
}

#[test]
fn diverged_checkout_is_skipped() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    local_commit(&path);
    let before = head(&path);
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("skipped"), "{text}");
    assert!(line.contains("1 local commit, 1 behind"), "{text}");
    assert_eq!(head(&path), before);
    assert_eq!(last_results(&env)[0].reason.as_deref(), Some("diverged"));
}

#[test]
fn local_commits_alone_are_up_to_date() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    local_commit(&path);
    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("up to date"), "{text}");
    assert!(line.contains("1 local commit"), "{text}");
}

#[test]
fn detached_head_and_untracked_branch_are_skipped() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let detached = cloned(&env, &fixture, "alpha");
    let lonely = fixture.clone_to(&env.checkout("beta"));
    env.ok(&[
        "add",
        &fixture.url(),
        "beta",
        "--name",
        "beta",
        "--no-clone",
    ]);
    git(&detached, &["checkout", "-q", "--detach"]);
    git(&lonely, &["checkout", "-q", "-b", "lonely"]);
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(
        line.contains("skipped") && line.contains("HEAD is detached at"),
        "{text}"
    );
    let line = line_for(&text, "beta");
    assert!(
        line.contains("skipped") && line.contains("branch lonely tracks no remote branch"),
        "{text}"
    );
    let reasons: Vec<_> = last_results(&env)
        .into_iter()
        .map(|r| r.reason.unwrap())
        .collect();
    assert_eq!(reasons, ["detached", "no_upstream"]);
}

#[test]
fn other_origin_is_skipped() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    git(
        &path,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.com/other.git",
        ],
    );
    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("skipped"), "{text}");
    assert!(
        line.contains("origin is https://example.com/other.git"),
        "{text}"
    );
    assert_eq!(
        last_results(&env)[0].reason.as_deref(),
        Some("remote_mismatch")
    );
}

#[test]
fn plain_directory_fails_as_not_a_checkout() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("notes.txt"), "mine").unwrap();

    let out = env.code(1, &["sync"]);
    let text = stdout(&out);
    let line = line_for(&text, "alpha");
    assert!(line.contains("failed"), "{text}");
    assert!(line.contains("not a Git checkout"), "{text}");
    assert_eq!(fs::read_to_string(path.join("notes.txt")).unwrap(), "mine");
    assert_eq!(
        last_results(&env)[0].reason.as_deref(),
        Some("not_a_checkout")
    );
}

#[test]
fn deleted_upstream_fails_but_others_still_sync() {
    let env = Env::with_root();
    let gone = Fixture::new();
    let mut live = Fixture::new();
    cloned(&env, &gone, "alpha");
    let beta = cloned(&env, &live, "beta");
    fs::remove_dir_all(gone.path("upstream.git")).unwrap();
    let new = live.push_commit();

    let out = env.code(1, &["sync"]);
    let text = stdout(&out);
    let line = line_for(&text, "alpha");
    assert!(line.contains("failed") && line.contains("fetch:"), "{text}");
    assert!(line_for(&text, "beta").contains("updated"), "{text}");
    assert_eq!(head(&beta), new);
    assert!(
        text.contains("1 updated") && text.contains("1 failed"),
        "{text}"
    );

    // The full git output is kept and shown by `vuta show`.
    let failed = last_results(&env).remove(0);
    assert_eq!(failed.reason.as_deref(), Some("fetch"));
    assert!(!failed.message.unwrap().is_empty());
    let show = env.ok(&["show", "alpha"]);
    assert!(show.contains("Last failure"), "{show}");
    assert!(show.contains("Recent results"), "{show}");
}

#[test]
fn failed_clone_is_reported_and_leaves_nothing() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let missing = fixture.path("missing.git");
    let out = env.code(1, &["add", missing.to_str().unwrap(), "--name", "alpha"]);
    let text = stdout(&out);
    assert!(line_for(&text, "[1/1]").contains("clone:"), "{text}");
    assert!(!env.checkout("alpha").exists());
    // The repository stays registered for the next sync.
    assert!(db::repos::by_name(&env.conn(), "alpha").unwrap().is_some());
}

#[test]
fn unknown_names_exit_3() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    for args in [&["sync", "alpha", "nope"][..], &["sync", "--root", "nope"]] {
        let out = env.code(3, args);
        let err = stderr(&out);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.starts_with("error: no "), "{err}");
    }
    assert!(runs(&env).is_empty());
    assert!(!env.checkout("alpha").exists());
}

#[test]
fn a_held_lock_exits_3() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(env.db().with_file_name("vuta.lock"))
        .unwrap();
    lock.try_lock().unwrap();

    let out = env.code(3, &["sync"]);
    assert!(stderr(&out).contains("already running"), "{}", stderr(&out));
    let out = env.code(3, &["add", &fixture.url(), "--name", "beta"]);
    assert!(stderr(&out).contains("already running"), "{}", stderr(&out));
    assert!(db::repos::by_name(&env.conn(), "beta").unwrap().is_none());
    assert!(runs(&env).is_empty());

    drop(lock);
    env.ok(&["sync"]);
}

#[test]
fn fetch_only_reports_behind_and_still_clones_missing() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let alpha = cloned(&env, &fixture, "alpha");
    let beta = register(&env, &fixture, "beta");
    let before = head(&alpha);
    fixture.push_commit();
    fixture.push_commit();

    let text = env.ok(&["sync", "--fetch-only"]);
    let line = line_for(&text, "alpha");
    assert!(
        line.contains("skipped") && line.contains("2 commits behind"),
        "{text}"
    );
    assert!(line_for(&text, "beta").contains("cloned"), "{text}");
    assert_eq!(head(&alpha), before);
    assert_eq!(head(&beta), fixture.upstream_rev());
    let alpha_result = last_results(&env).remove(0);
    assert_eq!(alpha_result.reason.as_deref(), Some("fetch_only"));
    assert_eq!(alpha_result.commits, Some(2));

    // Skips from --fetch-only count under --strict like any other skip.
    env.code(1, &["sync", "--fetch-only", "--strict", "alpha"]);
}

#[test]
fn dry_run_changes_nothing() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    cloned(&env, &fixture, "alpha");
    register(&env, &fixture, "beta");
    let runs_before = runs(&env).len();

    let text = env.ok(&["sync", "--dry-run"]);
    assert!(line_for(&text, "alpha").starts_with("fetch"), "{text}");
    assert!(line_for(&text, "beta").starts_with("clone"), "{text}");
    assert!(!env.checkout("beta").exists());
    assert_eq!(runs(&env).len(), runs_before);
}

#[test]
fn selection_by_name_tag_and_root() {
    let env = Env::with_root();
    let work = env.dir.path().join("work");
    env.ok(&["root", "add", "work", work.to_str().unwrap()]);
    let fixture = Fixture::new();
    let url = fixture.url();
    env.ok(&["add", &url, "--name", "alpha", "-t", "rust", "--no-clone"]);
    env.ok(&["add", &url, "--name", "beta", "--no-clone"]);
    env.ok(&[
        "add",
        &url,
        "--name",
        "gamma",
        "--root",
        "work",
        "-t",
        "rust",
        "--no-clone",
    ]);
    env.ok(&["add", &url, "--name", "delta", "--no-clone"]);
    env.ok(&["set", "delta", "--disable"]);

    let names = |args: &[&str]| -> Vec<String> {
        env.ok(args);
        last_results(&env)
            .into_iter()
            .map(|r| r.repo_name)
            .collect()
    };
    assert_eq!(
        names(&["sync", "--tag", "RUST", "--root", "src"]),
        ["alpha"]
    );
    assert_eq!(names(&["sync", "--tag", "rust"]), ["alpha", "gamma"]);
    assert_eq!(names(&["sync", "BETA"]), ["beta"]);
    // Disabled repositories are left out unless named, and then skipped.
    assert_eq!(names(&["sync"]), ["alpha", "beta", "gamma"]);
    let text = env.ok(&["sync", "delta"]);
    assert!(line_for(&text, "delta").contains("disabled"), "{text}");
    assert!(!env.checkout("delta").exists());

    let runs_before = runs(&env).len();
    let out = env.code(0, &["sync", "--tag", "none"]);
    assert!(stderr(&out).contains("nothing to sync"), "{}", stderr(&out));
    assert_eq!(runs(&env).len(), runs_before);
}

#[test]
fn jobs_come_from_the_flag_or_the_setting() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    for name in ["alpha", "beta", "gamma"] {
        register(&env, &fixture, name);
    }
    let out = env.code(0, &["sync", "-j", "2"]);
    assert!(
        stderr(&out).contains("3 repositories, 2 at a time"),
        "{}",
        stderr(&out)
    );
    env.ok(&["config", "set", "sync.jobs", "1"]);
    let out = env.code(0, &["sync"]);
    assert!(stderr(&out).contains("1 at a time"), "{}", stderr(&out));
    // More workers than repositories are capped.
    let out = env.code(0, &["sync", "--jobs", "64"]);
    assert!(stderr(&out).contains("3 at a time"), "{}", stderr(&out));
    env.code(2, &["sync", "-j", "0"]);
}

#[test]
fn history_is_pruned_to_keep_runs() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    env.ok(&["config", "set", "history.keep_runs", "2"]);
    for _ in 0..4 {
        env.ok(&["sync"]);
    }
    let runs = runs(&env);
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r.exit_code == Some(0)));
}

#[test]
fn quiet_prints_only_problems_and_the_summary() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    cloned(&env, &fixture, "alpha");
    let beta = cloned(&env, &fixture, "beta");
    git(&beta, &["checkout", "-q", "--detach"]);

    let out = env.code(0, &["--quiet", "sync"]);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(line_for(&text, "beta").contains("skipped"), "{text}");
    assert!(text.contains("1 up to date"), "{text}");
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn verbose_traces_git_commands() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    let out = env.code(0, &["--verbose", "sync"]);
    let err = stderr(&out);
    assert!(err.contains("$ git"), "{err}");
    assert!(err.contains(" clone "), "{err}");
    assert!(err.contains("cloning alpha"), "{err}");
}

#[test]
fn colour_follows_the_flag() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    cloned(&env, &fixture, "alpha");
    let never = env.code(0, &["--color", "never", "sync"]);
    assert!(!never.stdout.contains(&0x1b));
    let always = env.code(0, &["--color", "always", "sync"]);
    assert!(always.stdout.contains(&0x1b));
}

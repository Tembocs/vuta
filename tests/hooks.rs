//! M3 and M4: `vuta hook add|list|remove`, the hook phase of `vuta sync`,
//! and Ctrl-C.
//!
//! Hook commands must work under both `sh -c` and `cmd /C`. `echo x>> f`
//! and `exit N` mean the same in both.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, Fixture, git, line_for, stderr, stdout};
use vuta::db;
use vuta::model::{HookOutcome, OutcomeKind, RunResult};

/// Appends a line to `hook.log` in the checkout each time it runs.
const LOG_RUN: &str = "echo run>> hook.log";

/// How many times the logging hook ran in `checkout`.
fn runs_logged(checkout: &Path) -> usize {
    fs::read_to_string(checkout.join("hook.log"))
        .map(|text| text.lines().filter(|l| l.trim() == "run").count())
        .unwrap_or(0)
}

/// Register `name` without cloning, so hooks can be added before the clone.
fn register(env: &Env, fixture: &Fixture, name: &str) -> PathBuf {
    env.ok(&["add", &fixture.url(), name, "--name", name, "--no-clone"]);
    env.checkout(name)
}

fn last_results(env: &Env) -> Vec<RunResult> {
    let conn = env.conn();
    let run = db::runs::recent(&conn, 1).unwrap().remove(0).run;
    db::runs::results_for_run(&conn, run.id).unwrap()
}

fn hook_outcomes(env: &Env) -> Vec<(String, Option<HookOutcome>)> {
    last_results(env)
        .into_iter()
        .map(|r| (r.repo_name, r.hook_outcome))
        .collect()
}

#[test]
fn hooks_run_only_for_repositories_that_changed() {
    let env = Env::with_root();
    let mut changing = Fixture::new();
    let still = Fixture::new();
    let alpha = register(&env, &changing, "alpha");
    let beta = register(&env, &still, "beta");
    env.ok(&["hook", "add", "alpha", LOG_RUN]);
    env.ok(&["hook", "add", "beta", LOG_RUN]);

    // Both are cloned, which counts as a change.
    let text = env.ok(&["sync"]);
    assert!(text.contains("Hooks"), "{text}");
    assert_eq!((runs_logged(&alpha), runs_logged(&beta)), (1, 1));

    // Only alpha's upstream moves.
    changing.push_commit();
    let text = env.ok(&["sync"]);
    assert!(line_for(&text, "alpha").contains("updated"), "{text}");
    assert!(line_for(&text, "beta").contains("up to date"), "{text}");
    assert_eq!((runs_logged(&alpha), runs_logged(&beta)), (2, 1));
    assert_eq!(
        hook_outcomes(&env),
        [
            ("alpha".into(), Some(HookOutcome::Ok)),
            ("beta".into(), Some(HookOutcome::NotRun)),
        ]
    );

    // Nothing changed: no hooks and no heading.
    let text = env.ok(&["sync"]);
    assert!(!text.contains("Hooks"), "{text}");
    assert_eq!((runs_logged(&alpha), runs_logged(&beta)), (2, 1));
}

#[test]
fn a_failing_hook_keeps_the_outcome_and_exits_1() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    env.ok(&["sync"]);
    env.ok(&["hook", "add", "alpha", "exit 3"]);
    env.ok(&["hook", "add", "alpha", LOG_RUN]);
    fixture.push_commit();

    let out = env.code(1, &["sync"]);
    let text = stdout(&out);
    assert!(line_for(&text, "alpha").contains("updated"), "{text}");
    assert!(text.contains("hook failed (exit 3)"), "{text}");
    assert!(text.contains("1 hook failed"), "{text}");
    // The first failure stops the repository's remaining hooks.
    assert_eq!(runs_logged(&path), 0);

    let result = &last_results(&env)[0];
    assert_eq!(result.outcome, OutcomeKind::Updated);
    assert_eq!(result.hook_outcome, Some(HookOutcome::Failed));
    let conn = env.conn();
    assert_eq!(
        db::runs::recent(&conn, 1).unwrap()[0].run.exit_code,
        Some(1)
    );

    let log = env.ok(&["log"]);
    assert!(log.contains("exit 1"), "{log}");
    assert!(line_for(&log, "alpha").contains("hook failed"), "{log}");
}

#[test]
fn no_hooks_force_hooks_and_always() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    env.ok(&["hook", "add", "alpha", LOG_RUN]);

    env.ok(&["sync", "--no-hooks"]);
    assert_eq!(runs_logged(&path), 0);
    assert_eq!(hook_outcomes(&env)[0].1, Some(HookOutcome::NotRun));

    // Up to date, but forced.
    env.ok(&["sync", "--force-hooks"]);
    assert_eq!(runs_logged(&path), 1);
    assert_eq!(hook_outcomes(&env)[0].1, Some(HookOutcome::Ok));

    fixture.push_commit();
    env.ok(&["sync", "--no-hooks"]);
    assert_eq!(runs_logged(&path), 1);

    env.code(2, &["sync", "--no-hooks", "--force-hooks"]);

    // An `--always` hook runs after an up-to-date sync too.
    env.ok(&["hook", "add", "alpha", "echo run>> always.log", "--always"]);
    env.ok(&["sync"]);
    assert_eq!(runs_logged(&path), 1);
    assert!(path.join("always.log").exists());
}

#[test]
fn hooks_never_run_after_a_skip_or_failure() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    env.ok(&["sync"]);
    env.ok(&["hook", "add", "alpha", LOG_RUN, "--always"]);
    fs::write(path.join("file.txt"), "local edit").unwrap();
    fixture.push_commit();

    let text = env.ok(&["sync", "--force-hooks"]);
    assert!(line_for(&text, "alpha").contains("skipped"), "{text}");
    assert_eq!(runs_logged(&path), 0);
    assert_eq!(hook_outcomes(&env)[0].1, Some(HookOutcome::NotRun));

    fs::remove_dir_all(fixture.path("upstream.git")).unwrap();
    env.code(1, &["sync", "--force-hooks"]);
    assert_eq!(runs_logged(&path), 0);

    // --fetch-only skips too.
    let other = Fixture::new();
    let beta = register(&env, &other, "beta");
    env.ok(&["sync", "beta"]);
    env.ok(&["hook", "add", "beta", LOG_RUN, "--always"]);
    let mut other = other;
    other.push_commit();
    env.ok(&["sync", "beta", "--fetch-only"]);
    assert_eq!(runs_logged(&beta), 0);
}

#[test]
fn hooks_for_another_platform_are_ignored() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    let (here, there) = if cfg!(windows) {
        ("windows", "unix")
    } else {
        ("unix", "windows")
    };
    env.ok(&[
        "hook",
        "add",
        "alpha",
        "echo run>> other.log",
        "--platform",
        there,
    ]);
    env.ok(&["hook", "add", "alpha", LOG_RUN, "--platform", here]);
    env.ok(&["sync"]);
    assert_eq!(runs_logged(&path), 1);
    assert!(!path.join("other.log").exists());
    env.code(2, &["hook", "add", "alpha", "x", "--platform", "mac"]);
}

#[test]
fn hooks_see_the_repository_environment() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = register(&env, &fixture, "alpha");
    env.ok(&["sync"]);
    let old = git(&path, &["rev-parse", "HEAD"]);
    env.ok(&[
        "hook",
        "add",
        "alpha",
        "echo \"$VUTA_REPO_NAME $VUTA_OLD_REV $VUTA_NEW_REV $PWD\" > env.txt",
        "--platform",
        "unix",
    ]);
    env.ok(&[
        "hook",
        "add",
        "alpha",
        "echo %VUTA_REPO_NAME% %VUTA_OLD_REV% %VUTA_NEW_REV% %CD%> env.txt",
        "--platform",
        "windows",
    ]);
    let new = fixture.push_commit();
    env.ok(&["sync"]);

    let text = fs::read_to_string(path.join("env.txt")).unwrap();
    let words: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(&words[..3], ["alpha", old.as_str(), new.as_str()], "{text}");
    // The hook runs in the checkout.
    let cwd = fs::canonicalize(words[3]).unwrap();
    assert_eq!(cwd, fs::canonicalize(&path).unwrap());
}

#[test]
fn hook_add_list_and_remove() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    register(&env, &fixture, "beta");

    let out = env.ok(&["hook", "add", "beta", "make"]);
    assert!(out.contains("added hook 1 to beta: make"), "{out}");
    env.ok(&[
        "hook",
        "add",
        "alpha",
        "make.bat",
        "--platform",
        "windows",
        "--always",
    ]);

    let list = env.ok(&["hook", "list"]);
    let lines: Vec<&str> = list.lines().collect();
    assert!(lines[0].starts_with("ID"), "{list}");
    assert!(
        lines[1].contains("alpha") && lines[1].contains("windows"),
        "{list}"
    );
    assert!(
        lines[1].contains("always") && lines[1].ends_with("make.bat"),
        "{list}"
    );
    assert!(
        lines[2].contains("beta") && lines[2].contains("changed"),
        "{list}"
    );

    let list = env.ok(&["hook", "ls", "BETA"]);
    assert_eq!(list.lines().count(), 2, "{list}");
    let show = env.ok(&["show", "beta"]);
    assert!(show.contains("Hooks") && show.contains("make"), "{show}");

    let out = env.ok(&["hook", "remove", "1"]);
    assert!(out.contains("removed hook 1: make"), "{out}");
    let out = env.code(3, &["hook", "rm", "1"]);
    assert!(
        stderr(&out).contains("no hook with id 1"),
        "{}",
        stderr(&out)
    );
    let out = env.code(3, &["hook", "add", "nope", "make"]);
    assert!(stderr(&out).starts_with("error: no repository named 'nope'"));
    env.code(3, &["hook", "list", "nope"]);
    env.code(3, &["hook", "add", "alpha", "  "]);

    // Removing a repository removes its hooks.
    env.ok(&["remove", "alpha"]);
    let out = env.code(0, &["hook", "list"]);
    assert!(stderr(&out).contains("no hooks"), "{}", stderr(&out));
}

/// Ctrl-C during a hook: the hook finishes, nothing else runs, the run row
/// is closed and vuta exits 130. Unix only, since the test sends SIGINT
/// with `kill`.
#[cfg(unix)]
#[test]
fn interrupt_exits_130_with_a_closed_run() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let env = Env::with_root();
    let fixture = Fixture::new();
    let alpha = register(&env, &fixture, "alpha");
    let beta = register(&env, &fixture, "beta");
    env.ok(&["sync"]);
    env.ok(&["hook", "add", "alpha", "touch started && sleep 2"]);
    env.ok(&["hook", "add", "beta", LOG_RUN]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_vuta"))
        .arg("--db")
        .arg(env.db())
        .args(["sync", "--force-hooks"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !alpha.join("started").exists() {
        assert!(Instant::now() < deadline, "the hook never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());

    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    // beta's hook, which would have run next, did not.
    assert_eq!(runs_logged(&beta), 0);
    let run = db::runs::recent(&env.conn(), 1).unwrap().remove(0).run;
    assert_eq!(run.exit_code, Some(130));
    assert!(run.finished_at.is_some());
}

#[test]
fn json_sync_sends_hook_output_to_stderr() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    register(&env, &fixture, "alpha");
    env.ok(&["hook", "add", "alpha", "echo HOOK-MARKER"]);

    let out = env.code(0, &["sync", "--format", "json"]);
    let text = stdout(&out);
    let doc: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"));
    assert_eq!(doc["results"][0]["hook_outcome"], "ok");
    assert!(
        !text.contains("HOOK-MARKER") && !text.contains("Hooks"),
        "{text}"
    );
    assert!(stderr(&out).contains("HOOK-MARKER"), "{}", stderr(&out));
}

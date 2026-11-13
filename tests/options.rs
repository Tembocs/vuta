//! Per-repository sync options: fast-forwarding around local changes
//! (`--allow-dirty`), the rebase strategy, and submodules. Each has a case
//! where the option lets a sync through and one where git or Vuta refuses,
//! which must leave the checkout exactly as it was.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, Fixture, git, identify, line_for, local_commit, stderr, stdout};

/// Register and clone `name` with plain `vuta add`, giving the clone a
/// committer identity so a rebase can run on any machine.
fn cloned(env: &Env, fixture: &Fixture, name: &str) -> PathBuf {
    env.ok(&["add", &fixture.url(), name, "--name", name]);
    let path = env.checkout(name);
    identify(&path);
    path
}

fn head(path: &Path) -> String {
    git(path, &["rev-parse", "HEAD"])
}

fn read(path: &Path, name: &str) -> String {
    fs::read_to_string(path.join(name)).unwrap()
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
}

/// The newest run's results, from `vuta log --format json`.
fn last_results(env: &Env) -> serde_json::Value {
    json(&env.ok(&["log", "-n", "1", "--format", "json"]))[0]["results"].clone()
}

#[test]
fn options_are_set_shown_listed_exported_and_imported() {
    let env = Env::with_root();
    env.ok(&[
        "add",
        "https://example.com/zed.git",
        "--no-clone",
        "--submodules",
    ]);
    env.ok(&["add", "https://example.com/v.git", "--no-clone"]);
    let out = env.ok(&["show", "zed"]);
    assert!(
        line_for(&out, "sync").ends_with("ff-only, submodules"),
        "{out}"
    );
    let out = env.ok(&["show", "v"]);
    assert!(line_for(&out, "sync").ends_with("ff-only"), "{out}");

    env.ok(&["set", "zed", "--allow-dirty", "--strategy", "rebase"]);
    let out = env.ok(&["show", "zed"]);
    assert!(
        line_for(&out, "sync").ends_with("rebase, allow-dirty, submodules"),
        "{out}"
    );

    let list = json(&env.ok(&["list", "--format", "json"]));
    let options = |repo: &serde_json::Value| {
        (
            repo["strategy"].clone(),
            repo["allow_dirty"].clone(),
            repo["submodules"].clone(),
        )
    };
    let on = (
        serde_json::json!("rebase"),
        serde_json::json!(true),
        serde_json::json!(true),
    );
    let off = (
        serde_json::json!("ff-only"),
        serde_json::json!(false),
        serde_json::json!(false),
    );
    assert_eq!(list[0]["name"], "v");
    assert_eq!(options(&list[0]), off);
    assert_eq!(options(&list[1]), on);
    let show = json(&env.ok(&["show", "zed", "--format", "json"]));
    assert_eq!(options(&show), on);

    // Export and import carry them to another machine.
    let file = env.dir.path().join("registry.json");
    env.ok(&["export", "-o", file.to_str().unwrap()]);
    let exported = json(&fs::read_to_string(&file).unwrap());
    assert_eq!(options(&exported["repos"][1]), on);
    let other = Env::new();
    let place = format!("src={}", other.root().display());
    let plan = json(&other.ok(&[
        "import",
        file.to_str().unwrap(),
        "--map-root",
        &place,
        "--dry-run",
        "--format",
        "json",
    ]));
    assert_eq!(options(&plan["repos"][1]), on);
    let text = other.ok(&["import", file.to_str().unwrap(), "--map-root", &place]);
    assert!(
        line_for(&text, "zed").contains("sync: rebase, allow-dirty, submodules"),
        "{text}"
    );
    assert!(!line_for(&text, "v").contains("sync:"), "{text}");
    let show = json(&other.ok(&["show", "zed", "--format", "json"]));
    assert_eq!(options(&show), on);

    // And each can be turned off again.
    env.ok(&[
        "set",
        "zed",
        "--no-allow-dirty",
        "--no-submodules",
        "--strategy",
        "ff-only",
    ]);
    let show = json(&env.ok(&["show", "zed", "--format", "json"]));
    assert_eq!(options(&show), off);

    // Only the two strategies exist, and on/off pairs exclude each other.
    env.code(2, &["set", "zed", "--strategy", "merge"]);
    env.code(2, &["set", "zed", "--allow-dirty", "--no-allow-dirty"]);
    env.code(2, &["set", "zed", "--submodules", "--no-submodules"]);
}

#[test]
fn import_refuses_an_unknown_strategy() {
    let env = Env::with_root();
    let file = env.dir.path().join("registry.json");
    let root = env.root();
    let text = serde_json::json!({
        "format": "vuta-registry",
        "version": 1,
        "roots": [{ "name": "src", "path": root.to_str().unwrap() }],
        "repos": [{ "name": "zed", "root": "src", "path": "zed",
                    "url": "https://example.com/zed.git", "strategy": "merge" }]
    });
    fs::write(&file, text.to_string()).unwrap();
    let out = env.code(3, &["import", file.to_str().unwrap()]);
    assert!(
        stderr(&out).contains("repository 'zed': unknown strategy 'merge'"),
        "{}",
        stderr(&out)
    );
    assert!(env.ok(&["list", "--format", "json"]).trim() == "[]");
}

#[test]
fn allow_dirty_fast_forwards_around_changes_upstream_does_not_touch() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    fixture.push_file("extra.txt", "extra\n");
    let path = cloned(&env, &fixture, "alpha");
    fs::write(path.join("extra.txt"), "local edit\n").unwrap();
    let new = fixture.push_commit();

    // Skipped as before until the option is on.
    let text = env.ok(&["sync"]);
    assert!(
        line_for(&text, "alpha").contains("local changes; 1 commit behind"),
        "{text}"
    );

    env.ok(&["set", "alpha", "--allow-dirty"]);
    let text = env.ok(&["sync"]);
    assert!(line_for(&text, "alpha").contains("updated"), "{text}");
    assert_eq!(head(&path), new);
    assert_eq!(read(&path, "extra.txt"), "local edit\n");
    assert_eq!(read(&path, "file.txt"), "2\n");
    assert_eq!(
        git(&path, &["status", "--porcelain", "--untracked-files=no"]),
        "M extra.txt"
    );
}

#[test]
fn allow_dirty_skips_when_upstream_changes_the_same_file() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    env.ok(&["set", "alpha", "--allow-dirty"]);
    let before = head(&path);
    fs::write(path.join("file.txt"), "local edit\n").unwrap();
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("skipped"), "{text}");
    assert!(
        line.contains("local changes overlap upstream in file.txt; 1 commit behind"),
        "{text}"
    );
    assert_eq!(head(&path), before);
    assert_eq!(read(&path, "file.txt"), "local edit\n");
    let result = &last_results(&env)[0];
    assert_eq!(result["reason"], "dirty");
    assert_eq!(result["commits"], 1);

    env.code(1, &["sync", "--strict"]);
    assert_eq!(head(&path), before);
    assert_eq!(read(&path, "file.txt"), "local edit\n");
}

#[test]
fn rebase_strategy_replays_local_commits_onto_upstream() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    local_commit(&path, "local.txt", "mine\n");
    let upstream = fixture.push_commit();

    // ff-only, the default, skips a diverged checkout.
    let text = env.ok(&["sync"]);
    assert!(
        line_for(&text, "alpha").contains("1 local commit, 1 behind"),
        "{text}"
    );

    env.ok(&["set", "alpha", "--strategy", "rebase"]);
    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("updated"), "{text}");
    assert!(line.contains("1 commit; rebased 1 local commit"), "{text}");
    assert_eq!(git(&path, &["rev-parse", "HEAD~1"]), upstream);
    assert_eq!(read(&path, "local.txt"), "mine\n");
    let result = &last_results(&env)[0];
    assert_eq!(result["outcome"], "updated");
    assert_eq!(result["rebased"], 1);
    assert_eq!(result["new_rev"], head(&path).as_str());

    // The stored result reads back the same way.
    let log = env.ok(&["log", "alpha", "-n", "1"]);
    assert!(log.contains("rebased 1 local commit"), "{log}");
}

#[test]
fn a_rebase_that_stops_is_aborted_and_the_checkout_left_as_it_was() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let path = cloned(&env, &fixture, "alpha");
    env.ok(&["set", "alpha", "--strategy", "rebase"]);
    let mine = local_commit(&path, "file.txt", "mine\n");
    fs::write(path.join("untracked.txt"), "keep me").unwrap();
    fixture.push_commit();
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    let line = line_for(&text, "alpha");
    assert!(line.contains("skipped"), "{text}");
    assert!(
        line.contains(
            "rebase stopped (conflict in file.txt) and was undone; 1 local commit, 2 behind"
        ),
        "{text}"
    );
    assert_eq!(head(&path), mine);
    assert_eq!(git(&path, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(git(&path, &["status", "--porcelain"]), "?? untracked.txt");
    assert_eq!(read(&path, "file.txt"), "mine\n");
    assert_eq!(read(&path, "untracked.txt"), "keep me");
    for dir in ["rebase-merge", "rebase-apply"] {
        assert!(!path.join(".git").join(dir).exists(), "{dir} left behind");
    }
    let result = &last_results(&env)[0];
    assert_eq!(result["outcome"], "skipped");
    assert_eq!(result["reason"], "rebase_stopped");

    env.code(1, &["sync", "--strict"]);
    assert_eq!(head(&path), mine);
}

#[test]
fn rebase_strategy_skips_a_checkout_with_local_changes() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    fixture.push_file("extra.txt", "extra\n");
    let path = cloned(&env, &fixture, "alpha");
    env.ok(&["set", "alpha", "--strategy", "rebase", "--allow-dirty"]);
    let mine = local_commit(&path, "local.txt", "mine\n");
    fs::write(path.join("extra.txt"), "local edit\n").unwrap();
    fixture.push_commit();

    let text = env.ok(&["sync"]);
    assert!(
        line_for(&text, "alpha").contains("local changes; 1 commit behind"),
        "{text}"
    );
    assert_eq!(head(&path), mine);
    assert_eq!(read(&path, "extra.txt"), "local edit\n");
}

/// A superproject upstream with `lib` as a submodule at `lib`.
fn superproject() -> (Fixture, Fixture) {
    let lib = Fixture::new();
    let mut fixture = Fixture::new();
    fixture.push_submodule("lib", &lib);
    (fixture, lib)
}

#[test]
fn submodules_are_cloned_and_follow_updates() {
    let env = Env::with_root();
    let (mut fixture, mut lib) = superproject();
    let out = env.ok(&[
        "add",
        &fixture.url(),
        "app",
        "--name",
        "app",
        "--submodules",
    ]);
    // The result line, after the "added app" line.
    let line = line_for(&out, "[1/1]");
    assert!(line.contains("cloned"), "{out}");
    assert!(line.contains("submodules ok"), "{out}");
    let path = env.checkout("app");
    assert_eq!(read(&path.join("lib"), "file.txt"), "1\n");

    let lib_rev = lib.push_commit();
    let new = fixture.bump_submodule("lib");
    let text = env.ok(&["sync"]);
    let line = line_for(&text, "app");
    assert!(line.contains("updated"), "{text}");
    assert!(line.contains("submodules ok"), "{text}");
    assert_eq!(head(&path), new);
    assert_eq!(head(&path.join("lib")), lib_rev);
    assert_eq!(
        git(&path, &["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
    assert_eq!(last_results(&env)[0]["submodules"], "ok");

    // Without the option, submodules are left alone.
    let plain_env = Env::with_root();
    plain_env.ok(&["add", &fixture.url(), "app", "--name", "app"]);
    let plain = plain_env.checkout("app");
    assert!(!plain.join("lib").join("file.txt").exists());
    assert_eq!(
        last_results(&plain_env)[0]["submodules"],
        serde_json::Value::Null
    );
}

#[test]
fn a_failed_submodule_update_keeps_the_outcome_fails_the_run_and_heals_later() {
    let env = Env::with_root();
    let (fixture, lib) = superproject();
    // The submodule's upstream is unreachable.
    let lib_upstream = lib.path("upstream.git");
    let hidden = lib.path("hidden.git");
    fs::rename(&lib_upstream, &hidden).unwrap();
    env.ok(&[
        "add",
        &fixture.url(),
        "app",
        "--name",
        "app",
        "--submodules",
        "--no-clone",
    ]);
    env.ok(&["hook", "add", "app", "echo run>> hook.log"]);

    let out = env.code(1, &["sync"]);
    let text = stdout(&out);
    let line = line_for(&text, "app");
    assert!(line.contains("cloned"), "{text}");
    assert!(line.contains("submodules failed:"), "{text}");
    assert!(text.contains("1 submodule update failed"), "{text}");
    let path = env.checkout("app");
    assert_eq!(head(&path), fixture.upstream_rev());
    assert!(!path.join("lib").join("file.txt").exists());
    // Hooks still run: the repository itself was cloned.
    assert_eq!(read(&path, "hook.log").trim(), "run");

    let show = json(&env.ok(&["show", "app", "--format", "json"]));
    assert_eq!(show["last_outcome"], "cloned");
    let result = &show["results"][0];
    assert_eq!(result["outcome"], "cloned");
    assert_eq!(result["submodules"], "failed");
    assert!(!result["message"].as_str().unwrap().is_empty());
    let sync_json = json(&env.ok(&["log", "-n", "1", "--format", "json"]));
    assert_eq!(sync_json[0]["tally"]["submodules_failed"], 1);
    assert_eq!(sync_json[0]["exit_code"], 1);
    let shown = env.ok(&["show", "app"]);
    assert!(shown.contains("Last failure"), "{shown}");

    // Once the submodule's upstream is back, an up-to-date sync brings the
    // submodule in.
    fs::rename(&hidden, &lib_upstream).unwrap();
    let text = env.ok(&["sync"]);
    let line = line_for(&text, "app");
    assert!(line.contains("up to date"), "{text}");
    assert!(line.contains("submodules ok"), "{text}");
    assert_eq!(read(&path.join("lib"), "file.txt"), "1\n");

    // With nothing left to do, the step does not run again.
    let text = env.ok(&["sync"]);
    assert!(!line_for(&text, "app").contains("submodules"), "{text}");
}

//! M1: roots, `add --no-clone`, `list`, `show`, `set`, `tag`, `remove` and
//! `config`.

mod common;

use common::{Env, Fixture, arg, line_for, stderr, stdout};
use vuta::db;

const ZED: &str = "https://github.com/zed-industries/zed.git";
const V: &str = "git@github.com:vlang/v.git";

#[test]
fn first_root_is_the_default_and_roots_can_change() {
    let env = Env::new();
    let src = env.dir.path().join("src");
    let work = env.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let out = env.ok(&["root", "add", "src", arg(&src)]);
    assert!(out.contains("(default)"), "{out}");
    env.ok(&["root", "add", "work", arg(&work)]);

    let list = env.ok(&["root", "list"]);
    assert!(line_for(&list, "src").ends_with("default"), "{list}");
    assert!(line_for(&list, "src").contains("(missing)"), "{list}");
    assert!(!line_for(&list, "work").contains("default"), "{list}");

    env.ok(&["root", "default", "WORK"]);
    let list = env.ok(&["root", "list"]);
    assert!(line_for(&list, "work").ends_with("default"), "{list}");

    let moved = env.dir.path().join("moved");
    env.ok(&["root", "move", "src", arg(&moved)]);
    let conn = env.conn();
    assert_eq!(
        db::roots::by_name(&conn, "src").unwrap().unwrap().path,
        moved
    );

    env.ok(&["root", "remove", "src"]);
    assert!(db::roots::by_name(&conn, "src").unwrap().is_none());
}

#[test]
fn relative_root_paths_are_made_absolute() {
    let env = Env::new();
    env.vuta()
        .current_dir(env.dir.path())
        .args(["root", "add", "src", "src"])
        .assert()
        .success();
    let root = db::roots::by_name(&env.conn(), "src").unwrap().unwrap();
    assert!(root.path.is_absolute());
    assert!(root.path.ends_with("src"));
}

#[test]
fn a_root_with_repositories_cannot_be_removed() {
    let env = Env::with_root();
    env.ok(&["add", ZED, "--no-clone"]);
    let out = env.code(3, &["root", "remove", "src"]);
    assert!(
        stderr(&out).contains("still holds 1 repositories"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unknown_names_exit_3_with_one_line() {
    let env = Env::with_root();
    for args in [
        &["show", "nope"][..],
        &["set", "nope", "--disable"],
        &["tag", "nope", "+x"],
        &["remove", "nope"],
        &["root", "default", "nope"],
        &["add", ZED, "--no-clone", "--root", "nope"],
    ] {
        let out = env.code(3, args);
        let err = stderr(&out);
        assert_eq!(err.lines().count(), 1, "{args:?}: {err}");
        assert!(err.starts_with("error: no "), "{args:?}: {err}");
    }
}

#[test]
fn add_needs_a_root() {
    let env = Env::new();
    let out = env.code(3, &["add", ZED, "--no-clone"]);
    assert!(stderr(&out).contains("vuta root add"), "{}", stderr(&out));
}

#[test]
fn add_registers_with_inferred_name_path_and_tags() {
    let env = Env::with_root();
    let out = env.ok(&[
        "add",
        ZED,
        "rust\\zed\\",
        "--tag",
        "rust",
        "-t",
        "editor",
        "--no-clone",
    ]);
    assert!(out.contains("added zed  src:rust/zed"), "{out}");
    env.ok(&[
        "add",
        V,
        "--no-clone",
        "--branch",
        "master",
        "--filter",
        "blob:none",
        "--depth",
        "1",
    ]);

    let conn = env.conn();
    let zed = db::repos::by_name(&conn, "zed").unwrap().unwrap();
    assert_eq!(zed.rel_path, "rust/zed");
    assert_eq!(zed.url, ZED);
    assert_eq!(db::repos::tags(&conn, zed.id).unwrap(), ["editor", "rust"]);
    let v = db::repos::by_name(&conn, "v").unwrap().unwrap();
    assert_eq!(v.rel_path, "v");
    assert_eq!(v.branch.as_deref(), Some("master"));
    assert_eq!(v.clone_filter.as_deref(), Some("blob:none"));
    assert_eq!(v.clone_depth, Some(1));

    let list = env.ok(&["ls"]);
    let lines: Vec<&str> = list.lines().collect();
    assert!(lines[0].starts_with("NAME"), "{list}");
    assert!(lines[1].starts_with("v "), "{list}");
    assert!(lines[2].starts_with("zed "), "{list}");
    assert!(line_for(&list, "zed").contains("editor,rust"), "{list}");
    assert!(line_for(&list, "zed").contains("never"), "{list}");

    let rust = env.ok(&["list", "--tag", "RUST"]);
    assert_eq!(rust.lines().count(), 2, "{rust}");
}

#[test]
fn add_refuses_clashes_and_bad_paths() {
    let env = Env::with_root();
    env.ok(&["add", ZED, "--no-clone"]);
    let out = env.code(3, &["add", "https://example.com/other/zed", "--no-clone"]);
    assert!(stderr(&out).contains("--name"), "{}", stderr(&out));
    let out = env.code(3, &["add", "https://example.com/x/y", "zed", "--no-clone"]);
    assert!(stderr(&out).contains("already at zed"), "{}", stderr(&out));
    env.code(3, &["add", ZED, "../outside", "--name", "z2", "--no-clone"]);
    env.code(3, &["add", ZED, "--name", "two words", "--no-clone"]);
    env.code(2, &["add", ZED, "--depth", "0", "--no-clone"]);
}

#[test]
fn local_remote_paths_are_stored_absolute() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    env.vuta()
        .current_dir(fixture.dir.path())
        .args(["add", "upstream.git", "--name", "local", "--no-clone"])
        .assert()
        .success();
    let repo = db::repos::by_name(&env.conn(), "local").unwrap().unwrap();
    assert_eq!(
        std::path::Path::new(&repo.url),
        fixture.path("upstream.git")
    );
}

#[test]
fn show_prints_everything_about_one_repository() {
    let env = Env::with_root();
    env.ok(&[
        "add",
        ZED,
        "rust/zed",
        "--tag",
        "rust",
        "--filter",
        "blob:none",
        "--no-clone",
    ]);
    let out = env.ok(&["show", "ZED"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "zed");
    assert!(line_for(&out, "root").contains("src"), "{out}");
    assert!(line_for(&out, "path").contains("(not cloned)"), "{out}");
    assert!(line_for(&out, "url").ends_with(ZED), "{out}");
    assert!(line_for(&out, "branch").contains("default"), "{out}");
    assert!(
        line_for(&out, "clone").contains("filter blob:none"),
        "{out}"
    );
    assert!(line_for(&out, "tags").ends_with("rust"), "{out}");
    assert!(line_for(&out, "synced").ends_with("never"), "{out}");
}

#[test]
fn set_changes_the_record() {
    let env = Env::with_root();
    env.ok(&[
        "add",
        "https://github.com/modular/modular.git",
        "--no-clone",
    ]);
    let out = env.ok(&[
        "set",
        "modular",
        "--name",
        "mojo",
        "--path",
        "mojo/modular",
        "--branch",
        "main",
        "--depth",
        "5",
        "--disable",
    ]);
    assert!(out.contains("updated mojo (was modular)"), "{out}");
    assert!(out.contains("src:modular"), "{out}");
    assert!(out.contains("src:mojo/modular"), "{out}");

    let conn = env.conn();
    assert!(db::repos::by_name(&conn, "modular").unwrap().is_none());
    let mojo = db::repos::by_name(&conn, "mojo").unwrap().unwrap();
    assert_eq!(mojo.rel_path, "mojo/modular");
    assert_eq!(mojo.branch.as_deref(), Some("main"));
    assert_eq!(mojo.clone_depth, Some(5));
    assert!(!mojo.enabled);
    assert!(env.ok(&["list"]).contains("(disabled)"));

    env.ok(&["set", "mojo", "--default-branch", "--no-depth", "--enable"]);
    let mojo = db::repos::by_name(&conn, "mojo").unwrap().unwrap();
    assert_eq!(
        (mojo.branch, mojo.clone_depth, mojo.enabled),
        (None, None, true)
    );

    // Nothing to change is a usage error.
    env.code(2, &["set", "mojo"]);
    env.code(2, &["set", "mojo", "--enable", "--disable"]);
}

#[test]
fn set_can_move_a_record_to_another_root() {
    let env = Env::with_root();
    let work = env.dir.path().join("work");
    env.ok(&["root", "add", "work", arg(&work)]);
    env.ok(&["add", ZED, "--no-clone"]);
    env.ok(&["set", "zed", "--root", "work"]);
    let conn = env.conn();
    let zed = db::repos::by_name(&conn, "zed").unwrap().unwrap();
    let work_root = db::roots::by_name(&conn, "work").unwrap().unwrap();
    assert_eq!(zed.root_id, work_root.id);
    // With two roots, the listing names them.
    let list = env.ok(&["list"]);
    assert!(list.lines().next().unwrap().contains("ROOT"), "{list}");
    assert!(line_for(&list, "zed").contains("work"), "{list}");
}

#[test]
fn set_refuses_a_taken_name() {
    let env = Env::with_root();
    env.ok(&["add", ZED, "--no-clone"]);
    env.ok(&["add", V, "--no-clone"]);
    let out = env.code(3, &["set", "v", "--name", "ZED"]);
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
}

#[test]
fn tags_are_added_and_removed() {
    let env = Env::with_root();
    env.ok(&["add", ZED, "--tag", "editor", "--no-clone"]);
    let out = env.ok(&["tag", "zed", "+rust", "-editor", "tools"]);
    assert_eq!(out.trim(), "zed  rust, tools");
    let out = env.ok(&["tag", "zed"]);
    assert_eq!(out.trim(), "zed  rust, tools");
    let out = env.ok(&["tag", "zed", "-rust", "-tools"]);
    assert_eq!(out.trim(), "zed  no tags");
    env.code(3, &["tag", "zed", "+"]);
}

#[test]
fn remove_forgets_and_keeps_files_by_default() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let checkout = fixture.clone_to(&env.checkout("zed"));
    env.ok(&["add", &fixture.url(), "zed", "--name", "zed", "--no-clone"]);
    let out = env.ok(&["rm", "zed"]);
    assert!(out.contains("its checkout stays at"), "{out}");
    assert!(checkout.exists());
    assert!(db::repos::by_name(&env.conn(), "zed").unwrap().is_none());
}

#[test]
fn remove_delete_files_asks_first() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let checkout = fixture.clone_to(&env.checkout("zed"));
    env.ok(&["add", &fixture.url(), "zed", "--name", "zed", "--no-clone"]);

    // No answer, or anything but yes, keeps everything.
    for answer in ["", "n\n", "nope\n"] {
        let out = env
            .vuta()
            .args(["remove", "zed", "--delete-files"])
            .write_stdin(answer)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(stderr(&out).contains("[y/N]"), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("nothing was removed"),
            "{}",
            stderr(&out)
        );
        assert!(checkout.exists());
        assert!(db::repos::by_name(&env.conn(), "zed").unwrap().is_some());
    }

    let out = env
        .vuta()
        .args(["remove", "zed", "--delete-files"])
        .write_stdin("y\n")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("and deleted"), "{}", stdout(&out));
    assert!(!checkout.exists());
    assert!(env.root().exists());
    assert!(db::repos::by_name(&env.conn(), "zed").unwrap().is_none());
}

#[test]
fn remove_delete_files_with_yes_and_refuses_non_checkouts() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    let checkout = fixture.clone_to(&env.checkout("a"));
    env.ok(&["add", &fixture.url(), "a", "--name", "a", "--no-clone"]);
    env.ok(&["remove", "a", "--delete-files", "--yes"]);
    assert!(!checkout.exists());

    let plain = env.checkout("plain");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("precious.txt"), "keep me").unwrap();
    env.ok(&[
        "add",
        &fixture.url(),
        "plain",
        "--name",
        "plain",
        "--no-clone",
    ]);
    let out = env.code(3, &["remove", "plain", "--delete-files", "-y"]);
    assert!(
        stderr(&out).contains("not a Git checkout"),
        "{}",
        stderr(&out)
    );
    assert!(plain.join("precious.txt").exists());
    assert!(db::repos::by_name(&env.conn(), "plain").unwrap().is_some());

    // --yes without --delete-files is a usage error.
    env.code(2, &["remove", "plain", "--yes"]);
}

#[test]
fn config_reads_and_writes_settings() {
    let env = Env::new();
    assert_eq!(env.ok(&["config", "get", "sync.jobs"]).trim(), "4");
    env.ok(&["config", "set", "sync.jobs", "8"]);
    assert_eq!(env.ok(&["config", "get", "sync.jobs"]).trim(), "8");

    let list = env.ok(&["config", "list"]);
    assert!(line_for(&list, "sync.jobs").contains(" 8 "), "{list}");
    assert!(
        line_for(&list, "history.keep_runs").contains("200 (default)"),
        "{list}"
    );

    let out = env.ok(&["config", "unset", "sync.jobs"]);
    assert!(out.contains("4 (default)"), "{out}");

    let out = env.code(3, &["config", "set", "sync.jobs", "0"]);
    assert!(
        stderr(&out).contains("invalid value '0'"),
        "{}",
        stderr(&out)
    );
    let out = env.code(3, &["config", "get", "nope"]);
    assert!(stderr(&out).contains("unknown setting"), "{}", stderr(&out));
}

#[test]
fn glyph_setting_changes_output() {
    let env = Env::new();
    env.ok(&["config", "set", "ui.glyphs", "ascii"]);
    let out = env.ok(&["doctor"]);
    assert!(line_for(&out, "git").starts_with("ok "), "{out}");
    assert!(!out.contains('✓'), "{out}");

    env.ok(&["config", "set", "ui.glyphs", "unicode"]);
    let out = env.ok(&["doctor"]);
    assert!(line_for(&out, "git").starts_with('✓'), "{out}");
}

/// A clone of `fixture` at `rel` under the root, whose origin then claims
/// to be `url`. Scan reads the URL and never fetches it.
fn checkout_claiming(env: &Env, fixture: &Fixture, rel: &str, url: &str) -> std::path::PathBuf {
    let path = fixture.clone_to(&env.checkout(rel));
    common::git(&path, &["remote", "set-url", "origin", url]);
    path
}

fn names(env: &Env) -> Vec<String> {
    db::repos::select(
        &env.conn(),
        &db::repos::Selection {
            include_disabled: true,
            ..Default::default()
        },
    )
    .unwrap()
    .into_iter()
    .map(|e| e.repo.name)
    .collect()
}

#[test]
fn scan_adopts_an_existing_tree() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    checkout_claiming(&env, &fixture, "rust/zed", ZED);
    let v = checkout_claiming(&env, &fixture, "v", V);
    // A repository inside a checkout is not registered separately.
    checkout_claiming(
        &env,
        &fixture,
        "v/vendor/inner",
        "https://example.com/inner.git",
    );
    std::fs::create_dir_all(v.join("vendor")).unwrap();
    // A checkout without origin is skipped with the reason.
    let lonely = env.checkout("misc/lonely");
    std::fs::create_dir_all(&lonely).unwrap();
    common::git(&lonely, &["init", "-q"]);
    // Plain folders are walked through.
    std::fs::create_dir_all(env.checkout("empty/dir")).unwrap();

    let dry = env.ok(&["scan", "--tag-from-parent", "--dry-run"]);
    assert!(line_for(&dry, "zed").starts_with("would add"), "{dry}");
    assert!(dry.contains("2 would be added"), "{dry}");
    assert!(names(&env).is_empty());

    let out = env.ok(&["scan", "--tag-from-parent"]);
    assert!(line_for(&out, "zed").contains("src:rust/zed"), "{out}");
    assert!(line_for(&out, "zed").contains("tags: rust"), "{out}");
    assert!(
        line_for(&out, "src:misc/lonely").contains("no origin remote"),
        "{out}"
    );
    assert!(!out.contains("inner"), "{out}");
    assert!(
        out.contains("2 added") && out.contains("1 skipped"),
        "{out}"
    );

    let conn = env.conn();
    let zed = db::repos::by_name(&conn, "zed").unwrap().unwrap();
    assert_eq!((zed.rel_path.as_str(), zed.url.as_str()), ("rust/zed", ZED));
    assert_eq!(db::repos::tags(&conn, zed.id).unwrap(), ["rust"]);
    let v = db::repos::by_name(&conn, "v").unwrap().unwrap();
    // Directly under the root there is no parent to tag with.
    assert!(db::repos::tags(&conn, v.id).unwrap().is_empty());

    // A second scan finds nothing new.
    let again = env.ok(&["scan"]);
    assert!(
        again.contains("0 added") && again.contains("2 already registered"),
        "{again}"
    );
    assert_eq!(names(&env), ["v", "zed"]);
}

#[test]
fn scan_respects_max_depth_and_dir() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    checkout_claiming(&env, &fixture, "a/b/c/deep", "https://example.com/deep.git");
    checkout_claiming(
        &env,
        &fixture,
        "a/shallow",
        "https://example.com/shallow.git",
    );

    env.ok(&["scan", "--max-depth", "2"]);
    assert_eq!(names(&env), ["shallow"]);
    // Scanning a subdirectory registers under the root that contains it.
    let out = env.ok(&["scan", env.checkout("a/b").to_str().unwrap()]);
    assert!(line_for(&out, "deep").contains("src:a/b/c/deep"), "{out}");
    assert_eq!(names(&env), ["deep", "shallow"]);
    env.code(2, &["scan", "--max-depth", "0"]);
}

#[test]
fn scan_skips_name_clashes_and_checkouts_outside_roots() {
    let env = Env::with_root();
    let fixture = Fixture::new();
    env.ok(&["add", ZED, "elsewhere/zed", "--no-clone"]);
    checkout_claiming(
        &env,
        &fixture,
        "rust/zed",
        "https://gitlab.com/fork/zed.git",
    );

    let outside = env.dir.path().join("outside");
    let stray = fixture.clone_to(&outside.join("stray"));
    common::git(
        &stray,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.com/stray.git",
        ],
    );

    let out = env.ok(&["scan"]);
    let line = line_for(&out, "src:rust/zed");
    assert!(line.contains("the name 'zed' is taken"), "{out}");
    assert!(line.contains("--name"), "{out}");

    let out = env.ok(&["scan", outside.to_str().unwrap()]);
    assert!(out.contains("outside every root"), "{out}");
    let out = env.ok(&["scan", outside.to_str().unwrap(), "--root", "src"]);
    assert!(out.contains("outside root src"), "{out}");
    assert_eq!(names(&env), ["zed"]);

    env.code(3, &["scan", "--root", "nope"]);
    let missing = env.dir.path().join("missing");
    let out = env.code(3, &["scan", missing.to_str().unwrap()]);
    assert!(
        stderr(&out).contains("is not a directory"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn scan_needs_a_root() {
    let env = Env::new();
    let out = env.code(3, &["scan"]);
    assert!(stderr(&out).contains("vuta root add"), "{}", stderr(&out));
}

#[test]
fn status_reads_local_state_without_fetching() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let url = fixture.url();
    for name in ["clean", "dirty", "detached", "elsewhere", "missing"] {
        env.ok(&["add", &url, "--name", name, "--no-clone"]);
    }
    let plain = env.checkout("plain");
    std::fs::create_dir_all(&plain).unwrap();
    env.ok(&["add", &url, "--name", "plain", "--no-clone"]);
    env.ok(&["sync", "clean", "dirty", "detached", "elsewhere"]);
    env.ok(&["add", &url, "--name", "off", "--no-clone"]);
    env.ok(&["set", "off", "--disable"]);

    std::fs::write(env.checkout("dirty").join("file.txt"), "edit").unwrap();
    common::git(&env.checkout("detached"), &["checkout", "-q", "--detach"]);
    common::git(
        &env.checkout("elsewhere"),
        &["remote", "set-url", "origin", "https://example.com/x.git"],
    );
    fixture.push_commit();

    let out = env.ok(&["status"]);
    let state = |name: &str| line_for(&out, name).to_owned();
    assert!(state("clean").ends_with("clean"), "{out}");
    assert!(state("clean").contains("main"), "{out}");
    assert!(state("dirty").ends_with("local changes"), "{out}");
    assert!(state("detached").ends_with("HEAD is detached"), "{out}");
    assert!(
        state("elsewhere").contains("origin is https://example.com/x.git"),
        "{out}"
    );
    assert!(state("missing").ends_with("not cloned"), "{out}");
    assert!(state("plain").ends_with("not a Git checkout"), "{out}");
    assert!(state("off").ends_with("disabled"), "{out}");

    // Behind counts come from the last fetch.
    env.ok(&["sync", "clean", "--fetch-only"]);
    let out = env.ok(&["status", "CLEAN"]);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(
        line_for(&out, "clean").ends_with("1 commit behind"),
        "{out}"
    );

    env.code(3, &["status", "nope"]);
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
}

#[test]
fn list_as_json() {
    let env = Env::with_root();
    assert_eq!(
        json(&env.ok(&["list", "--format", "json"])),
        serde_json::json!([])
    );
    env.ok(&[
        "add",
        ZED,
        "rust/zed",
        "-t",
        "rust",
        "--branch",
        "main",
        "--no-clone",
    ]);
    env.ok(&["add", V, "--no-clone"]);
    env.ok(&["set", "v", "--disable"]);

    let list = json(&env.ok(&["list", "--format", "json"]));
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 2);
    let zed = &items[1];
    assert_eq!(zed["name"], "zed");
    assert_eq!(zed["root"], "src");
    assert_eq!(zed["rel_path"], "rust/zed");
    assert_eq!(zed["url"], ZED);
    assert_eq!(zed["branch"], "main");
    assert_eq!(zed["tags"], serde_json::json!(["rust"]));
    assert_eq!(zed["last_rev"], serde_json::Value::Null);
    assert_eq!(
        std::path::Path::new(zed["path"].as_str().unwrap()),
        env.checkout("rust/zed")
    );
    assert_eq!(items[0]["enabled"], false);

    let rust = json(&env.ok(&["list", "--tag", "rust", "--format", "json"]));
    assert_eq!(rust.as_array().unwrap().len(), 1);
}

#[test]
fn status_as_json() {
    let env = Env::with_root();
    let mut fixture = Fixture::new();
    let url = fixture.url();
    env.ok(&["add", &url, "--name", "clean"]);
    env.ok(&["add", &url, "--name", "dirty"]);
    env.ok(&["add", &url, "--name", "missing", "--no-clone"]);
    std::fs::write(env.checkout("dirty").join("file.txt"), "edit").unwrap();
    fixture.push_commit();
    env.ok(&["sync", "clean", "--fetch-only"]);

    let status = json(&env.ok(&["status", "--format", "json"]));
    let by_name = |name: &str| {
        status
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()
            .clone()
    };
    let clean = by_name("clean");
    assert_eq!(clean["state"], "branch");
    assert_eq!(clean["branch"], "main");
    assert_eq!(
        (clean["dirty"].clone(), clean["behind"].clone()),
        (false.into(), 1.into())
    );
    assert_eq!(clean["has_upstream"], true);
    assert_eq!(clean["rev"].as_str().unwrap().len(), 40);
    assert_eq!(by_name("dirty")["dirty"], true);
    let missing = by_name("missing");
    assert_eq!(missing["state"], "not_cloned");
    assert_eq!(missing["rev"], serde_json::Value::Null);
}

/// Machine A: two roots, tagged repositories, hooks, a disabled
/// repository and a changed setting.
fn machine_a() -> Env {
    let env = Env::with_root();
    let work = env.dir.path().join("work");
    env.ok(&["root", "add", "work", arg(&work)]);
    env.ok(&[
        "add",
        ZED,
        "rust/zed",
        "-t",
        "rust",
        "-t",
        "editor",
        "--branch",
        "main",
        "--no-clone",
    ]);
    env.ok(&["add", V, "v/v", "--filter", "blob:none", "--no-clone"]);
    env.ok(&[
        "add",
        "https://example.com/tools.git",
        "--root",
        "work",
        "--depth",
        "1",
        "--no-clone",
    ]);
    env.ok(&["set", "tools", "--disable"]);
    env.ok(&["hook", "add", "v", "make", "--platform", "unix"]);
    env.ok(&["hook", "add", "v", "make.bat", "--platform", "windows"]);
    env.ok(&["hook", "add", "zed", "cargo build", "--always"]);
    env.ok(&["config", "set", "sync.jobs", "8"]);
    env
}

fn export_file(env: &Env) -> std::path::PathBuf {
    let file = env.dir.path().join("registry.json");
    let out = env.code(0, &["export", "-o", arg(&file)]);
    assert!(
        stderr(&out).contains("exported 3 repositories, 3 hooks and 2 roots"),
        "{}",
        stderr(&out)
    );
    file
}

fn repos_section(env: &Env) -> serde_json::Value {
    json(&env.ok(&["export"]))["repos"].clone()
}

#[test]
fn export_and_import_move_the_registry_to_another_machine() {
    let a = machine_a();
    let file = export_file(&a);

    let b = Env::new();
    let src = b.dir.path().join("src");
    let work = b.dir.path().join("work");
    let out = b.ok(&[
        "import",
        arg(&file),
        "--map-root",
        &format!("src={}", arg(&src)),
        "--map-root",
        &format!("work={}", arg(&work)),
    ]);
    assert!(out.contains("3 repositories added"), "{out}");
    assert!(
        line_for(&out, "make.bat").contains("[windows, changed]"),
        "{out}"
    );

    // Same repositories, tags and hooks; roots at the mapped paths.
    assert_eq!(repos_section(&b), repos_section(&a));
    let conn = b.conn();
    let src_root = db::roots::by_name(&conn, "src").unwrap().unwrap();
    assert_eq!(src_root.path, src);
    assert!(src_root.is_default);
    assert_eq!(
        db::roots::by_name(&conn, "work").unwrap().unwrap().path,
        work
    );
    assert!(!db::repos::by_name(&conn, "tools").unwrap().unwrap().enabled);
    assert_eq!(b.ok(&["config", "get", "sync.jobs"]).trim(), "8");

    // Importing again changes nothing.
    let again = b.ok(&[
        "import",
        arg(&file),
        "--map-root",
        &format!("src={}", arg(&src)),
        "--map-root",
        &format!("work={}", arg(&work)),
    ]);
    assert!(
        again.contains("0 repositories added") && again.contains("3 skipped"),
        "{again}"
    );
    assert_eq!(db::hooks::all(&conn).unwrap().len(), 3);
}

#[test]
fn export_to_stdout_and_import_from_stdin() {
    let a = machine_a();
    let exported = a.ok(&["export"]);
    let doc = json(&exported);
    assert_eq!(doc["format"], "vuta-registry");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["settings"], serde_json::json!({ "sync.jobs": "8" }));
    assert!(doc.get("last_rev").is_none() && doc["repos"][0].get("last_rev").is_none());

    // Without mappings, roots are created at their exported paths.
    let b = Env::new();
    let out = b
        .vuta()
        .args(["import", "-", "--no-hooks"])
        .write_stdin(exported)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(db::hooks::all(&b.conn()).unwrap().len(), 0);
    assert_eq!(
        db::repos::by_name(&b.conn(), "zed")
            .unwrap()
            .unwrap()
            .rel_path,
        "rust/zed"
    );
    assert_eq!(
        db::roots::by_name(&b.conn(), "src").unwrap().unwrap().path,
        a.root()
    );
}

#[test]
fn import_merges_into_an_existing_registry() {
    let a = machine_a();
    let file = export_file(&a);

    // Machine B already has a root at the place src should go, a v of its
    // own, something else at rust/zed, and its own sync.jobs.
    let b = Env::new();
    let code = b.dir.path().join("code");
    b.ok(&["root", "add", "code", arg(&code)]);
    b.ok(&[
        "add",
        "https://example.com/my-v.git",
        "v/v",
        "--name",
        "v",
        "--no-clone",
    ]);
    b.ok(&[
        "add",
        "https://example.com/other.git",
        "rust/zed",
        "--name",
        "other",
        "--no-clone",
    ]);
    b.ok(&["config", "set", "sync.jobs", "2"]);
    let work = b.dir.path().join("work");

    let out = b.ok(&[
        "import",
        arg(&file),
        "--map-root",
        &format!("src={}", arg(&code)),
        "--map-root",
        &format!("work={}", arg(&work)),
    ]);
    assert!(line_for(&out, "code").contains("use"), "{out}");
    assert!(line_for(&out, "v").contains("another URL"), "{out}");
    assert!(out.contains("'other' is already at code:rust/zed"), "{out}");
    assert!(
        line_for(&out, "sync.jobs").contains("local value 2 kept"),
        "{out}"
    );
    assert!(out.contains("1 repository added"), "{out}");

    let conn = b.conn();
    let tools = db::repos::by_name(&conn, "tools").unwrap().unwrap();
    assert_eq!(
        db::roots::by_id(&conn, tools.root_id)
            .unwrap()
            .unwrap()
            .name,
        "work"
    );
    // Skipped repositories bring no hooks; the existing default stays.
    assert!(db::hooks::all(&conn).unwrap().is_empty());
    assert_eq!(db::roots::default(&conn).unwrap().unwrap().name, "code");
    assert_eq!(b.ok(&["config", "get", "sync.jobs"]).trim(), "2");
}

#[test]
fn import_dry_run_and_errors_change_nothing() {
    let a = machine_a();
    let file = export_file(&a);
    let b = Env::new();
    let src = format!("src={}", arg(&b.dir.path().join("src")));
    let work = format!("work={}", arg(&b.dir.path().join("work")));

    let out = b.ok(&[
        "import",
        arg(&file),
        "--map-root",
        &src,
        "--map-root",
        &work,
        "--dry-run",
    ]);
    assert!(out.contains("dry run, nothing changed"), "{out}");
    assert!(names(&b).is_empty());
    assert!(db::roots::all(&b.conn()).unwrap().is_empty());

    // A bad hook at the end rejects the whole file.
    let mut doc = json(&std::fs::read_to_string(&file).unwrap());
    doc["repos"][2]["hooks"] = serde_json::json!([{ "command": "x", "platform": "mac" }]);
    let bad = b.dir.path().join("bad.json");
    std::fs::write(&bad, doc.to_string()).unwrap();
    let out = b.code(
        3,
        &["import", arg(&bad), "--map-root", &src, "--map-root", &work],
    );
    assert!(
        stderr(&out).contains("unknown platform 'mac'"),
        "{}",
        stderr(&out)
    );
    assert!(names(&b).is_empty());
    assert!(db::roots::all(&b.conn()).unwrap().is_empty());

    // A root path that is not absolute here needs a mapping.
    let mut doc = json(&std::fs::read_to_string(&file).unwrap());
    doc["roots"][0]["path"] = "relative/src".into();
    std::fs::write(&bad, doc.to_string()).unwrap();
    let out = b.code(3, &["import", arg(&bad), "--map-root", &work]);
    assert!(
        stderr(&out).contains("--map-root src=<path>"),
        "{}",
        stderr(&out)
    );

    for args in [
        &["import", arg(&file), "--map-root", "nope=/x"][..],
        &["import", arg(&file), "--map-root", "src"],
        &["import", arg(&b.dir.path().join("missing.json"))],
    ] {
        let out = b.code(3, args);
        assert_eq!(
            stderr(&out).lines().count(),
            1,
            "{args:?}: {}",
            stderr(&out)
        );
    }
    std::fs::write(&bad, "{\"format\": \"something-else\"}").unwrap();
    let out = b.code(3, &["import", arg(&bad)]);
    assert!(
        stderr(&out).contains("not a Vuta registry"),
        "{}",
        stderr(&out)
    );
    assert!(names(&b).is_empty());
}

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

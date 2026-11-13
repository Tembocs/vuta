//! M0: global flags, `vuta db path`, `vuta doctor`, exit codes and colour.

mod common;

use common::{Env, stderr};

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn db_path_prints_the_flag_value() {
    let env = Env::new();
    let out = env.vuta().args(["db", "path"]).assert().success();
    assert_eq!(
        stdout(out.get_output()).trim(),
        env.db().display().to_string()
    );
}

#[test]
fn db_path_reads_vuta_db() {
    let env = Env::new();
    let mut cmd = assert_cmd::Command::new(env!("CARGO_BIN_EXE_vuta"));
    let out = cmd
        .env("VUTA_DB", env.db())
        .args(["db", "path"])
        .assert()
        .success();
    assert_eq!(
        stdout(out.get_output()).trim(),
        env.db().display().to_string()
    );
}

#[test]
fn doctor_reports_a_healthy_empty_database() {
    let env = Env::new();
    let out = env
        .vuta()
        .args(["--color", "never", "doctor"])
        .assert()
        .success();
    let text = stdout(out.get_output());
    assert!(text.contains("git version"), "{text}");
    assert!(text.contains("(schema 1)"), "{text}");
    assert!(text.contains("none registered"), "{text}");
    assert!(env.db().is_file());
}

#[test]
fn color_flag_controls_escape_codes() {
    let env = Env::new();
    let never = env
        .vuta()
        .args(["--color", "never", "doctor"])
        .output()
        .unwrap();
    assert!(!never.stdout.contains(&0x1b));

    let always = env
        .vuta()
        .args(["--color", "always", "doctor"])
        .output()
        .unwrap();
    assert!(always.stdout.contains(&0x1b));
}

#[test]
fn usage_errors_exit_2() {
    let env = Env::new();
    env.vuta().arg("--no-such-flag").assert().code(2);
}

#[test]
fn doctor_reports_a_too_new_database() {
    let env = Env::new();
    env.vuta().arg("doctor").assert().success();
    let conn = rusqlite::Connection::open(env.db()).unwrap();
    conn.pragma_update(None, "user_version", 999).unwrap();
    drop(conn);
    let out = env
        .vuta()
        .args(["--color", "never", "doctor"])
        .assert()
        .code(1);
    let text = stdout(out.get_output());
    assert!(text.contains("upgrade vuta"), "{text}");
}

#[test]
fn completions_print_a_script_without_touching_the_database() {
    let env = Env::new();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = env.vuta().args(["completions", shell]).assert().success();
        let text = stdout(out.get_output());
        // The script calls `COMPLETE=<shell> vuta -- <words>` on TAB.
        assert!(
            text.contains("COMPLETE") && text.contains("vuta"),
            "{shell}"
        );
    }
    assert!(!env.db().exists());
    env.vuta().args(["completions", "tcsh"]).assert().code(2);
}

/// What the shell gets on TAB for `words`, the last being the word being
/// completed: one value per line, from fish's protocol, which needs no
/// other variables. Help text after a tab is dropped. The database is the
/// one the words name, as for a real shell.
fn complete(words: &[&str]) -> Vec<String> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vuta"))
        .env("COMPLETE", "fish")
        .env_remove("VUTA_DB")
        .arg("--")
        .args(words)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
    stdout(&out)
        .lines()
        .map(|line| line.split('\t').next().unwrap().to_owned())
        // Drop flags such as `--db` and `-q`, keeping values like `-rust`.
        .filter(|value| !(value.starts_with("--") || value.starts_with('-') && value.len() == 2))
        .collect()
}

#[test]
fn completion_reads_names_from_the_registry() {
    let env = Env::with_root();
    let db = env.db();
    let db = db.to_str().unwrap();
    env.ok(&[
        "root",
        "add",
        "work",
        env.dir.path().join("work").to_str().unwrap(),
    ]);
    env.ok(&[
        "add",
        "https://example.com/zed.git",
        "-t",
        "rust",
        "-t",
        "editor",
        "--no-clone",
    ]);
    env.ok(&[
        "add",
        "https://example.com/zig.git",
        "-t",
        "rust",
        "--no-clone",
    ]);
    env.ok(&["add", "https://example.com/v.git", "--no-clone"]);
    env.ok(&["hook", "add", "v", "make"]);

    assert_eq!(complete(&["vuta", "--db", db, "sync", "z"]), ["zed", "zig"]);
    // `--db=PATH` and `--db` after the subcommand count too.
    assert_eq!(
        complete(&["vuta", "show", &format!("--db={db}"), "v"]),
        ["v"]
    );
    assert_eq!(
        complete(&["vuta", "--db", db, "list", "--root", ""]),
        ["src", "work"]
    );
    assert_eq!(
        complete(&["vuta", "--db", db, "root", "remove", "w"]),
        ["work"]
    );
    assert_eq!(
        complete(&["vuta", "--db", db, "sync", "--tag", "r"]),
        ["rust"]
    );
    assert_eq!(
        complete(&["vuta", "--db", db, "tag", "zed", "+e"]),
        ["+editor"]
    );
    assert_eq!(
        complete(&["vuta", "--db", db, "tag", "zed", "-"]),
        ["-editor", "-rust"]
    );
    assert_eq!(complete(&["vuta", "--db", db, "hook", "remove", ""]), ["1"]);
    assert_eq!(
        complete(&["vuta", "config", "get", ""]),
        ["sync.jobs", "history.keep_runs", "ui.glyphs"]
    );

    // VUTA_DB is used when the line has no --db.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vuta"))
        .env("COMPLETE", "fish")
        .env("VUTA_DB", db)
        .args(["--", "vuta", "log", "zi"])
        .output()
        .unwrap();
    assert_eq!(stdout(&out).split('\t').next(), Some("zig"));
}

#[test]
fn completion_never_creates_a_database() {
    let env = Env::new();
    let db = env.db();
    for words in [
        &["vuta", "--db", db.to_str().unwrap(), "sync", ""][..],
        &["vuta", "--db", db.to_str().unwrap(), "root", "default", ""],
    ] {
        assert!(complete(words).is_empty(), "{words:?}");
    }
    assert!(!db.exists());
    assert!(!db.parent().unwrap().exists());
}

/// Each script must parse in its own shell. Shells that are not installed
/// are skipped.
#[cfg(unix)]
#[test]
fn completion_scripts_parse() {
    let env = Env::new();
    for (shell, check) in [
        ("bash", &["-n"][..]),
        ("zsh", &["-n"]),
        ("fish", &["--no-execute"]),
    ] {
        if std::process::Command::new(shell)
            .arg("-c")
            .arg("exit 0")
            .output()
            .is_err()
        {
            continue;
        }
        let script = env.dir.path().join(format!("vuta.{shell}"));
        let out = env.vuta().args(["completions", shell]).output().unwrap();
        std::fs::write(&script, out.stdout).unwrap();
        let status = std::process::Command::new(shell)
            .args(check)
            .arg(&script)
            .status()
            .unwrap();
        assert!(status.success(), "{shell} rejects its completion script");
    }
}

#[cfg(windows)]
#[test]
fn completion_script_parses_in_powershell() {
    let env = Env::new();
    let script = env.dir.path().join("vuta.ps1");
    let out = env
        .vuta()
        .args(["completions", "powershell"])
        .output()
        .unwrap();
    std::fs::write(&script, out.stdout).unwrap();
    let parse = format!(
        "$errors = $null; [System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$null, [ref]$errors) | Out-Null; if ($errors) {{ $errors; exit 1 }}",
        script.display()
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &parse])
        .status()
        .unwrap();
    assert!(status.success(), "PowerShell rejects its completion script");
}

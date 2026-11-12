//! M0: global flags, `vuta db path`, `vuta doctor`, exit codes and colour.

mod common;

use common::Env;

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
        assert!(
            text.contains("sync") && text.contains("fetch-only"),
            "{shell}"
        );
    }
    assert!(!env.db().exists());
    env.vuta().args(["completions", "tcsh"]).assert().code(2);
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

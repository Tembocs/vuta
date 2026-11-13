//! Shared test helpers: a temporary directory, a `vuta` command already
//! pointed at a database inside it, and the bare-upstream fixture that the
//! unit tests use too.

#![allow(dead_code, unused_imports)]

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use tempfile::TempDir;

#[path = "../../src/testutil.rs"]
mod testutil;

pub use testutil::{ALLOW_FILE_SUBMODULES, Fixture, git, identify, local_commit};

pub struct Env {
    pub dir: TempDir,
}

impl Env {
    pub fn new() -> Self {
        Env {
            dir: tempfile::tempdir().expect("create temp dir"),
        }
    }

    /// An environment with one root, `src`, at `<temp>/src`.
    pub fn with_root() -> Self {
        let env = Env::new();
        env.ok(&["root", "add", "src", env.root().to_str().unwrap()]);
        env
    }

    pub fn db(&self) -> PathBuf {
        self.dir.path().join("data").join("vuta.db")
    }

    /// The directory of the `src` root that `with_root` registers.
    pub fn root(&self) -> PathBuf {
        self.dir.path().join("src")
    }

    /// Where a checkout at `rel` (with `/` separators) under `src` lives.
    pub fn checkout(&self, rel: &str) -> PathBuf {
        let mut path = self.root();
        path.extend(rel.split('/'));
        path
    }

    /// `vuta --db <temp db>` with colour-related environment cleared, so
    /// output is plain because it is piped. Git may clone submodules from
    /// local paths, which the fixtures' submodules are.
    pub fn vuta(&self) -> Command {
        let (key, value) = ALLOW_FILE_SUBMODULES.split_once('=').unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vuta"));
        cmd.arg("--db")
            .arg(self.db())
            .env_remove("VUTA_DB")
            .env_remove("VUTA_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", key)
            .env("GIT_CONFIG_VALUE_0", value);
        cmd
    }

    /// Run vuta with `args` and return its output, whatever the exit code.
    pub fn run(&self, args: &[&str]) -> Output {
        self.vuta().args(args).output().expect("run vuta")
    }

    /// Run vuta, require success, and return stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "vuta {args:?} exited {:?}\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            stdout(&out),
            stderr(&out)
        );
        stdout(&out)
    }

    /// Run vuta, require exit code `code`, and return the output.
    pub fn code(&self, code: i32, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(code),
            "vuta {args:?}\nstdout:\n{}\nstderr:\n{}",
            stdout(&out),
            stderr(&out)
        );
        out
    }

    /// The database, opened through vuta's own `db` module.
    pub fn conn(&self) -> rusqlite::Connection {
        vuta::db::open(&self.db()).expect("open db")
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// The line of `text` that mentions `name` as a word, for checking one
/// repository's result line.
pub fn line_for<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|line| line.split_whitespace().any(|word| word == name))
        .unwrap_or_else(|| panic!("no line for {name} in:\n{text}"))
}

/// A path as an argument.
pub fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

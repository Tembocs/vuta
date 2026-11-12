//! Shared test helpers: a temporary directory and a `vuta` command already
//! pointed at a database inside it.

use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

pub struct Env {
    pub dir: TempDir,
}

impl Env {
    pub fn new() -> Self {
        Env {
            dir: tempfile::tempdir().expect("create temp dir"),
        }
    }

    pub fn db(&self) -> PathBuf {
        self.dir.path().join("data").join("vuta.db")
    }

    /// `vuta --db <temp db>` with colour-related environment cleared.
    pub fn vuta(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vuta"));
        cmd.arg("--db")
            .arg(self.db())
            .env_remove("VUTA_DB")
            .env_remove("VUTA_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE");
        cmd
    }
}

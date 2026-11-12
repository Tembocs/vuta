//! Test fixtures shared by unit tests: a bare upstream repository in a
//! temporary directory, and a helper to run git for setup.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::vcs::git::Git;
use crate::vcs::{CloneSpec, Vcs};

/// Run git for test setup with a fixed identity, failing the test on error.
pub fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Vuta Test",
            "-c",
            "user.email=test@example.com",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A bare upstream and a working "seed" clone used to push to it.
pub struct Fixture {
    pub dir: TempDir,
    commits: u32,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--bare", "upstream.git"]);
        git(dir.path(), &["clone", "upstream.git", "seed"]);
        let mut fixture = Fixture { dir, commits: 0 };
        fixture.push_commit();
        fixture
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub fn url(&self) -> String {
        self.path("upstream.git").to_str().unwrap().to_owned()
    }

    /// The upstream's current revision.
    pub fn upstream_rev(&self) -> String {
        git(&self.path("seed"), &["rev-parse", "HEAD"])
    }

    /// Commit to the seed and push; returns the new upstream revision.
    pub fn push_commit(&mut self) -> String {
        self.commits += 1;
        let seed = self.path("seed");
        fs::write(seed.join("file.txt"), format!("{}\n", self.commits)).unwrap();
        git(&seed, &["add", "file.txt"]);
        git(
            &seed,
            &["commit", "-m", &format!("commit {}", self.commits)],
        );
        git(&seed, &["push", "-q", "origin", "HEAD:main"]);
        self.upstream_rev()
    }

    /// A clone at `work/checkout`, made by the backend.
    pub fn checkout(&self) -> PathBuf {
        let dest = self.path("work").join("checkout");
        let url = self.url();
        let spec = CloneSpec {
            url: &url,
            branch: None,
            filter: None,
            depth: None,
        };
        Git::default().clone_into(&spec, &dest).unwrap();
        dest
    }
}

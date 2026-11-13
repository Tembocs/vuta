//! Test fixtures: a bare upstream repository in a temporary directory, and
//! a helper to run git for setup.
//!
//! Unit tests reach this as `crate::testutil`, and the integration tests
//! include the same file from `tests/common`, so it must not name anything
//! through `crate::`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

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
        let content = format!("{}\n", self.commits);
        self.push_file("file.txt", &content)
    }

    /// Write `name` in the seed, commit and push; returns the new upstream
    /// revision.
    pub fn push_file(&mut self, name: &str, content: &str) -> String {
        let seed = self.path("seed");
        fs::write(seed.join(name), content).unwrap();
        git(&seed, &["add", name]);
        git(&seed, &["commit", "-q", "-m", &format!("change {name}")]);
        git(&seed, &["push", "-q", "origin", "HEAD:main"]);
        self.upstream_rev()
    }

    /// Add `lib`'s upstream as a submodule at `path` in the seed, and push.
    /// Returns the new upstream revision. Git refuses submodules at local
    /// paths unless `protocol.file.allow` says otherwise, so this setup and
    /// any test that has Vuta clone such a submodule must pass it.
    pub fn push_submodule(&mut self, path: &str, lib: &Fixture) -> String {
        let seed = self.path("seed");
        git(
            &seed,
            &[
                "-c",
                ALLOW_FILE_SUBMODULES,
                "submodule",
                "add",
                "-q",
                &lib.url(),
                path,
            ],
        );
        git(
            &seed,
            &["commit", "-q", "-m", &format!("add submodule {path}")],
        );
        git(&seed, &["push", "-q", "origin", "HEAD:main"]);
        self.upstream_rev()
    }

    /// Point the seed's submodule at `path` to its upstream's newest
    /// commit, commit and push. Returns the new upstream revision.
    pub fn bump_submodule(&mut self, path: &str) -> String {
        let seed = self.path("seed");
        let sub = seed.join(path);
        git(&sub, &["fetch", "-q", "origin"]);
        git(&sub, &["checkout", "-q", "origin/main"]);
        git(&seed, &["commit", "-q", "-am", &format!("bump {path}")]);
        git(&seed, &["push", "-q", "origin", "HEAD:main"]);
        self.upstream_rev()
    }

    /// A clone at `work/checkout`.
    pub fn checkout(&self) -> PathBuf {
        self.clone_to(&self.path("work").join("checkout"))
    }

    /// A clone of the upstream at `dest`, which must not exist yet.
    pub fn clone_to(&self, dest: &Path) -> PathBuf {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let dest_text = dest.to_str().unwrap();
        git(self.dir.path(), &["clone", "-q", &self.url(), dest_text]);
        identify(dest);
        dest.to_path_buf()
    }
}

/// The git setting that lets submodules live at local paths.
pub const ALLOW_FILE_SUBMODULES: &str = "protocol.file.allow=always";

/// Give a checkout its own committer identity and turn off signing, so
/// that a rebase run by Vuta (which sets neither) can make commits on a
/// machine with no global git identity, such as a CI runner.
pub fn identify(checkout: &Path) {
    git(checkout, &["config", "user.name", "Vuta Test"]);
    git(checkout, &["config", "user.email", "test@example.com"]);
    git(checkout, &["config", "commit.gpgsign", "false"]);
}

/// Commit `content` to `name` in a checkout, without pushing.
pub fn local_commit(checkout: &Path, name: &str, content: &str) -> String {
    fs::write(checkout.join(name), content).unwrap();
    git(checkout, &["add", name]);
    git(checkout, &["commit", "-q", "-m", &format!("local {name}")]);
    git(checkout, &["rev-parse", "HEAD"])
}

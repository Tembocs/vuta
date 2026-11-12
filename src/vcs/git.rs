//! The Git backend. It runs the `git` executable, so SSH config and
//! credential helpers work as they do by hand. Every call goes through
//! `crate::proc`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow, bail};

use super::{CloneSpec, Divergence, Head, Vcs};
use crate::proc::{self, Captured, Tracer};

/// The output of `git --version`, or `Err` with `NotFound` when git is not
/// on PATH.
pub fn version() -> io::Result<String> {
    let out = proc::capture(proc::git_command().arg("--version"))?;
    if !out.status.success() {
        return Err(io::Error::other(out.last_line().to_owned()));
    }
    Ok(out.stdout.trim().to_owned())
}

/// Where a clone is made before it is renamed into place.
pub fn partial_path(dest: &Path) -> Option<PathBuf> {
    let name = dest.file_name()?;
    let mut partial = name.to_os_string();
    partial.push(".vuta-partial");
    Some(dest.with_file_name(partial))
}

/// A git command that exited non-zero. It displays as the last line of
/// git's error output; `stderr` keeps all of it.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{}", self.summary())]
pub struct GitError {
    pub command: String,
    pub code: Option<i32>,
    pub stderr: String,
}

impl GitError {
    fn from_captured(out: &Captured) -> Self {
        GitError {
            command: out.command.clone(),
            code: out.status.code(),
            stderr: if out.stderr.trim().is_empty() {
                out.stdout.clone()
            } else {
                out.stderr.clone()
            },
        }
    }

    pub fn summary(&self) -> String {
        match proc::last_line(&self.stderr) {
            Some(line) => line.to_owned(),
            None => match self.code {
                Some(code) => format!("git exited with code {code}"),
                None => "git was terminated by a signal".to_owned(),
            },
        }
    }
}

#[derive(Default)]
pub struct Git {
    tracer: Option<Tracer>,
}

impl Git {
    pub fn new(tracer: Option<Tracer>) -> Self {
        Git { tracer }
    }

    fn command(&self, cwd: Option<&Path>) -> Command {
        let mut cmd = proc::git_command();
        if let Some(path) = cwd {
            cmd.arg("-C").arg(path);
        }
        cmd
    }

    /// Run and trace, whatever the exit status.
    fn exec(&self, cmd: &mut Command) -> anyhow::Result<Captured> {
        let out = proc::capture(cmd).context("could not run git")?;
        if let Some(tracer) = &self.tracer {
            tracer(&out);
        }
        Ok(out)
    }

    /// Run and trace; a non-zero exit is a `GitError`.
    fn check(&self, cmd: &mut Command) -> anyhow::Result<Captured> {
        let out = self.exec(cmd)?;
        if out.status.success() {
            Ok(out)
        } else {
            Err(GitError::from_captured(&out).into())
        }
    }
}

impl Vcs for Git {
    fn is_checkout(&self, path: &Path) -> bool {
        let Ok(out) = self.exec(
            self.command(Some(path))
                .args(["rev-parse", "--show-toplevel"]),
        ) else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        // Compare canonical forms: git prints `/` separators on Windows and
        // resolves symlinks. A folder inside another checkout has a
        // different top level.
        let top = PathBuf::from(out.stdout.trim());
        match (fs::canonicalize(top), fs::canonicalize(path)) {
            (Ok(top), Ok(path)) => top == path,
            _ => false,
        }
    }

    fn remote_url(&self, path: &Path) -> anyhow::Result<Option<String>> {
        let out = self.exec(
            self.command(Some(path))
                .args(["remote", "get-url", "origin"]),
        )?;
        if out.status.success() {
            return Ok(Some(out.stdout.trim().to_owned()));
        }
        if out.stderr.contains("No such remote") {
            return Ok(None);
        }
        Err(GitError::from_captured(&out).into())
    }

    fn head(&self, path: &Path) -> anyhow::Result<Head> {
        let rev =
            self.exec(
                self.command(Some(path))
                    .args(["rev-parse", "--verify", "--quiet", "HEAD"]),
            )?;
        if !rev.status.success() {
            // Exit 1 with --quiet means HEAD names no commit yet.
            if rev.status.code() == Some(1) {
                return Ok(Head::Unborn);
            }
            return Err(GitError::from_captured(&rev).into());
        }
        let rev = rev.stdout.trim().to_owned();
        let branch = self.exec(self.command(Some(path)).args([
            "symbolic-ref",
            "--quiet",
            "--short",
            "HEAD",
        ]))?;
        match branch.status.code() {
            Some(0) => Ok(Head::Branch {
                name: branch.stdout.trim().to_owned(),
                rev,
            }),
            Some(1) => Ok(Head::Detached { rev }),
            _ => Err(GitError::from_captured(&branch).into()),
        }
    }

    fn is_dirty(&self, path: &Path) -> anyhow::Result<bool> {
        let out = self.check(self.command(Some(path)).args([
            "status",
            "--porcelain",
            "--untracked-files=no",
        ]))?;
        Ok(!out.stdout.trim().is_empty())
    }

    fn clone_into(&self, spec: &CloneSpec, dest: &Path) -> anyhow::Result<()> {
        let partial =
            partial_path(dest).ok_or_else(|| anyhow!("cannot clone into {}", dest.display()))?;
        if dest.exists() {
            bail!("{} already exists", dest.display());
        }
        if partial.exists() {
            fs::remove_dir_all(&partial)
                .with_context(|| format!("cannot remove stale {}", partial.display()))?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }

        let mut cmd = self.command(None);
        cmd.args(["clone", "--origin", "origin"]);
        if let Some(branch) = spec.branch {
            cmd.args(["--branch", branch]);
        }
        if let Some(filter) = spec.filter {
            cmd.arg(format!("--filter={filter}"));
        }
        if let Some(depth) = spec.depth {
            cmd.arg(format!("--depth={depth}"));
        }
        cmd.arg("--").arg(spec.url).arg(&partial);
        if let Err(err) = self.check(&mut cmd) {
            let _ = fs::remove_dir_all(&partial);
            return Err(err);
        }
        fs::rename(&partial, dest)
            .with_context(|| format!("cannot move {} to {}", partial.display(), dest.display()))
    }

    fn fetch(&self, path: &Path) -> anyhow::Result<()> {
        self.check(
            self.command(Some(path))
                .args(["fetch", "--prune", "origin"]),
        )?;
        Ok(())
    }

    fn divergence(&self, path: &Path) -> anyhow::Result<Option<Divergence>> {
        // Fails when HEAD is detached, the branch tracks nothing, or the
        // tracked branch is gone. The caller has already checked for a
        // checkout on a branch, so all of these mean "no upstream".
        let upstream = self.exec(self.command(Some(path)).args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ]))?;
        if !upstream.status.success() {
            return Ok(None);
        }
        let out = self.check(self.command(Some(path)).args([
            "rev-list",
            "--left-right",
            "--count",
            "HEAD...@{upstream}",
        ]))?;
        let mut counts = out.stdout.split_whitespace().map(str::parse::<u32>);
        match (counts.next(), counts.next()) {
            (Some(Ok(ahead)), Some(Ok(behind))) => Ok(Some(Divergence { ahead, behind })),
            _ => bail!("unexpected output from git rev-list: {}", out.stdout.trim()),
        }
    }

    fn fast_forward(&self, path: &Path) -> anyhow::Result<()> {
        self.check(
            self.command(Some(path))
                .args(["merge", "--ff-only", "@{upstream}"]),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;

    use super::*;

    /// Run git for test setup with a fixed identity, failing the test on error.
    fn git(cwd: &Path, args: &[&str]) -> String {
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
    struct Fixture {
        dir: TempDir,
        commits: u32,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            git(dir.path(), &["init", "--bare", "upstream.git"]);
            git(dir.path(), &["clone", "upstream.git", "seed"]);
            let mut fixture = Fixture { dir, commits: 0 };
            fixture.push_commit();
            fixture
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn url(&self) -> String {
            self.path("upstream.git").to_str().unwrap().to_owned()
        }

        /// Commit to the seed and push; returns the new upstream revision.
        fn push_commit(&mut self) -> String {
            self.commits += 1;
            let seed = self.path("seed");
            fs::write(seed.join("file.txt"), format!("{}\n", self.commits)).unwrap();
            git(&seed, &["add", "file.txt"]);
            git(
                &seed,
                &["commit", "-m", &format!("commit {}", self.commits)],
            );
            git(&seed, &["push", "-q", "origin", "HEAD:main"]);
            git(&seed, &["rev-parse", "HEAD"])
        }

        /// A clone at `checkout`, made by the backend.
        fn checkout(&self) -> PathBuf {
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

    fn head_rev(path: &Path) -> String {
        match Git::default().head(path).unwrap() {
            Head::Branch { rev, .. } | Head::Detached { rev } => rev,
            Head::Unborn => panic!("unborn"),
        }
    }

    #[test]
    fn clones_into_place() {
        let fixture = Fixture::new();
        let upstream_rev = git(&fixture.path("seed"), &["rev-parse", "HEAD"]);
        let checkout = fixture.checkout();
        let vcs = Git::default();

        assert!(vcs.is_checkout(&checkout));
        assert!(!partial_path(&checkout).unwrap().exists());
        assert_eq!(
            vcs.head(&checkout).unwrap(),
            Head::Branch {
                name: "main".into(),
                rev: upstream_rev
            }
        );
        assert_eq!(vcs.remote_url(&checkout).unwrap(), Some(fixture.url()));
        assert!(!vcs.is_dirty(&checkout).unwrap());
        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 0,
                behind: 0
            })
        );
    }

    #[test]
    fn fetches_and_fast_forwards() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        fixture.push_commit();
        let newest = fixture.push_commit();
        let vcs = Git::default();

        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 0,
                behind: 0
            })
        );
        vcs.fetch(&checkout).unwrap();
        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 0,
                behind: 2
            })
        );
        vcs.fast_forward(&checkout).unwrap();
        assert_eq!(head_rev(&checkout), newest);
        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 0,
                behind: 0
            })
        );
    }

    #[test]
    fn only_tracked_changes_are_dirty() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let vcs = Git::default();
        fs::write(checkout.join("build-output"), "x").unwrap();
        assert!(!vcs.is_dirty(&checkout).unwrap());
        fs::write(checkout.join("file.txt"), "changed").unwrap();
        assert!(vcs.is_dirty(&checkout).unwrap());
    }

    #[test]
    fn diverged_checkout_refuses_to_fast_forward() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        fixture.push_commit();
        fs::write(checkout.join("local.txt"), "local").unwrap();
        git(&checkout, &["add", "local.txt"]);
        git(&checkout, &["commit", "-m", "local"]);
        let vcs = Git::default();

        vcs.fetch(&checkout).unwrap();
        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 1,
                behind: 1
            })
        );
        let err = vcs.fast_forward(&checkout).unwrap_err();
        let git_err = err.downcast_ref::<GitError>().expect("a GitError");
        assert!(
            git_err.summary().contains("Not possible to fast-forward"),
            "{git_err:?}"
        );
    }

    #[test]
    fn detached_and_no_upstream() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let vcs = Git::default();
        let rev = head_rev(&checkout);

        git(&checkout, &["checkout", "-q", "--detach"]);
        assert_eq!(
            vcs.head(&checkout).unwrap(),
            Head::Detached { rev: rev.clone() }
        );
        assert_eq!(vcs.divergence(&checkout).unwrap(), None);

        git(&checkout, &["checkout", "-q", "-b", "local"]);
        assert_eq!(
            vcs.head(&checkout).unwrap(),
            Head::Branch {
                name: "local".into(),
                rev
            }
        );
        assert_eq!(vcs.divergence(&checkout).unwrap(), None);
    }

    #[test]
    fn recognises_only_checkout_roots() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let vcs = Git::default();
        let inner = checkout.join("sub");
        fs::create_dir(&inner).unwrap();
        let plain = fixture.path("plain");
        fs::create_dir(&plain).unwrap();

        assert!(vcs.is_checkout(&checkout));
        assert!(!vcs.is_checkout(&inner));
        assert!(!vcs.is_checkout(&plain));
        assert!(!vcs.is_checkout(&fixture.path("missing")));
    }

    #[test]
    fn unborn_head_and_missing_origin() {
        let fixture = Fixture::new();
        let empty = fixture.path("empty");
        git(fixture.dir.path(), &["init", "-q", "empty"]);
        let vcs = Git::default();
        assert_eq!(vcs.head(&empty).unwrap(), Head::Unborn);
        assert_eq!(vcs.remote_url(&empty).unwrap(), None);
    }

    #[test]
    fn failed_clone_leaves_nothing_behind() {
        let fixture = Fixture::new();
        let dest = fixture.path("work").join("nothing");
        let missing = fixture.path("no-such-upstream.git");
        let url = missing.to_str().unwrap();
        let spec = CloneSpec {
            url,
            branch: None,
            filter: None,
            depth: None,
        };
        let err = Git::default().clone_into(&spec, &dest).unwrap_err();
        assert!(err.downcast_ref::<GitError>().is_some(), "{err:#}");
        assert!(!dest.exists());
        assert!(!partial_path(&dest).unwrap().exists());
    }

    #[test]
    fn stale_partial_clone_is_replaced() {
        let fixture = Fixture::new();
        let dest = fixture.path("work").join("checkout");
        let partial = partial_path(&dest).unwrap();
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("junk"), "x").unwrap();
        let url = fixture.url();
        let spec = CloneSpec {
            url: &url,
            branch: Some("main"),
            filter: None,
            depth: Some(1),
        };
        Git::default().clone_into(&spec, &dest).unwrap();
        assert!(Git::default().is_checkout(&dest));
        assert!(!partial.exists());
        assert!(
            Git::default().clone_into(&spec, &dest).is_err(),
            "dest exists"
        );
    }

    #[test]
    fn tracer_sees_each_command() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let vcs = Git::new(Some(Arc::new(move |out: &Captured| {
            sink.lock().unwrap().push(out.command.clone());
        })));
        vcs.fetch(&checkout).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].ends_with("fetch --prune origin"), "{}", seen[0]);
    }

    #[test]
    fn git_error_shows_the_last_line() {
        let err = GitError {
            command: "git fetch".into(),
            code: Some(128),
            stderr: "fatal: one\nfatal: could not resolve host\n".into(),
        };
        assert_eq!(err.to_string(), "fatal: could not resolve host");
        let silent = GitError {
            command: "git x".into(),
            code: Some(1),
            stderr: String::new(),
        };
        assert_eq!(silent.to_string(), "git exited with code 1");
    }
}

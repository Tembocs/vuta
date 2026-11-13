//! The Git backend. It runs the `git` executable, so SSH config and
//! credential helpers work as they do by hand. Every call goes through
//! `crate::proc`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow, bail};

use super::{CloneSpec, Divergence, Head, Rebase, Submodule, SubmoduleStatus, Vcs};
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

    fn is_dirty(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<bool> {
        let mut cmd = self.command(Some(path));
        cmd.args(["status", "--porcelain", "--untracked-files=no"]);
        if ignore_submodules {
            cmd.arg("--ignore-submodules=all");
        }
        let out = self.check(&mut cmd)?;
        Ok(!out.stdout.trim().is_empty())
    }

    fn local_changes(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<Vec<String>> {
        // Status, not diff: it also sees a staged change that the working
        // tree has since undone, which a fast-forward still refuses over.
        let mut cmd = self.command(Some(path));
        cmd.args([
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=no",
            "--no-renames",
        ]);
        if ignore_submodules {
            cmd.arg("--ignore-submodules=all");
        }
        let out = self.check(&mut cmd)?;
        Ok(status_paths(&out.stdout))
    }

    fn incoming_changes(&self, path: &Path) -> anyhow::Result<Vec<String>> {
        let out = self.check(self.command(Some(path)).args([
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "@{upstream}",
            "--",
        ]))?;
        Ok(out
            .stdout
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect())
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
        // Submodules are not fetched here: an unreachable submodule remote
        // must not fail the repository itself. The submodule step fetches
        // what it needs.
        self.check(self.command(Some(path)).args([
            "fetch",
            "--prune",
            "--no-recurse-submodules",
            "origin",
        ]))?;
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
        // merge.autoStash would stash local changes and could leave
        // conflict markers when it pops them; Vuta never stashes.
        self.check(self.command(Some(path)).args([
            "-c",
            "merge.autoStash=false",
            "merge",
            "--ff-only",
            "@{upstream}",
        ]))?;
        Ok(())
    }

    fn rebase(&self, path: &Path) -> anyhow::Result<Rebase> {
        // Settings that would make a plain rebase stash local changes,
        // squash commits or move other branches are turned off.
        let out = self.exec(self.command(Some(path)).args([
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rebase.autoSquash=false",
            "-c",
            "rebase.updateRefs=false",
            "rebase",
            "@{upstream}",
        ]))?;
        if out.status.success() {
            return Ok(Rebase::Done);
        }
        let why = rebase_stop_reason(&out);
        if self.rebase_in_progress(path)? {
            let abort = self.exec(self.command(Some(path)).args(["rebase", "--abort"]))?;
            if !abort.status.success() || self.rebase_in_progress(path)? {
                bail!(
                    "git stopped the rebase ({why}) and could not abort it: {}\n\
                     finish or abort it by hand: git -C {} rebase --abort",
                    GitError::from_captured(&abort).summary(),
                    path.display()
                );
            }
        }
        Ok(Rebase::Stopped { why })
    }

    fn submodules(&self, path: &Path) -> anyhow::Result<Vec<Submodule>> {
        let out =
            self.check(
                self.command(Some(path))
                    .args(["submodule", "status", "--recursive"]),
            )?;
        Ok(parse_submodule_status(&out.stdout))
    }

    fn head_is_referenced(&self, path: &Path) -> anyhow::Result<bool> {
        let branch =
            self.exec(
                self.command(Some(path))
                    .args(["symbolic-ref", "--quiet", "HEAD"]),
            )?;
        if branch.status.success() {
            return Ok(true);
        }
        let refs = self.check(self.command(Some(path)).args([
            "for-each-ref",
            "--count=1",
            "--contains",
            "HEAD",
            "--format=%(refname)",
        ]))?;
        Ok(!refs.stdout.trim().is_empty())
    }

    fn update_submodules(&self, path: &Path) -> anyhow::Result<()> {
        // --checkout overrides an `update = merge` or `rebase` mode from
        // .gitmodules, which could stop part-way inside a submodule.
        self.check(self.command(Some(path)).args([
            "submodule",
            "update",
            "--init",
            "--recursive",
            "--checkout",
        ]))?;
        Ok(())
    }
}

impl Git {
    /// Whether a rebase has stopped part-way in this checkout.
    fn rebase_in_progress(&self, path: &Path) -> anyhow::Result<bool> {
        for dir in ["rebase-merge", "rebase-apply"] {
            let out =
                self.check(
                    self.command(Some(path))
                        .args(["rev-parse", "--git-path", dir]),
                )?;
            if path.join(out.stdout.trim()).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// The paths of `git status --porcelain -z` output. With `--no-renames`
/// every entry is `XY path`.
fn status_paths(text: &str) -> Vec<String> {
    text.split('\0')
        .filter_map(|entry| entry.get(3..))
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A short reason for a failed rebase: the files in conflict, else git's
/// first error line.
fn rebase_stop_reason(out: &Captured) -> String {
    let text = format!("{}\n{}", out.stdout, out.stderr);
    let lines = || text.lines().map(str::trim);
    let conflicts: Vec<&str> = lines().filter(|l| l.starts_with("CONFLICT (")).collect();
    if !conflicts.is_empty() {
        let files: Vec<&str> = conflicts
            .iter()
            .filter_map(|l| l.split_once("Merge conflict in ").map(|(_, f)| f))
            .collect();
        return match files.as_slice() {
            [file] if conflicts.len() == 1 => format!("conflict in {file}"),
            _ => crate::model::count(conflicts.len() as u32, "conflict"),
        };
    }
    lines()
        .find_map(|l| {
            l.strip_prefix("error: ")
                .or_else(|| l.strip_prefix("fatal: "))
        })
        .map(|l| l.trim_end_matches('.').to_owned())
        .unwrap_or_else(|| GitError::from_captured(out).summary())
}

/// Parse `git submodule status --recursive`: a status character, the
/// commit, the path, and for checked-out submodules an optional
/// `(describe)` suffix.
fn parse_submodule_status(text: &str) -> Vec<Submodule> {
    text.lines()
        .filter_map(|line| {
            let mut chars = line.chars();
            let status = match chars.next()? {
                ' ' => SubmoduleStatus::Current,
                '-' => SubmoduleStatus::Uninitialized,
                '+' => SubmoduleStatus::Moved,
                'U' => SubmoduleStatus::Conflict,
                _ => return None,
            };
            let (rev, mut path) = chars.as_str().split_once(' ')?;
            if matches!(status, SubmoduleStatus::Current | SubmoduleStatus::Moved)
                && path.ends_with(')')
                && let Some(at) = path.rfind(" (")
            {
                path = &path[..at];
            }
            Some(Submodule {
                path: path.to_owned(),
                rev: rev.to_owned(),
                status,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::testutil::{Fixture, git, local_commit};

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
        let checkout = fixture.path("work").join("checkout");
        let url = fixture.url();
        let spec = CloneSpec {
            url: &url,
            branch: None,
            filter: None,
            depth: None,
        };
        let vcs = Git::default();
        vcs.clone_into(&spec, &checkout).unwrap();

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
        assert!(!vcs.is_dirty(&checkout, false).unwrap());
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
        assert!(!vcs.is_dirty(&checkout, false).unwrap());
        fs::write(checkout.join("file.txt"), "changed").unwrap();
        assert!(vcs.is_dirty(&checkout, false).unwrap());
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

    /// A checkout with `extra.txt` tracked as well as `file.txt`.
    fn two_file_checkout(fixture: &mut Fixture) -> PathBuf {
        fixture.push_file("extra.txt", "extra\n");
        fixture.checkout()
    }

    #[test]
    fn fast_forwards_around_local_changes_upstream_does_not_touch() {
        let mut fixture = Fixture::new();
        let checkout = two_file_checkout(&mut fixture);
        let newest = fixture.push_commit();
        fs::write(checkout.join("extra.txt"), "local edit\n").unwrap();
        let vcs = Git::default();
        vcs.fetch(&checkout).unwrap();

        assert_eq!(vcs.local_changes(&checkout, false).unwrap(), ["extra.txt"]);
        assert_eq!(vcs.incoming_changes(&checkout).unwrap(), ["file.txt"]);
        vcs.fast_forward(&checkout).unwrap();
        assert_eq!(head_rev(&checkout), newest);
        assert_eq!(
            fs::read_to_string(checkout.join("extra.txt")).unwrap(),
            "local edit\n"
        );
    }

    #[test]
    fn fast_forward_refuses_to_overwrite_local_changes() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let before = head_rev(&checkout);
        fs::write(checkout.join("file.txt"), "local edit\n").unwrap();
        // A staged change that the working tree has undone still counts.
        fs::write(checkout.join("staged.txt"), "x").unwrap();
        git(&checkout, &["add", "staged.txt"]);
        fixture.push_commit();
        let vcs = Git::default();
        vcs.fetch(&checkout).unwrap();

        let mut local = vcs.local_changes(&checkout, false).unwrap();
        local.sort();
        assert_eq!(local, ["file.txt", "staged.txt"]);
        let err = vcs.fast_forward(&checkout).unwrap_err();
        let git_err = err.downcast_ref::<GitError>().expect("a GitError");
        assert!(
            git_err.stderr.contains("would be overwritten"),
            "{git_err:?}"
        );
        assert_eq!(head_rev(&checkout), before);
        assert_eq!(
            fs::read_to_string(checkout.join("file.txt")).unwrap(),
            "local edit\n"
        );
    }

    #[test]
    fn rebase_replays_local_commits() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        local_commit(&checkout, "local.txt", "mine\n");
        let upstream = fixture.push_commit();
        let vcs = Git::default();
        vcs.fetch(&checkout).unwrap();

        assert_eq!(vcs.rebase(&checkout).unwrap(), Rebase::Done);
        assert_eq!(
            vcs.divergence(&checkout).unwrap(),
            Some(Divergence {
                ahead: 1,
                behind: 0
            })
        );
        assert_eq!(git(&checkout, &["rev-parse", "HEAD~1"]), upstream);
        assert!(!vcs.rebase_in_progress(&checkout).unwrap());
    }

    #[test]
    fn a_stopped_rebase_is_aborted_and_changes_nothing() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let mine = local_commit(&checkout, "file.txt", "mine\n");
        fs::write(checkout.join("untracked.txt"), "keep me").unwrap();
        fixture.push_commit();
        let vcs = Git::default();
        vcs.fetch(&checkout).unwrap();

        assert_eq!(
            vcs.rebase(&checkout).unwrap(),
            Rebase::Stopped {
                why: "conflict in file.txt".into()
            }
        );
        assert!(!vcs.rebase_in_progress(&checkout).unwrap());
        assert_eq!(
            vcs.head(&checkout).unwrap(),
            Head::Branch {
                name: "main".into(),
                rev: mine
            }
        );
        assert!(!vcs.is_dirty(&checkout, false).unwrap());
        assert_eq!(
            fs::read_to_string(checkout.join("file.txt")).unwrap(),
            "mine\n"
        );
        assert_eq!(
            fs::read_to_string(checkout.join("untracked.txt")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn rebase_refuses_local_changes() {
        let mut fixture = Fixture::new();
        let checkout = two_file_checkout(&mut fixture);
        let mine = local_commit(&checkout, "local.txt", "mine\n");
        fs::write(checkout.join("extra.txt"), "local edit\n").unwrap();
        fixture.push_commit();
        let vcs = Git::default();
        vcs.fetch(&checkout).unwrap();

        match vcs.rebase(&checkout).unwrap() {
            Rebase::Stopped { why } => assert!(why.contains("unstaged changes"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(head_rev(&checkout), mine);
        assert_eq!(
            fs::read_to_string(checkout.join("extra.txt")).unwrap(),
            "local edit\n"
        );
    }

    #[test]
    fn head_is_referenced_unless_only_a_detached_head_holds_it() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let vcs = Git::default();
        assert!(vcs.head_is_referenced(&checkout).unwrap());
        git(&checkout, &["checkout", "-q", "--detach"]);
        assert!(vcs.head_is_referenced(&checkout).unwrap(), "main holds it");
        local_commit(&checkout, "orphan.txt", "x");
        assert!(!vcs.head_is_referenced(&checkout).unwrap());
    }

    #[test]
    fn parses_submodule_status() {
        let text = "-1111111111111111111111111111111111111111 lib\n\
                    +2222222222222222222222222222222222222222 vendor/a b (heads/main)\n \
                    3333333333333333333333333333333333333333 vendor/c\n\
                    U0000000000000000000000000000000000000000 conflicted\n";
        let parsed = parse_submodule_status(text);
        let summary: Vec<_> = parsed
            .iter()
            .map(|s| (s.path.as_str(), s.status, &s.rev[..1]))
            .collect();
        assert_eq!(
            summary,
            [
                ("lib", SubmoduleStatus::Uninitialized, "1"),
                ("vendor/a b", SubmoduleStatus::Moved, "2"),
                ("vendor/c", SubmoduleStatus::Current, "3"),
                ("conflicted", SubmoduleStatus::Conflict, "0"),
            ]
        );
        assert!(parse_submodule_status("").is_empty());
    }

    #[test]
    fn a_checkout_without_submodules_has_none() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let vcs = Git::default();
        assert!(vcs.submodules(&checkout).unwrap().is_empty());
        vcs.update_submodules(&checkout).unwrap();
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
        assert!(
            seen[0].ends_with("fetch --prune --no-recurse-submodules origin"),
            "{}",
            seen[0]
        );
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

//! Per-repository sync decisions (docs/ARCHITECTURE.md section 7).
//!
//! Checks run top to bottom and the first that fails decides the outcome.
//! A working tree is changed only by a clean fast-forward, or by a clone
//! into a path that does not exist yet.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use super::Event;
use crate::model::{FailReason, Outcome, Repo, SkipReason};
use crate::vcs::git::GitError;
use crate::vcs::{CloneSpec, Head, Vcs, remote};

/// One repository to sync and where its checkout lives.
#[derive(Debug, Clone)]
pub struct Job {
    pub repo: Repo,
    /// The root's path joined with the repository's `rel_path`.
    pub path: PathBuf,
}

/// Sync one repository and return its outcome. `notify` receives
/// `Event::Cloning` when a clone starts. `stop` is checked between steps.
pub fn sync_one(
    job: &Job,
    vcs: &dyn Vcs,
    fetch_only: bool,
    stop: &AtomicBool,
    notify: &mut dyn FnMut(Event),
) -> Outcome {
    let repo = &job.repo;
    let path = job.path.as_path();
    let stopped = || stop.load(Ordering::Relaxed);
    let interrupted = || Outcome::Failed {
        reason: FailReason::Interrupted,
        message: String::new(),
    };
    // A git child killed by Ctrl-C fails; report that as an interruption.
    let fail = |reason: FailReason, err: anyhow::Error| {
        if stopped() {
            interrupted()
        } else {
            failed(reason, err)
        }
    };

    if !repo.enabled {
        return skipped(SkipReason::Disabled, 0);
    }
    if stopped() {
        return interrupted();
    }

    // Missing path: clone.
    if !path.exists() {
        notify(Event::Cloning { repo_id: repo.id });
        let spec = CloneSpec {
            url: &repo.url,
            branch: repo.branch.as_deref(),
            filter: repo.clone_filter.as_deref(),
            depth: repo.clone_depth,
        };
        if let Err(err) = vcs.clone_into(&spec, path) {
            return fail(FailReason::Clone, err);
        }
        return match vcs.head(path) {
            Ok(head) => Outcome::Cloned {
                rev: head_rev(&head).unwrap_or_default(),
            },
            Err(err) => fail(FailReason::Clone, err),
        };
    }

    // 1. The path must be a checkout's top level.
    if !vcs.is_checkout(path) {
        return Outcome::Failed {
            reason: FailReason::NotACheckout,
            message: format!("{} is not a Git checkout", path.display()),
        };
    }

    // 2. origin must be the registered remote.
    match vcs.remote_url(path) {
        Ok(Some(found)) if remote::same_repo(&found, &repo.url) => {}
        Ok(found) => {
            return skipped(
                SkipReason::RemoteMismatch {
                    found: found.unwrap_or_else(|| "missing".to_owned()),
                    expected: repo.url.clone(),
                },
                0,
            );
        }
        Err(err) => return fail(FailReason::Fetch, err),
    }

    // 3. HEAD must be on a branch.
    let (branch, old_rev) = match vcs.head(path) {
        Ok(Head::Branch { name, rev }) => (name, rev),
        Ok(Head::Detached { rev }) => return skipped(SkipReason::Detached { rev }, 0),
        Ok(Head::Unborn) => {
            return skipped(
                SkipReason::NoUpstream {
                    branch: "HEAD".to_owned(),
                },
                0,
            );
        }
        Err(err) => return fail(FailReason::Fetch, err),
    };

    if stopped() {
        return interrupted();
    }
    if let Err(err) = vcs.fetch(path) {
        return fail(FailReason::Fetch, err);
    }

    // 4. The branch must track a remote branch.
    let divergence = match vcs.divergence(path) {
        Ok(Some(d)) => d,
        Ok(None) => return skipped(SkipReason::NoUpstream { branch }, 0),
        Err(err) => return fail(FailReason::Fetch, err),
    };
    let behind = divergence.behind;

    // 5. Nothing to bring in: up to date, possibly with local commits.
    if behind == 0 {
        return Outcome::UpToDate {
            rev: old_rev,
            ahead: divergence.ahead,
        };
    }

    // 6. Local and upstream commits both exist.
    if divergence.ahead > 0 {
        return skipped(
            SkipReason::Diverged {
                ahead: divergence.ahead,
            },
            behind,
        );
    }

    // 7. Tracked files must be unmodified.
    match vcs.is_dirty(path) {
        Ok(false) => {}
        Ok(true) => return skipped(SkipReason::Dirty, behind),
        Err(err) => return fail(FailReason::FastForward, err),
    }

    if fetch_only {
        return skipped(SkipReason::FetchOnly, behind);
    }
    if stopped() {
        return interrupted();
    }
    if let Err(err) = vcs.fast_forward(path) {
        return fail(FailReason::FastForward, err);
    }
    match vcs.head(path) {
        Ok(head) => Outcome::Updated {
            old: old_rev,
            new: head_rev(&head).unwrap_or_default(),
            commits: behind,
        },
        Err(err) => fail(FailReason::FastForward, err),
    }
}

fn skipped(reason: SkipReason, behind: u32) -> Outcome {
    Outcome::Skipped { reason, behind }
}

/// A failure carrying git's full error output when there is one.
fn failed(reason: FailReason, err: anyhow::Error) -> Outcome {
    let message = match err.downcast_ref::<GitError>() {
        Some(git) => git.stderr.trim_end().to_owned(),
        None => format!("{err:#}"),
    };
    Outcome::Failed { reason, message }
}

fn head_rev(head: &Head) -> Option<String> {
    match head {
        Head::Branch { rev, .. } | Head::Detached { rev } => Some(rev.clone()),
        Head::Unborn => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::path::Path;

    use super::*;
    use crate::model::VcsKind;
    use crate::testutil::{Fixture, git};
    use crate::vcs::git::Git;

    fn job(fixture: &Fixture, path: PathBuf) -> Job {
        Job {
            repo: Repo {
                id: 1,
                name: "checkout".into(),
                root_id: 1,
                rel_path: "work/checkout".into(),
                vcs: VcsKind::Git,
                url: fixture.url(),
                branch: None,
                clone_filter: None,
                clone_depth: None,
                enabled: true,
            },
            path,
        }
    }

    fn sync(job: &Job) -> (Outcome, Vec<Event>) {
        sync_with(job, false, &AtomicBool::new(false))
    }

    fn sync_with(job: &Job, fetch_only: bool, stop: &AtomicBool) -> (Outcome, Vec<Event>) {
        let mut events = Vec::new();
        let outcome = sync_one(job, &Git::default(), fetch_only, stop, &mut |e| {
            events.push(e)
        });
        (outcome, events)
    }

    fn rev(path: &Path) -> String {
        git(path, &["rev-parse", "HEAD"])
    }

    #[test]
    fn missing_path_is_cloned() {
        let fixture = Fixture::new();
        let job = job(&fixture, fixture.path("work").join("new"));
        let (outcome, events) = sync(&job);
        assert_eq!(
            outcome,
            Outcome::Cloned {
                rev: fixture.upstream_rev()
            }
        );
        assert!(matches!(events[..], [Event::Cloning { repo_id: 1 }]));
        assert!(!crate::vcs::git::partial_path(&job.path).unwrap().exists());
    }

    #[test]
    fn upstream_commits_are_fast_forwarded() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let old = rev(&checkout);
        fixture.push_commit();
        let new = fixture.push_commit();
        let (outcome, events) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::Updated {
                old,
                new: new.clone(),
                commits: 2
            }
        );
        assert!(events.is_empty());
        assert_eq!(rev(&checkout), new);
    }

    #[test]
    fn nothing_new_is_up_to_date() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let (outcome, _) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::UpToDate {
                rev: rev(&checkout),
                ahead: 0
            }
        );
    }

    #[test]
    fn local_commits_alone_are_up_to_date() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        fs::write(checkout.join("local.txt"), "x").unwrap();
        git(&checkout, &["add", "local.txt"]);
        git(&checkout, &["commit", "-m", "local"]);
        let (outcome, _) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::UpToDate {
                rev: rev(&checkout),
                ahead: 1
            }
        );
    }

    #[test]
    fn dirty_checkout_is_skipped_and_untouched() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let before = rev(&checkout);
        fs::write(checkout.join("file.txt"), "local edit").unwrap();
        fixture.push_commit();
        let (outcome, _) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::Dirty,
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), before);
        assert_eq!(
            fs::read_to_string(checkout.join("file.txt")).unwrap(),
            "local edit"
        );
    }

    #[test]
    fn diverged_checkout_is_skipped() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        fs::write(checkout.join("local.txt"), "x").unwrap();
        git(&checkout, &["add", "local.txt"]);
        git(&checkout, &["commit", "-m", "local"]);
        fixture.push_commit();
        fixture.push_commit();
        let (outcome, _) = sync(&job(&fixture, checkout));
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::Diverged { ahead: 1 },
                behind: 2
            }
        );
    }

    #[test]
    fn detached_and_untracked_branches_are_skipped() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        let head = rev(&checkout);
        git(&checkout, &["checkout", "-q", "--detach"]);
        let (outcome, _) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::Detached { rev: head },
                behind: 0
            }
        );

        git(&checkout, &["checkout", "-q", "-b", "local"]);
        let (outcome, _) = sync(&job(&fixture, checkout));
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::NoUpstream {
                    branch: "local".into()
                },
                behind: 0
            }
        );
    }

    #[test]
    fn other_origin_is_skipped() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        git(
            &checkout,
            &[
                "remote",
                "set-url",
                "origin",
                "https://example.com/other.git",
            ],
        );
        let (outcome, _) = sync(&job(&fixture, checkout));
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::RemoteMismatch {
                    found: "https://example.com/other.git".into(),
                    expected: fixture.url(),
                },
                behind: 0,
            }
        );
    }

    #[test]
    fn plain_directory_fails() {
        let fixture = Fixture::new();
        let plain = fixture.path("plain");
        fs::create_dir(&plain).unwrap();
        fs::write(plain.join("readme"), "x").unwrap();
        let (outcome, _) = sync(&job(&fixture, plain));
        assert!(
            matches!(
                outcome,
                Outcome::Failed {
                    reason: FailReason::NotACheckout,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn deleted_upstream_fails_to_fetch() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        fs::remove_dir_all(fixture.path("upstream.git")).unwrap();
        let (outcome, _) = sync(&job(&fixture, checkout));
        match outcome {
            Outcome::Failed {
                reason: FailReason::Fetch,
                message,
            } => {
                assert!(!message.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fetch_only_leaves_the_tree_alone() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let before = rev(&checkout);
        fixture.push_commit();
        let (outcome, _) = sync_with(
            &job(&fixture, checkout.clone()),
            true,
            &AtomicBool::new(false),
        );
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::FetchOnly,
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), before);
    }

    #[test]
    fn disabled_and_interrupted() {
        let fixture = Fixture::new();
        let mut disabled = job(&fixture, fixture.path("never"));
        disabled.repo.enabled = false;
        assert_eq!(
            sync(&disabled).0,
            Outcome::Skipped {
                reason: SkipReason::Disabled,
                behind: 0
            }
        );

        let (outcome, events) = sync_with(
            &job(&fixture, fixture.path("never")),
            false,
            &AtomicBool::new(true),
        );
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: FailReason::Interrupted,
                message: String::new()
            }
        );
        assert!(events.is_empty());
        assert!(!fixture.path("never").exists());
    }
}

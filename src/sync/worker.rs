//! Per-repository sync decisions (docs/ARCHITECTURE.md section 7).
//!
//! Checks run top to bottom and the first that fails decides the outcome.
//! A working tree is changed only by a fast-forward (around local changes
//! only when the repository allows it and upstream does not touch them),
//! by a rebase of a clean tree when the repository asks for one, by a
//! clone into a path that does not exist yet, or by the submodule step of
//! a repository that turned it on. Anything git refuses or stops part-way
//! leaves the checkout as it was.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::Event;
use crate::model::{FailReason, Outcome, Repo, SkipReason, Strategy, Submodules, short_rev};
use crate::vcs::git::GitError;
use crate::vcs::{CloneSpec, Head, Rebase, SubmoduleStatus, Vcs, remote};

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
                // A fresh clone has no submodule work to lose.
                submodules: repo
                    .submodules
                    .then(|| update_submodules(vcs, path, &[], stop)),
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
    let ahead = divergence.ahead;

    // 5. Nothing to bring in: up to date, possibly with local commits.
    // Submodules left missing or moved by an earlier sync are brought back.
    if behind == 0 {
        let submodules = if repo.submodules && !fetch_only && !stopped() {
            heal_submodules(vcs, path, stop)
        } else {
            None
        };
        return Outcome::UpToDate {
            rev: old_rev,
            ahead,
            submodules,
        };
    }

    // 6. Local and upstream commits both exist: only a rebase can follow.
    let rebase = ahead > 0;
    if rebase && repo.strategy != Strategy::Rebase {
        return skipped(SkipReason::Diverged { ahead }, behind);
    }

    // 7. Tracked files must be unmodified. A repository that allows it
    // fast-forwards around changes to files upstream does not touch; a
    // rebase always needs a clean tree. With submodules turned on, a
    // submodule at another commit is the submodule step's business.
    let ignore_submodules = repo.submodules;
    match vcs.is_dirty(path, ignore_submodules) {
        Ok(false) => {}
        Ok(true) if rebase || !repo.allow_dirty => {
            return skipped(SkipReason::Dirty { overlap: vec![] }, behind);
        }
        Ok(true) => match overlap(vcs, path, ignore_submodules) {
            Ok(overlap) if overlap.is_empty() => {}
            Ok(overlap) => return skipped(SkipReason::Dirty { overlap }, behind),
            Err(err) => return fail(FailReason::FastForward, err),
        },
        Err(err) => return fail(FailReason::FastForward, err),
    }

    if fetch_only {
        return skipped(SkipReason::FetchOnly, behind);
    }
    if stopped() {
        return interrupted();
    }

    // Submodules at their recorded commits now may move with the update.
    let pristine = if repo.submodules {
        current_submodules(vcs, path)
    } else {
        Vec::new()
    };

    if rebase {
        match vcs.rebase(path) {
            Ok(Rebase::Done) => {}
            Ok(Rebase::Stopped { why }) => {
                // The abort must have put the branch back where it was.
                return match vcs.head(path) {
                    Ok(Head::Branch { name, rev }) if name == branch && rev == old_rev => {
                        skipped(SkipReason::RebaseStopped { ahead, why }, behind)
                    }
                    Ok(head) => Outcome::Failed {
                        reason: FailReason::Rebase,
                        message: format!(
                            "the rebase stopped ({why}) and was aborted, but HEAD is {}, \
                             not branch {branch} at {}",
                            describe_head(&head),
                            short_rev(&old_rev)
                        ),
                    },
                    Err(err) => failed(FailReason::Rebase, err),
                };
            }
            // Never reported as an interruption: the message says how to
            // finish a rebase that could not be aborted.
            Err(err) => return failed(FailReason::Rebase, err),
        }
    } else if let Err(err) = vcs.fast_forward(path) {
        return fail(FailReason::FastForward, err);
    }

    let reason = if rebase {
        FailReason::Rebase
    } else {
        FailReason::FastForward
    };
    match vcs.head(path) {
        Ok(head) => Outcome::Updated {
            old: old_rev,
            new: head_rev(&head).unwrap_or_default(),
            commits: behind,
            rebased: if rebase { ahead } else { 0 },
            submodules: repo
                .submodules
                .then(|| update_submodules(vcs, path, &pristine, stop)),
        },
        Err(err) => fail(reason, err),
    }
}

/// Locally changed paths that the upstream changes too.
fn overlap(vcs: &dyn Vcs, path: &Path, ignore_submodules: bool) -> anyhow::Result<Vec<String>> {
    let incoming = vcs.incoming_changes(path)?;
    let mut overlap: Vec<String> = vcs
        .local_changes(path, ignore_submodules)?
        .into_iter()
        .filter(|p| incoming.contains(p))
        .collect();
    overlap.sort();
    overlap.dedup();
    Ok(overlap)
}

/// Paths of submodules checked out at their recorded commits. A failure to
/// list them leaves the list empty, which only makes the step stricter.
fn current_submodules(vcs: &dyn Vcs, path: &Path) -> Vec<String> {
    vcs.submodules(path)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.status == SubmoduleStatus::Current)
        .map(|s| s.path)
        .collect()
}

/// Run the submodule step on an up-to-date checkout when a submodule is
/// missing or not at its recorded commit, as after a failed step.
fn heal_submodules(vcs: &dyn Vcs, path: &Path, stop: &AtomicBool) -> Option<Submodules> {
    match vcs.submodules(path) {
        Ok(subs)
            if subs.iter().any(|s| {
                matches!(
                    s.status,
                    SubmoduleStatus::Uninitialized | SubmoduleStatus::Moved
                )
            }) =>
        {
            Some(update_submodules(vcs, path, &[], stop))
        }
        Ok(_) => None,
        Err(err) => Some(Submodules::Failed {
            message: error_text(&err),
        }),
    }
}

/// Update submodules, unless that could lose a commit. `pristine` lists
/// submodules that were at their recorded commits before the checkout
/// moved, so moving them on loses nothing. Any other submodule git would
/// move must have its HEAD held by a branch, tag or remote-tracking
/// branch; otherwise no submodule is touched, so a commit made on a
/// detached HEAD inside one is never left behind.
fn update_submodules(
    vcs: &dyn Vcs,
    path: &Path,
    pristine: &[String],
    stop: &AtomicBool,
) -> Submodules {
    let failed = |message: String| Submodules::Failed { message };
    if stop.load(Ordering::Relaxed) {
        return failed("interrupted before submodules were updated".to_owned());
    }
    let subs = match vcs.submodules(path) {
        Ok(subs) => subs,
        Err(err) => return failed(error_text(&err)),
    };
    for sub in subs
        .iter()
        .filter(|s| s.status == SubmoduleStatus::Moved && !pristine.contains(&s.path))
    {
        match vcs.head_is_referenced(&path.join(&sub.path)) {
            Ok(true) => {}
            Ok(false) => {
                return failed(format!(
                    "submodule {} is at {}, which only a detached HEAD holds; \
                     submodules were left alone so it is not lost",
                    sub.path,
                    short_rev(&sub.rev)
                ));
            }
            Err(err) => return failed(error_text(&err)),
        }
    }
    match vcs.update_submodules(path) {
        Ok(()) => Submodules::Ok,
        Err(err) => failed(error_text(&err)),
    }
}

fn skipped(reason: SkipReason, behind: u32) -> Outcome {
    Outcome::Skipped { reason, behind }
}

/// A failure carrying git's full error output when there is one.
fn failed(reason: FailReason, err: anyhow::Error) -> Outcome {
    Outcome::Failed {
        reason,
        message: error_text(&err),
    }
}

/// Git's full error output, or the error's own text.
fn error_text(err: &anyhow::Error) -> String {
    match err.downcast_ref::<GitError>() {
        Some(git) => git.stderr.trim_end().to_owned(),
        None => format!("{err:#}"),
    }
}

fn head_rev(head: &Head) -> Option<String> {
    match head {
        Head::Branch { rev, .. } | Head::Detached { rev } => Some(rev.clone()),
        Head::Unborn => None,
    }
}

fn describe_head(head: &Head) -> String {
    match head {
        Head::Branch { name, rev } => format!("branch {name} at {}", short_rev(rev)),
        Head::Detached { rev } => format!("detached at {}", short_rev(rev)),
        Head::Unborn => "on a branch with no commits".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::path::Path;

    use super::*;
    use crate::model::VcsKind;
    use crate::testutil::{Fixture, git, local_commit};
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
                allow_dirty: false,
                submodules: false,
                strategy: Strategy::FfOnly,
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
                rev: fixture.upstream_rev(),
                submodules: None
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
                commits: 2,
                rebased: 0,
                submodules: None
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
                ahead: 0,
                submodules: None
            }
        );
    }

    #[test]
    fn local_commits_alone_are_up_to_date() {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        local_commit(&checkout, "local.txt", "x");
        let (outcome, _) = sync(&job(&fixture, checkout.clone()));
        assert_eq!(
            outcome,
            Outcome::UpToDate {
                rev: rev(&checkout),
                ahead: 1,
                submodules: None
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
                reason: SkipReason::Dirty { overlap: vec![] },
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), before);
        assert_eq!(
            fs::read_to_string(checkout.join("file.txt")).unwrap(),
            "local edit"
        );
    }

    /// The contents of `name` in a checkout.
    fn read(checkout: &Path, name: &str) -> String {
        fs::read_to_string(checkout.join(name)).unwrap()
    }

    #[test]
    fn allow_dirty_fast_forwards_around_untouched_changes() {
        let mut fixture = Fixture::new();
        fixture.push_file("extra.txt", "extra\n");
        let checkout = fixture.checkout();
        let old = rev(&checkout);
        fs::write(checkout.join("extra.txt"), "local edit\n").unwrap();
        let new = fixture.push_commit();
        let mut job = job(&fixture, checkout.clone());

        // Off by default: skipped, as before.
        assert!(matches!(
            sync(&job).0,
            Outcome::Skipped {
                reason: SkipReason::Dirty { .. },
                ..
            }
        ));
        job.repo.allow_dirty = true;
        let (outcome, _) = sync(&job);
        assert_eq!(
            outcome,
            Outcome::Updated {
                old,
                new: new.clone(),
                commits: 1,
                rebased: 0,
                submodules: None
            }
        );
        assert_eq!(rev(&checkout), new);
        assert_eq!(read(&checkout, "extra.txt"), "local edit\n");
    }

    #[test]
    fn allow_dirty_skips_changes_upstream_also_makes() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let before = rev(&checkout);
        fs::write(checkout.join("file.txt"), "local edit\n").unwrap();
        fixture.push_commit();
        let mut job = job(&fixture, checkout.clone());
        job.repo.allow_dirty = true;
        let (outcome, _) = sync(&job);
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::Dirty {
                    overlap: vec!["file.txt".into()]
                },
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), before);
        assert_eq!(read(&checkout, "file.txt"), "local edit\n");
    }

    #[test]
    fn rebase_strategy_replays_local_commits() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let old = local_commit(&checkout, "local.txt", "mine\n");
        let upstream = fixture.push_commit();
        let mut job = job(&fixture, checkout.clone());
        job.repo.strategy = Strategy::Rebase;
        let (outcome, _) = sync(&job);
        let Outcome::Updated {
            old: from,
            new,
            commits,
            rebased,
            submodules,
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!((from, commits, rebased, submodules), (old, 1, 1, None));
        assert_eq!(rev(&checkout), new);
        assert_eq!(git(&checkout, &["rev-parse", "HEAD~1"]), upstream);
        assert_eq!(read(&checkout, "local.txt"), "mine\n");
    }

    #[test]
    fn a_rebase_that_stops_is_undone_and_skipped() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        let mine = local_commit(&checkout, "file.txt", "mine\n");
        fixture.push_commit();
        let mut job = job(&fixture, checkout.clone());
        job.repo.strategy = Strategy::Rebase;
        let (outcome, _) = sync(&job);
        assert_eq!(
            outcome,
            Outcome::Skipped {
                reason: SkipReason::RebaseStopped {
                    ahead: 1,
                    why: "conflict in file.txt".into()
                },
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), mine);
        assert_eq!(git(&checkout, &["status", "--porcelain"]), "");
        assert_eq!(read(&checkout, "file.txt"), "mine\n");
    }

    #[test]
    fn rebase_strategy_needs_a_clean_tree() {
        let mut fixture = Fixture::new();
        fixture.push_file("extra.txt", "extra\n");
        let checkout = fixture.checkout();
        let mine = local_commit(&checkout, "local.txt", "mine\n");
        fs::write(checkout.join("extra.txt"), "local edit\n").unwrap();
        fixture.push_commit();
        let mut job = job(&fixture, checkout.clone());
        job.repo.strategy = Strategy::Rebase;
        job.repo.allow_dirty = true;
        assert_eq!(
            sync(&job).0,
            Outcome::Skipped {
                reason: SkipReason::Dirty { overlap: vec![] },
                behind: 1
            }
        );
        assert_eq!(rev(&checkout), mine);
        assert_eq!(read(&checkout, "extra.txt"), "local edit\n");
    }

    /// A superproject upstream with the library fixture `lib` as a
    /// submodule at `lib`, and a checkout with the submodule initialised.
    fn with_submodule(fixture: &mut Fixture, lib: &Fixture) -> PathBuf {
        fixture.push_submodule("lib", lib);
        let checkout = fixture.checkout();
        git(
            &checkout,
            &[
                "-c",
                crate::testutil::ALLOW_FILE_SUBMODULES,
                "submodule",
                "update",
                "-q",
                "--init",
            ],
        );
        checkout
    }

    #[test]
    fn submodules_are_left_alone_when_a_commit_could_be_lost() {
        let mut lib = Fixture::new();
        let mut fixture = Fixture::new();
        let checkout = with_submodule(&mut fixture, &lib);
        // A commit made on the submodule's detached HEAD, held by nothing
        // else.
        let sub = checkout.join("lib");
        crate::testutil::identify(&sub);
        let orphan = local_commit(&sub, "mine.txt", "mine\n");
        lib.push_commit();
        let new = fixture.bump_submodule("lib");

        let mut job = job(&fixture, checkout.clone());
        job.repo.submodules = true;
        let (outcome, _) = sync(&job);
        match outcome {
            Outcome::Updated {
                new: got,
                submodules: Some(Submodules::Failed { message }),
                ..
            } => {
                assert_eq!(got, new);
                assert!(message.contains("only a detached HEAD holds"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rev(&sub), orphan);
        assert_eq!(read(&sub, "mine.txt"), "mine\n");
    }

    #[test]
    fn a_failed_submodule_step_keeps_the_update_and_heals_later() {
        let mut lib = Fixture::new();
        let mut fixture = Fixture::new();
        let checkout = with_submodule(&mut fixture, &lib);
        lib.push_commit();
        let new = fixture.bump_submodule("lib");
        // The submodule's upstream is gone, so updating it fails.
        let lib_upstream = lib.path("upstream.git");
        let hidden = lib.path("hidden.git");
        fs::rename(&lib_upstream, &hidden).unwrap();

        let mut job = job(&fixture, checkout.clone());
        job.repo.submodules = true;
        let (outcome, _) = sync(&job);
        assert!(
            matches!(
                &outcome,
                Outcome::Updated { new: got, submodules: Some(Submodules::Failed { .. }), .. }
                    if *got == new
            ),
            "{outcome:?}"
        );
        assert_eq!(rev(&checkout), new);

        // The moved submodule does not count as a local change, and the
        // next sync tries the submodules again. git refuses local file
        // transports for submodules here, so the retry fails the same way.
        fs::rename(&hidden, &lib_upstream).unwrap();
        let (outcome, _) = sync(&job);
        assert!(
            matches!(
                &outcome,
                Outcome::UpToDate {
                    submodules: Some(_),
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn diverged_checkout_is_skipped() {
        let mut fixture = Fixture::new();
        let checkout = fixture.checkout();
        local_commit(&checkout, "local.txt", "x");
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

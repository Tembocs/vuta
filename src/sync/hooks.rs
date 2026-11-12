//! The post-sync hook runner (docs/ARCHITECTURE.md section 8).
//!
//! Hooks run one at a time through the platform shell, in the checkout,
//! with their output streamed straight to the terminal. The first failure
//! stops the repository's remaining hooks.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::model::{Hook, HookEnd, HookOutcome, Outcome, RunWhen};
use crate::proc;

/// Whether `hook` should run after `outcome`. Hooks never run after a skip
/// or failure, since the checkout is not in the state they expect. `force`
/// is `vuta sync --force-hooks`.
pub fn should_run(hook: &Hook, outcome: &Outcome, force: bool) -> bool {
    if !hook.enabled || !hook.platform.matches_current() {
        return false;
    }
    if matches!(outcome, Outcome::Skipped { .. } | Outcome::Failed { .. }) {
        return false;
    }
    force || hook.run_when == RunWhen::Always || outcome.changed()
}

/// What the hooks of one repository run with.
#[derive(Debug, Clone, Copy)]
pub struct HookContext<'a> {
    pub repo_name: &'a str,
    pub path: &'a Path,
    pub outcome: &'a Outcome,
    /// Send the hook's stdout to stderr, so stdout carries only Vuta's own
    /// output (`--format json`).
    pub stdout_to_stderr: bool,
}

/// Progress reported to the caller, which prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent<'a> {
    Started {
        hook: &'a Hook,
    },
    Finished {
        hook: &'a Hook,
        end: HookEnd,
        duration: Duration,
    },
}

/// Run the eligible hooks among `hooks` in order. Returns `NotRun` when
/// none were eligible or `stop` was set before the first one.
pub fn run(
    ctx: &HookContext,
    hooks: &[Hook],
    force: bool,
    stop: &AtomicBool,
    mut on_event: impl FnMut(HookEvent),
) -> HookOutcome {
    let (old_rev, new_rev) = revisions(ctx.outcome);
    let mut ran_any = false;
    for hook in hooks.iter().filter(|h| should_run(h, ctx.outcome, force)) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        ran_any = true;
        on_event(HookEvent::Started { hook });
        let started = Instant::now();
        let mut cmd = proc::shell_command(&hook.command);
        if ctx.stdout_to_stderr {
            cmd.stdout(Stdio::from(std::io::stderr()));
        }
        let status = cmd
            .current_dir(ctx.path)
            .env("VUTA_REPO_NAME", ctx.repo_name)
            .env("VUTA_REPO_PATH", ctx.path)
            .env("VUTA_OLD_REV", old_rev)
            .env("VUTA_NEW_REV", new_rev)
            .stdin(Stdio::inherit())
            .status();
        let end = match status {
            Ok(status) if status.success() => HookEnd::Ok,
            Ok(status) => HookEnd::Failed {
                code: status.code(),
            },
            Err(_) => HookEnd::CouldNotStart,
        };
        on_event(HookEvent::Finished {
            hook,
            end,
            duration: started.elapsed(),
        });
        if end != HookEnd::Ok {
            return HookOutcome::Failed;
        }
    }
    if ran_any {
        HookOutcome::Ok
    } else {
        HookOutcome::NotRun
    }
}

/// `VUTA_OLD_REV` (empty after a clone) and `VUTA_NEW_REV`.
fn revisions(outcome: &Outcome) -> (&str, &str) {
    match outcome {
        Outcome::Cloned { rev } => ("", rev),
        Outcome::Updated { old, new, .. } => (old, new),
        Outcome::UpToDate { rev, .. } => (rev, rev),
        Outcome::Skipped { .. } | Outcome::Failed { .. } => ("", ""),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::model::{FailReason, Platform, SkipReason};

    fn hook(id: i64, command: &str, platform: Platform, run_when: RunWhen) -> Hook {
        Hook {
            id,
            repo_id: 1,
            position: id,
            platform,
            run_when,
            command: command.into(),
            enabled: true,
        }
    }

    fn updated() -> Outcome {
        Outcome::Updated {
            old: "aaa".into(),
            new: "bbb".into(),
            commits: 1,
        }
    }

    fn up_to_date() -> Outcome {
        Outcome::UpToDate {
            rev: "bbb".into(),
            ahead: 0,
        }
    }

    /// Write `$VUTA_NEW_REV` (or the Windows equivalent) to `name`.
    fn write_new_rev(name: &str) -> String {
        if cfg!(windows) {
            format!("echo %VUTA_OLD_REV%-%VUTA_NEW_REV%-%VUTA_REPO_NAME%> {name}")
        } else {
            format!("echo \"$VUTA_OLD_REV-$VUTA_NEW_REV-$VUTA_REPO_NAME\" > {name}")
        }
    }

    #[test]
    fn eligibility() {
        let changed = hook(1, "x", Platform::Any, RunWhen::Changed);
        let always = hook(2, "x", Platform::Any, RunWhen::Always);
        let other_os = hook(
            3,
            "x",
            if cfg!(windows) {
                Platform::Unix
            } else {
                Platform::Windows
            },
            RunWhen::Always,
        );
        let mut disabled = always.clone();
        disabled.enabled = false;
        let skipped = Outcome::Skipped {
            reason: SkipReason::Dirty,
            behind: 1,
        };
        let failed = Outcome::Failed {
            reason: FailReason::Fetch,
            message: String::new(),
        };

        assert!(should_run(&changed, &updated(), false));
        assert!(should_run(
            &changed,
            &Outcome::Cloned { rev: "a".into() },
            false
        ));
        assert!(!should_run(&changed, &up_to_date(), false));
        assert!(should_run(&changed, &up_to_date(), true));
        assert!(should_run(&always, &up_to_date(), false));
        assert!(!should_run(&always, &skipped, true));
        assert!(!should_run(&always, &failed, true));
        assert!(!should_run(&other_os, &updated(), true));
        assert!(!should_run(&disabled, &updated(), true));
    }

    #[test]
    fn runs_in_the_checkout_with_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [hook(
            1,
            &write_new_rev("out.txt"),
            Platform::Any,
            RunWhen::Changed,
        )];
        let ctx = HookContext {
            repo_name: "v",
            path: dir.path(),
            outcome: &updated(),
            stdout_to_stderr: false,
        };
        let mut events = Vec::new();
        let outcome = run(&ctx, &hooks, false, &AtomicBool::new(false), |e| {
            events.push(match e {
                HookEvent::Started { hook } => (hook.id, None),
                HookEvent::Finished { hook, end, .. } => (hook.id, Some(end)),
            })
        });
        assert_eq!(outcome, HookOutcome::Ok);
        assert_eq!(events, [(1, None), (1, Some(HookEnd::Ok))]);
        let written = fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(written.trim(), "aaa-bbb-v");
    }

    #[test]
    fn first_failure_stops_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [
            hook(1, "exit 3", Platform::Any, RunWhen::Changed),
            hook(
                2,
                &write_new_rev("never.txt"),
                Platform::Any,
                RunWhen::Changed,
            ),
        ];
        let ctx = HookContext {
            repo_name: "v",
            path: dir.path(),
            outcome: &updated(),
            stdout_to_stderr: false,
        };
        let mut ends = Vec::new();
        let outcome = run(&ctx, &hooks, false, &AtomicBool::new(false), |e| {
            if let HookEvent::Finished { end, .. } = e {
                ends.push(end);
            }
        });
        assert_eq!(outcome, HookOutcome::Failed);
        assert_eq!(ends, [HookEnd::Failed { code: Some(3) }]);
        assert!(!dir.path().join("never.txt").exists());
    }

    #[test]
    fn nothing_eligible_or_stopped_is_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [hook(
            1,
            &write_new_rev("out.txt"),
            Platform::Any,
            RunWhen::Changed,
        )];
        let ctx = HookContext {
            repo_name: "v",
            path: dir.path(),
            outcome: &up_to_date(),
            stdout_to_stderr: false,
        };
        assert_eq!(
            run(&ctx, &hooks, false, &AtomicBool::new(false), |_| {}),
            HookOutcome::NotRun
        );
        let ctx = HookContext {
            outcome: &updated(),
            ..ctx
        };
        assert_eq!(
            run(&ctx, &hooks, false, &AtomicBool::new(true), |_| {}),
            HookOutcome::NotRun
        );
        assert!(!dir.path().join("out.txt").exists());
    }
}

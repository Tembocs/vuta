//! Sync result lines, live progress lines, hook lines and the summary line.
//!
//! ```text
//! [2/9] ✓ updated     zed         3f9a2c1 → 8be04d7   42 commits       6.1s
//! [3/9] ! skipped     edit        local changes; 3 commits behind      0.9s
//!       ▸ cloning     surrealdb   12s
//! ```

use std::time::Duration;

use anstyle::Style;

use super::theme::{Glyphs, THEME};
use super::{Styled, fmt_duration, pad};
use crate::model::{
    FailReason, HookEnd, Outcome, OutcomeKind, SkipReason, Tally, count, short_rev,
};
use crate::proc::last_line;

/// Width of the status word column; "up to date" is the longest.
const STATUS_WIDTH: usize = 10;
/// Width the detail column is padded to before the duration.
const DETAIL_WIDTH: usize = 36;
/// Width the duration is right-aligned in.
const DURATION_WIDTH: usize = 6;

/// One repository's result, as printed when it finishes.
#[derive(Debug, Clone, Copy)]
pub struct ResultLine<'a> {
    /// 1-based position in finishing order.
    pub index: usize,
    pub total: usize,
    pub name: &'a str,
    /// The longest name in the run, so names align.
    pub name_width: usize,
    pub outcome: &'a Outcome,
    pub duration: Duration,
}

/// The colour, glyph and word for an outcome.
pub fn status(kind: OutcomeKind, glyphs: &Glyphs) -> (Style, &'static str, &'static str) {
    match kind {
        OutcomeKind::Cloned => (THEME.new, glyphs.cloned, "cloned"),
        OutcomeKind::Updated => (THEME.ok, glyphs.updated, "updated"),
        OutcomeKind::UpToDate => (THEME.muted, glyphs.up_to_date, "up to date"),
        OutcomeKind::Skipped => (THEME.warn, glyphs.skipped, "skipped"),
        OutcomeKind::Failed => (THEME.err, glyphs.failed, "failed"),
    }
}

pub fn result_line(line: &ResultLine, glyphs: &Glyphs) -> String {
    let (style, glyph, word) = status(line.outcome.kind(), glyphs);
    let digits = line.total.to_string().len();
    let mut out = Styled::new();
    out.plain(&format!("[{:>digits$}/{}] ", line.index, line.total))
        .push(&pad(glyph, glyphs.status_width()), style)
        .plain(" ")
        .push(&pad(word, STATUS_WIDTH), style)
        .plain("  ")
        .push(&pad(line.name, line.name_width), THEME.name)
        .plain("   ");
    let detail = detail(line.outcome, glyphs);
    let gap = DETAIL_WIDTH.saturating_sub(detail.width()) + 2;
    out.append(&detail).plain(&" ".repeat(gap)).push(
        &format!("{:>DURATION_WIDTH$}", fmt_duration(line.duration)),
        THEME.muted,
    );
    out.into_string()
}

/// The words after the name on a result line.
pub fn detail(outcome: &Outcome, glyphs: &Glyphs) -> Styled {
    let mut out = Styled::new();
    match outcome {
        Outcome::Cloned { rev } => {
            out.push(short_rev(rev), THEME.rev);
        }
        Outcome::Updated { old, new, commits } => {
            out.push(short_rev(old), THEME.rev)
                .plain(&format!(" {} ", glyphs.arrow))
                .push(short_rev(new), THEME.rev)
                .plain("   ");
            counted(&mut out, *commits, "commit");
        }
        Outcome::UpToDate { rev, ahead } => {
            out.push(short_rev(rev), THEME.rev);
            if *ahead > 0 {
                out.plain("   ");
                counted(&mut out, *ahead, "local commit");
            }
        }
        Outcome::Skipped { reason, behind } => skip_detail(&mut out, reason, *behind),
        Outcome::Failed { reason, message } => {
            let line = last_line(message).map(strip_git_prefix).unwrap_or("");
            match reason {
                FailReason::NotACheckout => out.plain("path exists but is not a Git checkout"),
                FailReason::Interrupted => out.plain("interrupted"),
                other => {
                    let label = other.as_str().replace('_', "-");
                    if line.is_empty() {
                        out.plain(&label)
                    } else {
                        out.plain(&format!("{label}: {line}"))
                    }
                }
            };
        }
    }
    out
}

/// The styled form of `SkipReason::describe`.
fn skip_detail(out: &mut Styled, reason: &SkipReason, behind: u32) {
    match reason {
        SkipReason::Dirty => {
            out.plain("local changes");
            if behind > 0 {
                out.plain("; ");
                counted(out, behind, "commit");
                out.plain(" behind");
            }
        }
        SkipReason::Diverged { ahead } => {
            counted(out, *ahead, "local commit");
            out.plain(", ")
                .push(&behind.to_string(), THEME.rev)
                .plain(" behind");
        }
        SkipReason::Detached { rev } => {
            out.plain("HEAD is detached at ")
                .push(short_rev(rev), THEME.rev);
        }
        SkipReason::NoUpstream { branch } => {
            out.plain("branch ")
                .push(branch, THEME.name)
                .plain(" tracks no remote branch");
        }
        SkipReason::RemoteMismatch { found, expected } => {
            out.plain(&format!("origin is {found}, expected {expected}"));
        }
        SkipReason::Disabled => {
            out.plain("disabled");
        }
        SkipReason::FetchOnly => {
            counted(out, behind, "commit");
            out.plain(" behind");
        }
    }
}

/// `n noun(s)` with the number in the revision colour.
fn counted(out: &mut Styled, n: u32, noun: &str) {
    let text = count(n, noun);
    let (number, rest) = text.split_once(' ').unwrap_or((&text, ""));
    out.push(number, THEME.rev).plain(" ").plain(rest);
}

/// Git's messages start with `fatal: ` or `error: `; the result line
/// already says it failed.
fn strip_git_prefix(line: &str) -> &str {
    ["fatal: ", "error: "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .unwrap_or(line)
}

/// What a worker is doing, as a live progress line shows it.
#[derive(Debug, Clone, Copy)]
pub struct LiveLine<'a> {
    /// The run's repository count, so the line lines up with result lines.
    pub total: usize,
    pub name: &'a str,
    pub name_width: usize,
    pub cloning: bool,
    pub elapsed: Duration,
    /// The line is cut to this many characters, so it never wraps.
    pub max_width: usize,
}

/// `      ▸ cloning     zed         12s`, in the columns of a result line:
/// the glyph and word where the status goes, then the name and how long
/// the repository has taken so far.
pub fn live_line(line: &LiveLine, glyphs: &Glyphs) -> String {
    let digits = line.total.to_string().len();
    let word = if line.cloning { "cloning" } else { "syncing" };
    let elapsed = fmt_elapsed(line.elapsed);
    // `[n/total] ` is replaced by spaces.
    let indent = 2 * digits + 4;
    let before_name = indent + glyphs.status_width() + 1 + STATUS_WIDTH + 2;
    let after_name = 3 + super::text_width(&elapsed);
    let room = line.max_width.saturating_sub(before_name + after_name);
    let name: String = line.name.chars().take(room).collect();
    let mut out = Styled::new();
    out.plain(&" ".repeat(indent))
        .push(
            &pad(glyphs.hook_running, glyphs.status_width()),
            THEME.muted,
        )
        .plain(" ")
        .push(&pad(word, STATUS_WIDTH), THEME.muted)
        .plain("  ")
        .push(&pad(&name, line.name_width.min(room)), THEME.name)
        .plain("   ")
        .push(&elapsed, THEME.muted);
    out.into_string()
}

/// `      ▸ and 3 more`, the last line when more workers are busy than the
/// block shows.
pub fn live_more_line(total: usize, more: usize, glyphs: &Glyphs) -> String {
    let digits = total.to_string().len();
    let mut out = Styled::new();
    out.plain(&" ".repeat(2 * digits + 4))
        .push(
            &pad(glyphs.hook_running, glyphs.status_width()),
            THEME.muted,
        )
        .plain(" ")
        .push(&format!("and {more} more"), THEME.muted);
    out.into_string()
}

/// `0s`, `12s`, then `2m05s`: whole seconds, since the line is redrawn
/// about twice a second.
fn fmt_elapsed(elapsed: Duration) -> String {
    if elapsed.as_secs() < 60 {
        format!("{}s", elapsed.as_secs())
    } else {
        fmt_duration(elapsed)
    }
}

/// The dim line printed when a clone starts, since clones can take minutes.
pub fn clone_started_line(name: &str) -> String {
    let mut out = Styled::new();
    out.push(&format!("cloning {name}"), THEME.muted);
    out.into_string()
}

/// `1 cloned · 1 updated · 5 up to date · 1 skipped · 1 failed   in 41.0s`
pub fn summary_line(tally: &Tally, elapsed: Duration, glyphs: &Glyphs) -> String {
    let mut parts = vec![
        (tally.cloned, "cloned".to_owned(), THEME.new),
        (tally.updated, "updated".to_owned(), THEME.ok),
        (tally.up_to_date, "up to date".to_owned(), THEME.muted),
        (tally.skipped, "skipped".to_owned(), THEME.warn),
        (tally.failed, "failed".to_owned(), THEME.err),
    ];
    if tally.hooks_failed > 0 {
        let noun = if tally.hooks_failed == 1 {
            "hook failed"
        } else {
            "hooks failed"
        };
        parts.push((tally.hooks_failed, noun.to_owned(), THEME.err));
    }
    let mut out = Styled::new();
    for (i, (n, label, style)) in parts.iter().enumerate() {
        if i > 0 {
            out.plain(&format!(" {} ", glyphs.separator));
        }
        let style = if *n == 0 { THEME.muted } else { *style };
        out.push(&format!("{n} {label}"), style);
    }
    out.plain("   ")
        .push(&format!("in {}", fmt_duration(elapsed)), THEME.muted);
    out.into_string()
}

pub fn hooks_heading() -> String {
    let mut out = Styled::new();
    out.push("Hooks", THEME.name);
    out.into_string()
}

/// `  ▸ v  make`
pub fn hook_started_line(repo: &str, command: &str, glyphs: &Glyphs) -> String {
    let mut out = Styled::new();
    out.plain("  ")
        .push(glyphs.hook_running, THEME.muted)
        .plain(" ")
        .push(repo, THEME.name)
        .plain("  ")
        .plain(command);
    out.into_string()
}

/// `  ✓ v  hook finished   48.2s`
pub fn hook_finished_line(repo: &str, end: HookEnd, duration: Duration, glyphs: &Glyphs) -> String {
    let (style, glyph, words) = match end {
        HookEnd::Ok => (THEME.ok, glyphs.updated, "hook finished".to_owned()),
        HookEnd::Failed { code: Some(code) } => (
            THEME.err,
            glyphs.failed,
            format!("hook failed (exit {code})"),
        ),
        HookEnd::Failed { code: None } => (THEME.err, glyphs.failed, "hook was killed".to_owned()),
        HookEnd::CouldNotStart => (THEME.err, glyphs.failed, "hook could not start".to_owned()),
    };
    let mut out = Styled::new();
    out.plain("  ")
        .push(glyph, style)
        .plain(" ")
        .push(repo, THEME.name)
        .plain("  ")
        .push(&words, style)
        .plain("   ")
        .push(&fmt_duration(duration), THEME.muted);
    out.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(s: &str) -> String {
        anstream::adapter::strip_str(s).to_string()
    }

    fn line(index: usize, name: &str, outcome: &Outcome, ms: u64) -> String {
        let line = ResultLine {
            index,
            total: 9,
            name,
            name_width: 9,
            outcome,
            duration: Duration::from_millis(ms),
        };
        strip(&result_line(&line, &Glyphs::UNICODE))
    }

    const A: &str = "a91c0de0000";
    const B: &str = "3f9a2c10000";
    const C: &str = "8be04d70000";

    #[test]
    fn renders_each_outcome() {
        let cases = [
            (
                Outcome::UpToDate {
                    rev: A.into(),
                    ahead: 0,
                },
                1200,
                "[1/9] = up to date  surrealdb   a91c0de                                 1.2s",
            ),
            (
                Outcome::Updated {
                    old: B.into(),
                    new: C.into(),
                    commits: 42,
                },
                6100,
                "[1/9] ✓ updated     surrealdb   3f9a2c1 → 8be04d7   42 commits          6.1s",
            ),
            (
                Outcome::Skipped {
                    reason: SkipReason::Dirty,
                    behind: 3,
                },
                900,
                "[1/9] ! skipped     surrealdb   local changes; 3 commits behind         0.9s",
            ),
            (
                Outcome::Failed {
                    reason: FailReason::Fetch,
                    message: "fatal: could not resolve host\n".into(),
                },
                400,
                "[1/9] ✗ failed      surrealdb   fetch: could not resolve host           0.4s",
            ),
            (
                Outcome::Cloned { rev: A.into() },
                38_400,
                "[1/9] + cloned      surrealdb   a91c0de                                38.4s",
            ),
            (
                Outcome::UpToDate {
                    rev: A.into(),
                    ahead: 2,
                },
                100,
                "[1/9] = up to date  surrealdb   a91c0de   2 local commits               0.1s",
            ),
        ];
        for (outcome, ms, expected) in cases {
            assert_eq!(line(1, "surrealdb", &outcome, ms), expected);
        }
    }

    #[test]
    fn live_lines_align_with_result_lines() {
        let live = |name, cloning, secs, max_width| {
            let line = LiveLine {
                total: 9,
                name,
                name_width: 9,
                cloning,
                elapsed: Duration::from_secs(secs),
                max_width,
            };
            strip(&live_line(&line, &Glyphs::UNICODE))
        };
        let result = line(1, "surrealdb", &Outcome::Cloned { rev: A.into() }, 100);
        let cloning = live("surrealdb", true, 12, 200);
        assert_eq!(cloning, "      ▸ cloning     surrealdb   12s");
        // The name starts in the same column as on a result line.
        let column = |line: &str| line[..line.find("surrealdb").unwrap()].chars().count();
        assert_eq!(column(&result), column(&cloning));
        assert_eq!(
            live("zed", false, 125, 200),
            "      ▸ syncing     zed         2m05s"
        );
        // A narrow terminal cuts the name, never the time.
        let narrow = live("surrealdb", true, 3, 32);
        assert_eq!(narrow, "      ▸ cloning     surreal   3s");
        assert_eq!(narrow.chars().count(), 32);
        assert_eq!(
            strip(&live_more_line(12, 3, &Glyphs::ASCII)),
            "        >  and 3 more"
        );
    }

    #[test]
    fn short_names_are_padded_and_ascii_glyphs_align() {
        let outcome = Outcome::Updated {
            old: B.into(),
            new: C.into(),
            commits: 1,
        };
        let line = ResultLine {
            index: 2,
            total: 12,
            name: "zed",
            name_width: 5,
            outcome: &outcome,
            duration: Duration::from_secs(75),
        };
        assert_eq!(
            strip(&result_line(&line, &Glyphs::ASCII)),
            "[ 2/12] ok updated     zed     3f9a2c1 -> 8be04d7   1 commit          1m15s"
        );
    }

    #[test]
    fn styled_skip_details_match_stored_text() {
        let reasons = [
            SkipReason::Dirty,
            SkipReason::Diverged { ahead: 2 },
            SkipReason::Detached { rev: A.into() },
            SkipReason::NoUpstream {
                branch: "local".into(),
            },
            SkipReason::RemoteMismatch {
                found: "x".into(),
                expected: "y".into(),
            },
            SkipReason::Disabled,
            SkipReason::FetchOnly,
        ];
        for reason in reasons {
            for behind in [0, 1, 5] {
                let outcome = Outcome::Skipped {
                    reason: reason.clone(),
                    behind,
                };
                let styled = detail(&outcome, &Glyphs::UNICODE).into_string();
                assert_eq!(strip(&styled), reason.describe(behind));
            }
        }
    }

    #[test]
    fn failure_details() {
        let fail = |reason, message: &str| {
            let outcome = Outcome::Failed {
                reason,
                message: message.into(),
            };
            strip(&detail(&outcome, &Glyphs::UNICODE).into_string())
        };
        assert_eq!(
            fail(FailReason::NotACheckout, ""),
            "path exists but is not a Git checkout"
        );
        assert_eq!(
            fail(
                FailReason::FastForward,
                "error: x\nfatal: Not possible to fast-forward, aborting.\n"
            ),
            "fast-forward: Not possible to fast-forward, aborting."
        );
        assert_eq!(fail(FailReason::Clone, ""), "clone");
        assert_eq!(fail(FailReason::Interrupted, "whatever"), "interrupted");
    }

    #[test]
    fn summary_and_hooks() {
        let tally = Tally {
            cloned: 1,
            updated: 1,
            up_to_date: 5,
            skipped: 1,
            failed: 1,
            hooks_failed: 0,
        };
        assert_eq!(
            strip(&summary_line(
                &tally,
                Duration::from_millis(41_000),
                &Glyphs::UNICODE
            )),
            "1 cloned · 1 updated · 5 up to date · 1 skipped · 1 failed   in 41.0s"
        );
        let tally = Tally {
            updated: 1,
            hooks_failed: 2,
            ..Default::default()
        };
        assert_eq!(
            strip(&summary_line(
                &tally,
                Duration::from_secs(2),
                &Glyphs::ASCII
            )),
            "0 cloned - 1 updated - 0 up to date - 0 skipped - 0 failed - 2 hooks failed   in 2.0s"
        );
        assert_eq!(
            strip(&hook_started_line("v", "make", &Glyphs::UNICODE)),
            "  ▸ v  make"
        );
        assert_eq!(
            strip(&hook_finished_line(
                "v",
                HookEnd::Failed { code: Some(2) },
                Duration::from_secs(3),
                &Glyphs::UNICODE
            )),
            "  ✗ v  hook failed (exit 2)   3.0s"
        );
    }

    #[test]
    fn zero_counts_are_dim() {
        let tally = Tally {
            updated: 1,
            ..Default::default()
        };
        let line = summary_line(&tally, Duration::ZERO, &Glyphs::UNICODE);
        let dim = format!("{}0 cloned{:#}", THEME.muted, THEME.muted);
        let green = format!("{}1 updated{:#}", THEME.ok, THEME.ok);
        assert!(line.contains(&dim) && line.contains(&green), "{line:?}");
    }
}

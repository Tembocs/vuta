//! Core domain types. Database rows, result lines and exit codes are all
//! derived from these.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// A string did not name any variant of a fixed set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} '{value}'; expected one of: {expected}")]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub value: String,
    pub expected: String,
}

/// A fieldless enum stored and parsed as fixed lowercase strings. Parsing
/// ignores ASCII case.
macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident ($kind:literal) {
            $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.as_str().eq_ignore_ascii_case(s))
                    .ok_or_else(|| ParseEnumError {
                        kind: $kind,
                        value: s.to_owned(),
                        expected: Self::ALL
                            .iter()
                            .map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    })
            }
        }
    };
}

str_enum! {
    pub enum VcsKind ("VCS") {
        Git => "git",
    }
}

str_enum! {
    /// The stored form of an `Outcome`.
    pub enum OutcomeKind ("outcome") {
        Cloned => "cloned",
        Updated => "updated",
        UpToDate => "up_to_date",
        Skipped => "skipped",
        Failed => "failed",
    }
}

str_enum! {
    /// The stored form of a `SkipReason`.
    pub enum SkipKind ("skip reason") {
        Dirty => "dirty",
        Diverged => "diverged",
        Detached => "detached",
        NoUpstream => "no_upstream",
        RemoteMismatch => "remote_mismatch",
        Disabled => "disabled",
    }
}

str_enum! {
    pub enum FailReason ("failure reason") {
        NotACheckout => "not_a_checkout",
        Clone => "clone",
        Fetch => "fetch",
        FastForward => "fast_forward",
        Interrupted => "interrupted",
    }
}

str_enum! {
    pub enum HookOutcome ("hook outcome") {
        Ok => "ok",
        Failed => "failed",
        NotRun => "not_run",
    }
}

str_enum! {
    /// Which operating systems a hook runs on.
    pub enum Platform ("platform") {
        Any => "any",
        Unix => "unix",
        Windows => "windows",
    }
}

impl Platform {
    /// Whether a hook with this platform runs on the current OS.
    pub fn matches_current(self) -> bool {
        match self {
            Platform::Any => true,
            Platform::Unix => cfg!(unix),
            Platform::Windows => cfg!(windows),
        }
    }
}

str_enum! {
    /// When a hook runs.
    pub enum RunWhen ("run-when") {
        /// After a clone, or an update that moved the revision.
        Changed => "changed",
        /// After every sync of the repository.
        Always => "always",
    }
}

str_enum! {
    /// The `ui.glyphs` setting.
    pub enum GlyphMode ("glyph mode") {
        Auto => "auto",
        Unicode => "unicode",
        Ascii => "ascii",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub id: i64,
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: i64,
    pub name: String,
    pub root_id: i64,
    /// Always uses `/` separators.
    pub rel_path: String,
    pub vcs: VcsKind,
    pub url: String,
    /// `None` means the remote's default branch.
    pub branch: Option<String>,
    pub clone_filter: Option<String>,
    pub clone_depth: Option<u32>,
    pub enabled: bool,
}

/// The cached result of a repository's latest sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastSync {
    pub rev: Option<String>,
    pub outcome: Option<OutcomeKind>,
    /// Unix seconds.
    pub at: Option<i64>,
}

/// A repository with what listings show beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEntry {
    pub repo: Repo,
    pub root_name: String,
    pub tags: Vec<String>,
    pub last: LastSync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub id: i64,
    pub repo_id: i64,
    pub position: i64,
    pub platform: Platform,
    pub run_when: RunWhen,
    pub command: String,
    pub enabled: bool,
}

/// One `vuta sync` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: i64,
    /// Unix seconds.
    pub started_at: i64,
    /// `None` if the process was killed before finishing.
    pub finished_at: Option<i64>,
    pub vuta_version: String,
    pub args: String,
    pub exit_code: Option<i32>,
}

/// A run with its per-outcome counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub run: Run,
    pub tally: Tally,
}

/// One stored repository result, as `vuta log` and `vuta show` read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub id: i64,
    pub run_id: i64,
    pub run_started_at: i64,
    pub repo_id: i64,
    pub repo_name: String,
    pub outcome: OutcomeKind,
    /// A `SkipKind` or `FailReason` string.
    pub reason: Option<String>,
    pub old_rev: Option<String>,
    pub new_rev: Option<String>,
    /// Commits brought in (updated) or not yet applied (skipped).
    pub commits: Option<u32>,
    pub duration_ms: u64,
    pub hook_outcome: Option<HookOutcome>,
    pub message: Option<String>,
}

/// Counts of outcomes in a run, for the summary line and `vuta log`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub cloned: u32,
    pub updated: u32,
    pub up_to_date: u32,
    pub skipped: u32,
    pub failed: u32,
    pub hooks_failed: u32,
}

impl Tally {
    pub fn add(&mut self, kind: OutcomeKind) {
        match kind {
            OutcomeKind::Cloned => self.cloned += 1,
            OutcomeKind::Updated => self.updated += 1,
            OutcomeKind::UpToDate => self.up_to_date += 1,
            OutcomeKind::Skipped => self.skipped += 1,
            OutcomeKind::Failed => self.failed += 1,
        }
    }

    pub fn total(&self) -> u32 {
        self.cloned + self.updated + self.up_to_date + self.skipped + self.failed
    }
}

/// The one value a worker returns per repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Cloned {
        rev: String,
    },
    Updated {
        old: String,
        new: String,
        commits: u32,
    },
    /// `ahead` counts local commits not on the upstream.
    UpToDate {
        rev: String,
        ahead: u32,
    },
    /// `behind` counts upstream commits not applied.
    Skipped {
        reason: SkipReason,
        behind: u32,
    },
    /// `message` is git's full error output; result lines show its last line.
    Failed {
        reason: FailReason,
        message: String,
    },
}

impl Outcome {
    pub fn kind(&self) -> OutcomeKind {
        match self {
            Outcome::Cloned { .. } => OutcomeKind::Cloned,
            Outcome::Updated { .. } => OutcomeKind::Updated,
            Outcome::UpToDate { .. } => OutcomeKind::UpToDate,
            Outcome::Skipped { .. } => OutcomeKind::Skipped,
            Outcome::Failed { .. } => OutcomeKind::Failed,
        }
    }

    /// The checkout's revision after the sync, when known.
    pub fn rev(&self) -> Option<&str> {
        match self {
            Outcome::Cloned { rev } | Outcome::UpToDate { rev, .. } => Some(rev),
            Outcome::Updated { new, .. } => Some(new),
            Outcome::Skipped { .. } | Outcome::Failed { .. } => None,
        }
    }

    /// Whether the revision moved, which is when `changed` hooks run.
    pub fn changed(&self) -> bool {
        matches!(self, Outcome::Cloned { .. } | Outcome::Updated { .. })
    }
}

/// Why a repository was left untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Tracked files have local modifications.
    Dirty,
    /// Local commits and upstream commits both exist.
    Diverged {
        ahead: u32,
    },
    Detached {
        rev: String,
    },
    NoUpstream {
        branch: String,
    },
    RemoteMismatch {
        found: String,
        expected: String,
    },
    Disabled,
}

impl SkipReason {
    pub fn kind(&self) -> SkipKind {
        match self {
            SkipReason::Dirty => SkipKind::Dirty,
            SkipReason::Diverged { .. } => SkipKind::Diverged,
            SkipReason::Detached { .. } => SkipKind::Detached,
            SkipReason::NoUpstream { .. } => SkipKind::NoUpstream,
            SkipReason::RemoteMismatch { .. } => SkipKind::RemoteMismatch,
            SkipReason::Disabled => SkipKind::Disabled,
        }
    }

    /// The plain-text explanation stored in `run_results.message`. The
    /// result line shows the same words with colour.
    pub fn describe(&self, behind: u32) -> String {
        match self {
            SkipReason::Dirty if behind == 0 => "local changes".to_owned(),
            SkipReason::Dirty => format!("local changes; {}", count(behind, "commit") + " behind"),
            SkipReason::Diverged { ahead } => {
                format!("{}, {behind} behind", count(*ahead, "local commit"))
            }
            SkipReason::Detached { rev } => format!("HEAD is detached at {}", short_rev(rev)),
            SkipReason::NoUpstream { branch } => {
                format!("branch {branch} tracks no remote branch")
            }
            SkipReason::RemoteMismatch { found, expected } => {
                format!("origin is {found}, expected {expected}")
            }
            SkipReason::Disabled => "disabled".to_owned(),
        }
    }
}

/// `n` followed by `noun`, pluralised with a trailing `s`.
pub fn count(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The abbreviated form of a revision shown to people.
pub fn short_rev(rev: &str) -> &str {
    rev.get(..7).unwrap_or(rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_enums_round_trip_and_ignore_case() {
        for kind in OutcomeKind::ALL {
            assert_eq!(kind.as_str().parse::<OutcomeKind>().unwrap(), *kind);
        }
        assert_eq!("UNIX".parse::<Platform>().unwrap(), Platform::Unix);
        let err = "mac".parse::<Platform>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown platform 'mac'; expected one of: any, unix, windows"
        );
    }

    #[test]
    fn skip_descriptions() {
        assert_eq!(
            SkipReason::Dirty.describe(3),
            "local changes; 3 commits behind"
        );
        assert_eq!(SkipReason::Dirty.describe(0), "local changes");
        assert_eq!(
            SkipReason::Diverged { ahead: 2 }.describe(5),
            "2 local commits, 5 behind"
        );
        assert_eq!(
            SkipReason::Detached {
                rev: "0123456789abcdef".into()
            }
            .describe(0),
            "HEAD is detached at 0123456"
        );
    }

    #[test]
    fn tally_counts_kinds() {
        let mut tally = Tally::default();
        tally.add(OutcomeKind::Updated);
        tally.add(OutcomeKind::Updated);
        tally.add(OutcomeKind::Failed);
        assert_eq!((tally.updated, tally.failed, tally.total()), (2, 1, 3));
    }
}

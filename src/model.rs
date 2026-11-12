//! Core domain types. The database rows, result lines and exit codes are
//! all derived from these.

use std::path::PathBuf;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsKind {
    Git,
}

impl VcsKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VcsKind::Git => "git",
        }
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
    UpToDate {
        rev: String,
        ahead: u32,
    },
    Skipped {
        reason: SkipReason,
        behind: u32,
    },
    Failed {
        reason: FailReason,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Dirty,
    Diverged,
    Detached,
    NoUpstream,
    RemoteMismatch,
    Disabled,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::Dirty => "dirty",
            SkipReason::Diverged => "diverged",
            SkipReason::Detached => "detached",
            SkipReason::NoUpstream => "no_upstream",
            SkipReason::RemoteMismatch => "remote_mismatch",
            SkipReason::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    NotACheckout,
    Clone,
    Fetch,
    FastForward,
    Interrupted,
}

impl FailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::NotACheckout => "not_a_checkout",
            FailReason::Clone => "clone",
            FailReason::Fetch => "fetch",
            FailReason::FastForward => "fast_forward",
            FailReason::Interrupted => "interrupted",
        }
    }
}

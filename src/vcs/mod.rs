//! The VCS abstraction. The sync engine sees only the `Vcs` trait; all
//! policy (dirty, diverged, detached) lives in `sync::worker`.

pub mod git;
pub mod remote;

use std::path::Path;

pub use crate::model::VcsKind;
use crate::proc::Tracer;

pub struct CloneSpec<'a> {
    pub url: &'a str,
    pub branch: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch {
        name: String,
        rev: String,
    },
    Detached {
        rev: String,
    },
    /// A branch with no commits yet.
    Unborn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    pub ahead: u32,
    pub behind: u32,
}

/// What became of a rebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebase {
    Done,
    /// Git refused to start, or stopped part-way and the rebase was
    /// aborted: HEAD, the branch and the working tree are as they were.
    /// `why` is short, such as "conflict in a.txt".
    Stopped {
        why: String,
    },
}

/// Where a submodule's checkout stands against the commit its parent
/// records, as `git submodule status` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmoduleStatus {
    /// Checked out at the recorded commit.
    Current,
    /// Not cloned or not checked out yet.
    Uninitialized,
    /// Checked out at another commit.
    Moved,
    /// Has merge conflicts.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submodule {
    /// Relative to the top-level checkout, with `/` separators.
    pub path: String,
    /// The checked-out commit, or the recorded one when not initialised.
    pub rev: String,
    pub status: SubmoduleStatus,
}

pub trait Vcs: Send + Sync {
    fn is_checkout(&self, path: &Path) -> bool;
    fn remote_url(&self, path: &Path) -> anyhow::Result<Option<String>>;
    fn head(&self, path: &Path) -> anyhow::Result<Head>;
    /// Whether tracked files differ from HEAD, staged or not. With
    /// `ignore_submodules`, a submodule at another commit or with changes
    /// inside it does not count.
    fn is_dirty(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<bool>;
    /// The tracked paths that differ from HEAD, staged or not.
    fn local_changes(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<Vec<String>>;
    /// The paths the upstream branch changes relative to HEAD.
    fn incoming_changes(&self, path: &Path) -> anyhow::Result<Vec<String>>;
    fn clone_into(&self, spec: &CloneSpec, dest: &Path) -> anyhow::Result<()>;
    fn fetch(&self, path: &Path) -> anyhow::Result<()>;
    /// `None` when the current branch has no upstream.
    fn divergence(&self, path: &Path) -> anyhow::Result<Option<Divergence>>;
    /// Fast-forward to the upstream. Refuses, changing nothing, when that
    /// would overwrite local changes; never stashes them.
    fn fast_forward(&self, path: &Path) -> anyhow::Result<()>;
    /// Replay local commits onto the upstream. Never leaves a rebase in
    /// progress: one that stops is aborted and reported as
    /// `Rebase::Stopped`. An error means git failed otherwise, or the
    /// abort itself failed; its message says which.
    fn rebase(&self, path: &Path) -> anyhow::Result<Rebase>;
    /// Every submodule, nested ones included.
    fn submodules(&self, path: &Path) -> anyhow::Result<Vec<Submodule>>;
    /// Whether HEAD is on a branch, or a branch, tag or remote-tracking
    /// branch contains it, so moving HEAD away cannot lose a commit.
    fn head_is_referenced(&self, path: &Path) -> anyhow::Result<bool>;
    /// Clone missing submodules and check each out at the commit its
    /// parent records, recursively.
    fn update_submodules(&self, path: &Path) -> anyhow::Result<()>;
}

/// The backend for `kind`. `tracer`, when given, sees every command run.
pub fn backend(kind: VcsKind, tracer: Option<Tracer>) -> Box<dyn Vcs> {
    match kind {
        VcsKind::Git => Box::new(git::Git::new(tracer)),
    }
}

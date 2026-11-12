//! The VCS abstraction. The sync engine sees only the `Vcs` trait; all
//! policy (dirty, diverged, detached) lives in `sync::worker`.

pub mod git;
pub mod remote;

use std::path::Path;

pub use crate::model::VcsKind;

pub struct CloneSpec<'a> {
    pub url: &'a str,
    pub branch: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch { name: String, rev: String },
    Detached { rev: String },
    Unborn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    pub ahead: u32,
    pub behind: u32,
}

pub trait Vcs: Send + Sync {
    fn is_checkout(&self, path: &Path) -> bool;
    fn remote_url(&self, path: &Path) -> anyhow::Result<Option<String>>;
    fn head(&self, path: &Path) -> anyhow::Result<Head>;
    fn is_dirty(&self, path: &Path) -> anyhow::Result<bool>;
    fn clone_into(&self, spec: &CloneSpec, dest: &Path) -> anyhow::Result<()>;
    fn fetch(&self, path: &Path) -> anyhow::Result<()>;
    /// `None` when the current branch has no upstream.
    fn divergence(&self, path: &Path) -> anyhow::Result<Option<Divergence>>;
    fn fast_forward(&self, path: &Path) -> anyhow::Result<()>;
}

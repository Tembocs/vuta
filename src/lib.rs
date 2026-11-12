//! Vuta keeps a set of source checkouts cloned and up to date.
//!
//! See `docs/ARCHITECTURE.md` for the design. The binary in `main.rs` is a thin
//! wrapper so that integration and unit tests reach the same code.

pub mod cli;
pub mod commands;
pub mod db;
pub mod error;
pub mod model;
pub mod paths;
pub mod proc;
pub mod sync;
pub mod transfer;
pub mod ui;
pub mod vcs;

#[cfg(test)]
pub(crate) mod testutil;

pub use error::Error;

//! The error type for failures of Vuta itself, and the exit codes.
//!
//! A repository failing is data (`Outcome::Failed`), not an `Error`.

use std::process::ExitCode;

use crate::db::DbError;

/// Exit codes (ARCHITECTURE.md section 10).
pub mod exit {
    /// Everything requested succeeded.
    pub const OK: u8 = 0;
    /// At least one repository, hook or check failed.
    pub const FAILED: u8 = 1;
    /// Usage error. clap exits with this code itself.
    pub const USAGE: u8 = 2;
    /// Vuta could not start the work.
    pub const CANNOT_START: u8 = 3;
    /// Interrupted with Ctrl-C.
    pub const INTERRUPTED: u8 = 130;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git was not found on PATH")]
    GitMissing,
    #[error("no repository named '{0}'")]
    UnknownRepo(String),
    #[error("no root named '{0}'; add one with: vuta root add {0} <path>")]
    UnknownRoot(String),
    #[error("another vuta sync is already running")]
    Locked,
    #[error("could not find a per-user data directory; pass --db <PATH> or set VUTA_DB")]
    NoDataDir,
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(exit::CANNOT_START)
    }
}

//! The Git backend. Every `git` call goes through `crate::proc`.
//!
//! The `Vcs` implementation lands in M2.

use std::io;

use crate::proc;

/// The output of `git --version`, or `Err` with `NotFound` when git is not
/// on PATH.
pub fn version() -> io::Result<String> {
    let out = proc::capture(proc::git_command().arg("--version"))?;
    if !out.status.success() {
        return Err(io::Error::other(out.stderr.trim().to_owned()));
    }
    Ok(out.stdout.trim().to_owned())
}

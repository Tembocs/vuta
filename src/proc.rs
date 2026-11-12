//! Spawning child processes with a shared environment and captured output.

use std::io;
use std::process::{Command, ExitStatus, Stdio};

/// Output of a finished child process, decoded lossily as UTF-8.
#[derive(Debug, Clone)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// A `git` command with the environment every call shares: no prompts,
/// stable English output, and a stalled-transfer timeout.
pub fn git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("LC_ALL", "C")
        .args([
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=30",
        ]);
    if cfg!(windows) {
        cmd.args(["-c", "core.longpaths=true"]);
    }
    cmd
}

/// Run `cmd` to completion with no stdin, capturing stdout and stderr.
pub fn capture(cmd: &mut Command) -> io::Result<Captured> {
    let output = cmd.stdin(Stdio::null()).output()?;
    Ok(Captured {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

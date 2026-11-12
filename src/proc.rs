//! Spawning child processes with a shared environment and captured output.

use std::ffi::OsStr;
use std::io;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;

/// A finished child process: what ran and what it printed, decoded lossily
/// as UTF-8.
#[derive(Debug, Clone)]
pub struct Captured {
    /// The command line, for `--verbose` and error messages.
    pub command: String,
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// The last non-empty line of stderr, or of stdout if stderr is empty.
    /// This is what a failed repository's result line shows.
    pub fn last_line(&self) -> &str {
        last_line(&self.stderr)
            .or_else(|| last_line(&self.stdout))
            .unwrap_or("")
    }
}

/// The last non-empty, trimmed line of `text`.
pub fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|line| !line.is_empty())
}

/// Receives every captured command, for `--verbose`. It is called on the
/// thread that ran the command.
pub type Tracer = Arc<dyn Fn(&Captured) + Send + Sync>;

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

/// A command that runs `command` through the platform shell: `sh -c` on
/// Unix, `cmd /C` on Windows. Used for hooks.
pub fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").raw_arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// Run `cmd` to completion with no stdin, capturing stdout and stderr.
pub fn capture(cmd: &mut Command) -> io::Result<Captured> {
    let command = describe(cmd);
    let output = cmd.stdin(Stdio::null()).output()?;
    Ok(Captured {
        command,
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// The command line as one string, quoting arguments that contain spaces.
pub fn describe(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(arg: &OsStr) -> String {
    let arg = arg.to_string_lossy();
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_and_quotes() {
        let mut cmd = Command::new("git");
        cmd.args(["-C", "/a b", "status"]);
        assert_eq!(describe(&cmd), "git -C \"/a b\" status");
    }

    #[test]
    fn last_line_skips_blank_lines() {
        assert_eq!(last_line("one\n two \n\n"), Some("two"));
        assert_eq!(last_line("\n  \n"), None);
    }

    #[test]
    fn shell_runs_a_command() {
        let out = capture(&mut shell_command("echo hello")).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.trim(), "hello");
    }
}

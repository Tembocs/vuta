//! All terminal output. Nothing outside this module prints.
//!
//! Results go to stdout; progress, warnings, errors and `--verbose` traces
//! go to stderr. Everything is written through `anstream`, which strips
//! colour when it is off, so callers always write styled text.

pub mod report;
pub mod table;
pub mod theme;

use std::fmt::{Display, Write as _};
use std::time::Duration;

use anstyle::Style;

use crate::cli::ColorWhen;
use crate::model::{GlyphMode, OutcomeKind, Tally, count};
use crate::proc::Captured;
use report::{HookEnd, ResultLine};
use table::Table;
use theme::{Glyphs, THEME};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

/// Set the colour choice for stdout and stderr. `Auto` honours `NO_COLOR`,
/// `CLICOLOR_FORCE`, `TERM=dumb` and whether each stream is a terminal.
pub fn apply_color(when: ColorWhen) {
    let choice = match when {
        ColorWhen::Auto => anstream::ColorChoice::Auto,
        ColorWhen::Always => anstream::ColorChoice::Always,
        ColorWhen::Never => anstream::ColorChoice::Never,
    };
    choice.write_global();
}

/// The status of one line in a checklist such as `vuta doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Pass,
    Note,
    Fail,
}

pub struct Ui {
    verbosity: Verbosity,
    glyphs: Glyphs,
}

impl Ui {
    pub fn new(verbosity: Verbosity) -> Self {
        Ui {
            verbosity,
            glyphs: Glyphs::detect(),
        }
    }

    /// Apply the `ui.glyphs` setting.
    pub fn set_glyph_mode(&mut self, mode: GlyphMode) {
        self.glyphs = Glyphs::for_mode(mode);
    }

    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
    }

    pub fn glyphs(&self) -> &Glyphs {
        &self.glyphs
    }

    fn quiet(&self) -> bool {
        self.verbosity == Verbosity::Quiet
    }

    /// A plain result line on stdout, printed at every verbosity.
    pub fn result(&self, line: impl Display) {
        anstream::println!("{line}");
    }

    /// An empty line on stdout, except in quiet mode.
    pub fn blank(&self) {
        if !self.quiet() {
            anstream::println!();
        }
    }

    /// A table on stdout.
    pub fn table(&self, table: &Table) {
        for line in table.render() {
            anstream::println!("{line}");
        }
    }

    /// A progress message on stderr, hidden in quiet mode.
    pub fn progress(&self, message: impl Display) {
        if !self.quiet() {
            anstream::eprintln!("{message}");
        }
    }

    pub fn warn(&self, message: impl Display) {
        let warn = THEME.warn;
        anstream::eprintln!("{warn}warning:{warn:#} {message}");
    }

    pub fn error(&self, message: impl Display) {
        let err = THEME.err;
        anstream::eprintln!("{err}error:{err:#} {message:#}");
    }

    /// A git command and its output, on stderr, only with `--verbose`.
    pub fn trace(&self, captured: &Captured) {
        if self.verbosity != Verbosity::Verbose {
            return;
        }
        let muted = THEME.muted;
        let status = match captured.status.code() {
            Some(0) => String::new(),
            Some(code) => format!("  (exit {code})"),
            None => "  (killed)".to_owned(),
        };
        anstream::eprintln!("{muted}  $ {}{status}{muted:#}", captured.command);
        for line in captured.stdout.lines().chain(captured.stderr.lines()) {
            if !line.trim().is_empty() {
                anstream::eprintln!("{muted}    {line}{muted:#}");
            }
        }
    }

    /// One checklist line on stdout: glyph, bold label, detail. Passing
    /// lines are hidden in quiet mode.
    pub fn check(&self, status: Check, label: &str, detail: impl Display) {
        if status == Check::Pass && self.quiet() {
            return;
        }
        let (style, glyph) = match status {
            Check::Pass => (THEME.ok, self.glyphs.updated),
            Check::Note => (THEME.warn, self.glyphs.skipped),
            Check::Fail => (THEME.err, self.glyphs.failed),
        };
        let mut line = Styled::new();
        line.push(&pad(glyph, self.glyphs.status_width()), style)
            .plain(" ")
            .push(&pad(label, 10), THEME.name)
            .plain(" ")
            .push(&detail.to_string(), THEME.muted);
        anstream::println!("{line}");
    }

    /// The dim "cloning <name>" progress line, on stderr.
    pub fn clone_started(&self, name: &str) {
        self.progress(report::clone_started_line(name));
    }

    /// A repository's result line on stdout. Quiet mode keeps only skips
    /// and failures.
    pub fn sync_result(&self, line: &ResultLine) {
        let kind = line.outcome.kind();
        if self.quiet() && !matches!(kind, OutcomeKind::Skipped | OutcomeKind::Failed) {
            return;
        }
        anstream::println!("{}", report::result_line(line, &self.glyphs));
    }

    /// The summary line, printed at every verbosity.
    pub fn summary(&self, tally: &Tally, elapsed: Duration) {
        anstream::println!("{}", report::summary_line(tally, elapsed, &self.glyphs));
    }

    pub fn hooks_heading(&self) {
        if !self.quiet() {
            anstream::println!("{}", report::hooks_heading());
        }
    }

    pub fn hook_started(&self, repo: &str, command: &str) {
        if !self.quiet() {
            anstream::println!("{}", report::hook_started_line(repo, command, &self.glyphs));
        }
    }

    pub fn hook_finished(&self, repo: &str, end: HookEnd, duration: Duration) {
        if end == HookEnd::Ok && self.quiet() {
            return;
        }
        anstream::println!(
            "{}",
            report::hook_finished_line(repo, end, duration, &self.glyphs)
        );
    }
}

/// Styled text that tracks its visible width, so it can be padded after
/// styling without counting escape codes.
#[derive(Debug, Clone, Default)]
pub struct Styled {
    text: String,
    width: usize,
}

impl Styled {
    pub fn new() -> Self {
        Styled::default()
    }

    pub fn push(&mut self, text: &str, style: Style) -> &mut Self {
        if style == Style::new() {
            self.text.push_str(text);
        } else {
            let _ = write!(self.text, "{style}{text}{style:#}");
        }
        self.width += text_width(text);
        self
    }

    pub fn plain(&mut self, text: &str) -> &mut Self {
        self.push(text, Style::new())
    }

    pub fn append(&mut self, other: &Styled) -> &mut Self {
        self.text.push_str(&other.text);
        self.width += other.width;
        self
    }

    /// Visible width in characters.
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn into_string(self) -> String {
        self.text
    }
}

impl Display for Styled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// Visible width of plain text, counted in characters. Wide (CJK) characters
/// count as one, which is good enough for names, paths and hashes.
pub fn text_width(text: &str) -> usize {
    text.chars().count()
}

/// `text` padded with spaces to `width` characters.
pub fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(text_width(text));
    format!("{text}{}", " ".repeat(fill))
}

/// `1.2s`, `2m05s` or `1h03m`.
pub fn fmt_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{:.1}s", duration.as_secs_f64())
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    }
}

/// How long ago something happened, given its age in seconds:
/// `just now`, `5 minutes ago`, `2 hours ago`, `3 days ago`.
pub fn fmt_age(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let (n, unit) = match seconds {
        s if s < MINUTE => return "just now".to_owned(),
        s if s < HOUR => (s / MINUTE, "minute"),
        s if s < DAY => (s / HOUR, "hour"),
        s if s < 30 * DAY => (s / DAY, "day"),
        s if s < 365 * DAY => (s / (30 * DAY), "month"),
        s => (s / (365 * DAY), "year"),
    };
    format!("{} ago", count(u32::try_from(n).unwrap_or(u32::MAX), unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_tracks_visible_width() {
        let mut s = Styled::new();
        s.push("zed", THEME.name)
            .plain(" → ")
            .push("8be04d7", THEME.rev);
        assert_eq!(s.width(), 13);
        assert!(s.to_string().contains('\x1b'));
        assert_eq!(
            anstream::adapter::strip_str(&s.to_string()).to_string(),
            "zed → 8be04d7"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(Duration::from_millis(1234)), "1.2s");
        assert_eq!(fmt_duration(Duration::from_secs(125)), "2m05s");
        assert_eq!(fmt_duration(Duration::from_secs(3780)), "1h03m");
    }

    #[test]
    fn ages() {
        assert_eq!(fmt_age(-5), "just now");
        assert_eq!(fmt_age(59), "just now");
        assert_eq!(fmt_age(60), "1 minute ago");
        assert_eq!(fmt_age(2 * 3600 + 5), "2 hours ago");
        assert_eq!(fmt_age(3 * 86400), "3 days ago");
        assert_eq!(fmt_age(400 * 86400), "1 year ago");
    }
}

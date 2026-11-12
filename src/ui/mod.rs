//! All terminal output. Nothing outside this module prints.
//!
//! Results go to stdout; progress, warnings and errors go to stderr.

pub mod report;
pub mod table;
pub mod theme;

use std::fmt::Display;

use crate::cli::ColorWhen;
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

    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
    }

    /// A plain result line on stdout, printed at every verbosity.
    pub fn result(&self, line: impl Display) {
        anstream::println!("{line}");
    }

    /// One checklist line on stdout: glyph, bold label, detail.
    /// Passing lines are hidden in quiet mode.
    pub fn check(&self, status: Check, label: &str, detail: impl Display) {
        if status == Check::Pass && self.verbosity == Verbosity::Quiet {
            return;
        }
        let (style, glyph) = match status {
            Check::Pass => (THEME.ok, self.glyphs.updated),
            Check::Note => (THEME.warn, self.glyphs.skipped),
            Check::Fail => (THEME.err, self.glyphs.failed),
        };
        let name = THEME.name;
        let muted = THEME.muted;
        anstream::println!(
            "{style}{glyph:<2}{style:#} {name}{label:<10}{name:#} {muted}{detail}{muted:#}"
        );
    }

    pub fn warn(&self, message: impl Display) {
        let warn = THEME.warn;
        anstream::eprintln!("{warn}warning:{warn:#} {message}");
    }

    pub fn error(&self, message: impl Display) {
        let err = THEME.err;
        anstream::eprintln!("{err}error:{err:#} {message:#}");
    }
}

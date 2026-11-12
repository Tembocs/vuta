//! Semantic styles and glyphs. The rest of the code names a role, never a
//! colour.

use anstyle::{AnsiColor, Style};

use crate::model::GlyphMode;

pub struct Theme {
    pub ok: Style,
    pub new: Style,
    pub muted: Style,
    pub warn: Style,
    pub err: Style,
    pub name: Style,
    pub rev: Style,
}

pub const THEME: Theme = Theme {
    ok: AnsiColor::Green.on_default(),
    new: AnsiColor::Cyan.on_default(),
    muted: Style::new().dimmed(),
    warn: AnsiColor::Yellow.on_default(),
    err: AnsiColor::Red.on_default().bold(),
    name: Style::new().bold(),
    rev: AnsiColor::Magenta.on_default(),
};

/// Status glyphs. Every status also carries a word, so colour is never the
/// only signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    pub updated: &'static str,
    pub cloned: &'static str,
    pub up_to_date: &'static str,
    pub skipped: &'static str,
    pub failed: &'static str,
    pub hook_running: &'static str,
    /// Between the old and new revision of an update.
    pub arrow: &'static str,
    /// Between the counts in the summary line.
    pub separator: &'static str,
}

impl Glyphs {
    pub const UNICODE: Glyphs = Glyphs {
        updated: "✓",
        cloned: "+",
        up_to_date: "=",
        skipped: "!",
        failed: "✗",
        hook_running: "▸",
        arrow: "→",
        separator: "·",
    };

    pub const ASCII: Glyphs = Glyphs {
        updated: "ok",
        cloned: "+",
        up_to_date: "=",
        skipped: "!",
        failed: "x",
        hook_running: ">",
        arrow: "->",
        separator: "-",
    };

    pub fn for_mode(mode: GlyphMode) -> Glyphs {
        match mode {
            GlyphMode::Auto => Glyphs::detect(),
            GlyphMode::Unicode => Glyphs::UNICODE,
            GlyphMode::Ascii => Glyphs::ASCII,
        }
    }

    /// The widest status glyph, so result lines stay aligned.
    pub fn status_width(&self) -> usize {
        [
            self.updated,
            self.cloned,
            self.up_to_date,
            self.skipped,
            self.failed,
        ]
        .iter()
        .map(|g| g.chars().count())
        .max()
        .unwrap_or(1)
    }

    /// `ui.glyphs = auto`: Unicode everywhere except Windows consoles other
    /// than Windows Terminal, whose default fonts may lack these characters.
    pub fn detect() -> Glyphs {
        if cfg!(windows) && std::env::var_os("WT_SESSION").is_none() {
            Glyphs::ASCII
        } else {
            Glyphs::UNICODE
        }
    }
}

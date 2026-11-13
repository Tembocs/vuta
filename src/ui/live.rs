//! Live progress during a sync: one line per repository a worker is on,
//! with how long it has taken so far, redrawn in place on stderr.
//!
//! ```text
//! [1/9] = up to date  surrealdb   a91c0de                              1.2s
//! [2/9] ✓ updated     zed         3f9a2c1 → 8be04d7   42 commits       6.1s
//!       ▸ cloning     turso       38s
//!       ▸ syncing     edit        2s
//! ```
//!
//! The block sits below the result lines. Before a result line is printed
//! the block is erased (cursor up, clear to the end of the screen), and it
//! is drawn again after, so result lines scroll up above it exactly as they
//! do without it. Once the last repository finishes the block is empty, so
//! nothing is left on screen when the hooks phase starts streaming output.
//!
//! Only used when `Ui::can_redraw` says the terminal can take it. Erasing
//! counts lines, so no line may wrap: the terminal's auto-wrap is off while
//! the block is drawn, so it clips a long line instead, and when `COLUMNS`
//! gives the width the line is cut to fit first, keeping the elapsed time.

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

use super::Ui;
use super::report::{self, LiveLine, ResultLine};

/// At most this many lines are drawn; more workers are summed up in one
/// last line, so the block always fits on the screen.
const MAX_LINES: usize = 10;

struct Active {
    repo_id: i64,
    name: String,
    cloning: bool,
    since: Instant,
}

pub struct Live<'a> {
    ui: &'a Ui,
    total: usize,
    name_width: usize,
    /// The terminal's width, when known.
    columns: Option<usize>,
    /// In the order the repositories started.
    active: Vec<Active>,
    /// How many lines are on screen now.
    drawn: usize,
}

impl<'a> Live<'a> {
    /// `total` and `name_width` are those of the run's result lines.
    pub fn new(ui: &'a Ui, total: usize, name_width: usize) -> Self {
        Live {
            ui,
            total,
            name_width,
            columns: columns(),
            active: Vec::new(),
            drawn: 0,
        }
    }

    pub fn started(&mut self, repo_id: i64, name: &str) {
        self.active.push(Active {
            repo_id,
            name: name.to_owned(),
            cloning: false,
            since: Instant::now(),
        });
        self.redraw();
    }

    pub fn cloning(&mut self, repo_id: i64) {
        if let Some(active) = self.active.iter_mut().find(|a| a.repo_id == repo_id) {
            active.cloning = true;
        }
        self.redraw();
    }

    /// Replace the repository's live line with its result line.
    pub fn finished(&mut self, repo_id: i64, line: &ResultLine) {
        self.active.retain(|a| a.repo_id != repo_id);
        self.erase();
        self.ui.sync_result(line);
        // The result is on stdout; it must reach the terminal before the
        // block is drawn on stderr below it.
        let _ = std::io::stdout().flush();
        self.draw();
    }

    /// Update the elapsed times.
    pub fn tick(&mut self) {
        self.redraw();
    }

    /// Take the block off the screen.
    pub fn clear(&mut self) {
        self.active.clear();
        self.erase();
    }

    fn redraw(&mut self) {
        self.erase();
        self.draw();
    }

    fn erase(&mut self) {
        if self.drawn > 0 {
            cursor(&format!("\x1b[{}A\x1b[J", self.drawn));
            self.drawn = 0;
        }
    }

    fn draw(&mut self) {
        if self.active.is_empty() {
            return;
        }
        // One column less than the terminal, so no line ends exactly at the
        // edge, where some terminals wrap.
        let max_width = self.columns.map_or(usize::MAX, |c| c - 1);
        let shown = if self.active.len() > MAX_LINES {
            MAX_LINES - 1
        } else {
            self.active.len()
        };
        let mut block = String::new();
        let now = Instant::now();
        for active in &self.active[..shown] {
            let line = LiveLine {
                total: self.total,
                name: &active.name,
                name_width: self.name_width,
                cloning: active.cloning,
                elapsed: now.duration_since(active.since),
                max_width,
            };
            let _ = writeln!(block, "{}", report::live_line(&line, self.ui.glyphs()));
        }
        let more = self.active.len() - shown;
        if more > 0 {
            let _ = writeln!(
                block,
                "{}",
                report::live_more_line(self.total, more, self.ui.glyphs())
            );
        }
        // Auto-wrap off (DECAWM) while drawing, on again right after.
        cursor("\x1b[?7l");
        self.ui.live_block(&block);
        cursor("\x1b[?7h");
        self.drawn = shown + usize::from(more > 0);
    }
}

impl Drop for Live<'_> {
    /// An error or panic part-way through a run still leaves a clean screen.
    fn drop(&mut self) {
        self.clear();
    }
}

/// Write a cursor control sequence to stderr. It goes around anstream's
/// colour stripping: moving the cursor is needed whether or not colour is
/// on. On Windows, `Ui::can_redraw` has already turned on the console's
/// handling of these sequences.
fn cursor(sequence: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(sequence.as_bytes());
    let _ = err.flush();
}

/// The terminal's width from `COLUMNS`, when the shell exports it.
fn columns() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.trim().parse().ok())
        .filter(|&c: &usize| c >= 20)
}

//! Command-line definitions. No logic lives here.

use std::path::PathBuf;

use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand, ValueEnum};

use crate::ui::Verbosity;

const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Green.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default().bold())
    .valid(AnsiColor::Green.on_default())
    .invalid(AnsiColor::Yellow.on_default());

/// Keep many source checkouts cloned and up to date.
#[derive(Debug, Parser)]
#[command(name = "vuta", version, about, styles = HELP_STYLES, propagate_version = true)]
pub struct Cli {
    /// Use this database file instead of the platform default
    #[arg(long, global = true, env = "VUTA_DB", value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// When to colour output
    #[arg(
        long,
        global = true,
        env = "VUTA_COLOR",
        value_name = "WHEN",
        default_value = "auto"
    )]
    pub color: ColorWhen,

    /// Print only warnings, errors and the summary
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Also print each git command and its output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn verbosity(&self) -> Verbosity {
        if self.quiet {
            Verbosity::Quiet
        } else if self.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect the database
    #[command(subcommand)]
    Db(DbCommand),

    /// Check that git is on PATH, the database is sound, and every root exists
    Doctor,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Print the database location
    Path,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

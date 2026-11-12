//! Command-line definitions. No logic lives here.

use std::path::PathBuf;

use clap::builder::styling::{AnsiColor, Styles};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::Shell;

use crate::model::Platform;
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
    #[arg(long, global = true, env = "VUTA_DB", value_name = "PATH", value_hint = ValueHint::FilePath)]
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
    /// Manage roots, the named base directories that hold checkouts
    #[command(subcommand)]
    Root(RootCommand),

    /// Register a repository and clone it
    Add(AddArgs),

    /// Register checkouts that already exist on disk
    Scan(ScanArgs),

    /// Clone what is missing and update the rest
    Sync(SyncArgs),

    /// One line per repository
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Local state of each checkout, without touching the network
    Status(StatusArgs),

    /// Everything about one repository
    Show(ShowArgs),

    /// Change a repository's name, URL, branch, path or enabled state
    Set(SetArgs),

    /// Add or remove tags, for example: vuta tag zed +rust -editor
    Tag(TagArgs),

    /// Forget a repository
    #[command(visible_alias = "rm")]
    Remove(RemoveArgs),

    /// Manage commands that run after a repository syncs
    #[command(subcommand)]
    Hook(HookCommand),

    /// Recent sync runs and their results
    Log(LogArgs),

    /// Read and write settings
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Print a shell completion script, for example: vuta completions bash
    Completions {
        /// The shell to complete for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Inspect the database
    #[command(subcommand)]
    Db(DbCommand),

    /// Check that git is on PATH, the database is sound, and every root exists
    Doctor,
}

#[derive(Debug, Subcommand)]
pub enum RootCommand {
    /// Register a base directory. The first root becomes the default.
    Add {
        name: String,
        #[arg(value_hint = ValueHint::DirPath)]
        path: PathBuf,
        /// Make this the default root
        #[arg(long)]
        default: bool,
    },
    /// Show roots and how many repositories each holds
    #[command(visible_alias = "ls")]
    List,
    /// Change the default root
    Default { name: String },
    /// Point a root at a new location. No files are moved.
    Move {
        name: String,
        #[arg(value_hint = ValueHint::DirPath)]
        new_path: PathBuf,
    },
    /// Remove a root that holds no repositories
    #[command(visible_alias = "rm")]
    Remove { name: String },
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// The remote to clone from
    #[arg(value_hint = ValueHint::Url)]
    pub url: String,
    /// Where to put the checkout, relative to the root [default: the name]
    pub path: Option<String>,
    /// Repository name [default: inferred from the URL]
    #[arg(long)]
    pub name: Option<String>,
    /// Root to put the checkout under [default: the default root]
    #[arg(long, value_name = "ROOT")]
    pub root: Option<String>,
    /// Tag the repository; repeat for several tags
    #[arg(short, long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Branch to clone [default: the remote's default branch]
    #[arg(long)]
    pub branch: Option<String>,
    /// Partial clone filter, for example blob:none
    #[arg(long, value_name = "FILTER")]
    pub filter: Option<String>,
    /// Shallow clone with this many commits
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub depth: Option<u32>,
    /// Only register the repository; the next sync clones it
    #[arg(long)]
    pub no_clone: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Directory to search [default: the root's path]
    #[arg(value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
    /// Register under this root [default: the root that contains each checkout]
    #[arg(long, value_name = "ROOT")]
    pub root: Option<String>,
    /// Tag each repository with its parent directory's name
    #[arg(long)]
    pub tag_from_parent: bool,
    /// How many directory levels below DIR to search
    #[arg(long, value_name = "N", default_value_t = 4,
          value_parser = clap::value_parser!(u32).range(1..))]
    pub max_depth: u32,
    /// Show what would be registered without registering it
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Repositories to sync [default: every enabled repository]
    pub names: Vec<String>,
    /// Only repositories with this tag; repeat to allow several
    #[arg(short, long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Only repositories under this root
    #[arg(long, value_name = "ROOT")]
    pub root: Option<String>,
    /// How many repositories to sync at once [default: sync.jobs]
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=64))]
    pub jobs: Option<u32>,
    /// Fetch and report how far behind, without changing any working tree
    #[arg(long)]
    pub fetch_only: bool,
    /// Run no hooks
    #[arg(long, conflicts_with = "force_hooks")]
    pub no_hooks: bool,
    /// Run hooks even for repositories that did not change
    #[arg(long)]
    pub force_hooks: bool,
    /// Count skipped repositories as failures for the exit code
    #[arg(long)]
    pub strict: bool,
    /// Print what would be cloned or fetched, and exit
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only repositories with this tag; repeat to allow several
    #[arg(short, long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Only repositories under this root
    #[arg(long, value_name = "ROOT")]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Repositories to inspect [default: all]
    pub names: Vec<String>,
    /// Only repositories with this tag; repeat to allow several
    #[arg(short, long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Only repositories under this root
    #[arg(long, value_name = "ROOT")]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    pub name: String,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("change").required(true).multiple(true)))]
pub struct SetArgs {
    pub name: String,
    /// Rename the repository
    #[arg(long = "name", value_name = "NEW_NAME", group = "change")]
    pub new_name: Option<String>,
    /// Change the remote URL
    #[arg(long, group = "change", value_hint = ValueHint::Url)]
    pub url: Option<String>,
    /// Clone this branch
    #[arg(long, group = "change", conflicts_with = "default_branch")]
    pub branch: Option<String>,
    /// Clone the remote's default branch
    #[arg(long, group = "change")]
    pub default_branch: bool,
    /// Move the record to this path under its root. No files are moved.
    #[arg(long, group = "change")]
    pub path: Option<String>,
    /// Move the record to another root. No files are moved.
    #[arg(long, value_name = "ROOT", group = "change")]
    pub root: Option<String>,
    /// Partial clone filter for future clones
    #[arg(
        long,
        value_name = "FILTER",
        group = "change",
        conflicts_with = "no_filter"
    )]
    pub filter: Option<String>,
    /// Make future clones full clones
    #[arg(long, group = "change")]
    pub no_filter: bool,
    /// Shallow depth for future clones
    #[arg(long, value_name = "N", group = "change", conflicts_with = "no_depth",
          value_parser = clap::value_parser!(u32).range(1..))]
    pub depth: Option<u32>,
    /// Make future clones not shallow
    #[arg(long, group = "change")]
    pub no_depth: bool,
    /// Include the repository in syncs
    #[arg(long, group = "change", conflicts_with = "disable")]
    pub enable: bool,
    /// Leave the repository out of syncs
    #[arg(long, group = "change")]
    pub disable: bool,
}

#[derive(Debug, Args)]
pub struct TagArgs {
    pub name: String,
    /// +TAG adds a tag, -TAG removes one, and a bare TAG adds it.
    /// With none, print the repository's tags.
    #[arg(
        value_name = "+TAG|-TAG",
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    pub changes: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    pub name: String,
    /// Also delete the checkout from disk, after a confirmation
    #[arg(long)]
    pub delete_files: bool,
    /// Do not ask for confirmation
    #[arg(short, long, requires = "delete_files")]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Attach a shell command that runs after the repository syncs
    Add {
        repo: String,
        #[arg(value_hint = ValueHint::CommandString)]
        command: String,
        /// Run only on this kind of OS
        #[arg(long, default_value = "any", value_parser = platform_parser())]
        platform: Platform,
        /// Run after every sync, not only when the revision changed
        #[arg(long)]
        always: bool,
    },
    /// List hooks, of one repository or of all
    #[command(visible_alias = "ls")]
    List { repo: Option<String> },
    /// Remove a hook by the id that `vuta hook list` shows
    #[command(visible_alias = "rm")]
    Remove { id: i64 },
}

fn platform_parser() -> impl TypedValueParser<Value = Platform> {
    PossibleValuesParser::new(Platform::ALL.iter().map(|p| p.as_str()))
        .map(|s| s.parse::<Platform>().expect("a listed platform parses"))
}

#[derive(Debug, Args)]
pub struct LogArgs {
    /// Show this repository's results instead of whole runs
    pub name: Option<String>,
    /// How many runs (or results) to show
    #[arg(short = 'n', long, value_name = "N", default_value_t = 10,
          value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print one setting's value
    Get { key: String },
    /// Change a setting
    Set { key: String, value: String },
    /// Return a setting to its default
    Unset { key: String },
    /// Show every setting with its value
    #[command(visible_alias = "ls")]
    List,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Print the database location
    Path,
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Command};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn tag_accepts_leading_hyphens() {
        let cli = Cli::try_parse_from(["vuta", "tag", "zed", "-editor", "+rust", "-v"]).unwrap();
        match cli.command {
            Command::Tag(args) => assert_eq!(args.changes, ["-editor", "+rust", "-v"]),
            other => panic!("{other:?}"),
        }
        assert!(!cli.verbose);
    }
}

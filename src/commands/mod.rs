//! One module per subcommand: read arguments, call the core, render.

pub mod add;
pub mod config;
pub mod db;
pub mod doctor;
pub mod hook;
pub mod list;
pub mod log;
pub mod remove;
pub mod root;
pub mod scan;
pub mod set;
pub mod show;
pub mod status;
pub mod sync;
pub mod tag;

use std::path::PathBuf;
use std::process::ExitCode;

use crate::Error;
use crate::cli::{Cli, Command, DbCommand};
use crate::paths;
use crate::ui::Ui;

/// What every command gets: the terminal and the resolved database path.
/// The database is opened only by commands that need it.
pub struct Ctx<'a> {
    pub ui: &'a Ui,
    pub db_path: PathBuf,
}

impl Ctx<'_> {
    pub fn open_db(&self) -> Result<rusqlite::Connection, Error> {
        Ok(crate::db::open(&self.db_path)?)
    }
}

pub fn run(cli: &Cli, ui: &Ui) -> Result<ExitCode, Error> {
    let ctx = Ctx {
        ui,
        db_path: paths::resolve_db_path(cli.db.as_deref())?,
    };
    match &cli.command {
        Command::Db(DbCommand::Path) => db::path(&ctx),
        Command::Doctor => doctor::run(&ctx),
    }
}

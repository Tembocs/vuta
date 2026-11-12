//! `vuta doctor`: check that git is on PATH, the database is sound, and every
//! root exists. Problems are reported, not raised, and make the exit code 1.

use std::io;
use std::process::ExitCode;

use super::Ctx;
use crate::Error;
use crate::db;
use crate::error::exit;
use crate::ui::Check;
use crate::vcs::git;

pub fn run(ctx: &mut Ctx) -> Result<ExitCode, Error> {
    let mut healthy = true;
    let conn = ctx.open_db();
    let ui = &*ctx.ui;

    match git::version() {
        Ok(version) => ui.check(Check::Pass, "git", version),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            healthy = false;
            ui.check(Check::Fail, "git", Error::GitMissing);
        }
        Err(err) => {
            healthy = false;
            ui.check(
                Check::Fail,
                "git",
                format_args!("git --version failed: {err}"),
            );
        }
    }

    let conn = match conn {
        Ok(conn) => conn,
        Err(err) => {
            ui.check(Check::Fail, "database", err);
            return Ok(ExitCode::from(exit::FAILED));
        }
    };
    match db::integrity_problem(&conn)? {
        None => {
            let version = db::schema_version(&conn)?;
            ui.check(
                Check::Pass,
                "database",
                format_args!("{} (schema {version})", ctx.db_path.display()),
            );
        }
        Some(problem) => {
            healthy = false;
            ui.check(
                Check::Fail,
                "database",
                format_args!("{}: {problem}", ctx.db_path.display()),
            );
        }
    }

    let roots = db::roots::all(&conn)?;
    if roots.is_empty() {
        ui.check(
            Check::Note,
            "roots",
            "none registered; add one with: vuta root add <name> <path>",
        );
    }
    for root in roots {
        if root.path.is_dir() {
            ui.check(Check::Pass, &root.name, root.path.display());
        } else {
            healthy = false;
            ui.check(
                Check::Fail,
                &root.name,
                format_args!("{} does not exist", root.path.display()),
            );
        }
    }

    Ok(ExitCode::from(if healthy {
        exit::OK
    } else {
        exit::FAILED
    }))
}

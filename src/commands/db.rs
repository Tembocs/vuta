//! `vuta db path`.

use std::process::ExitCode;

use super::Ctx;
use crate::Error;

pub fn path(ctx: &Ctx) -> Result<ExitCode, Error> {
    ctx.ui.result(ctx.db_path.display());
    Ok(ExitCode::SUCCESS)
}

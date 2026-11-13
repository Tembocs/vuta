//! `vuta config get|set|unset|list`: rows in the `settings` table.

use std::process::ExitCode;

use super::{Ctx, print_json};
use crate::Error;
use crate::cli::{ConfigCommand, Format};
use crate::db::settings;
use crate::ui::table::{Cell, Table};
use crate::ui::theme::THEME;
use crate::ui::{Styled, json};

pub fn run(ctx: &mut Ctx, cmd: &ConfigCommand) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;
    match cmd {
        ConfigCommand::Get { key } => {
            ui.result(settings::get(&conn, key)?.value);
        }
        ConfigCommand::Set { key, value } => {
            settings::set(&conn, key, value)?;
            let mut line = Styled::new();
            line.push(key, THEME.name).plain(" = ").plain(value);
            ui.result(line);
        }
        ConfigCommand::Unset { key } => {
            settings::unset(&conn, key)?;
            let setting = settings::get(&conn, key)?;
            let mut line = Styled::new();
            line.push(setting.key, THEME.name)
                .plain(" = ")
                .plain(&setting.value)
                .push(" (default)", THEME.muted);
            ui.result(line);
        }
        ConfigCommand::List { format } => {
            let settings = settings::list(&conn)?;
            if *format == Format::Json {
                let items = settings
                    .iter()
                    .zip(settings::KNOWN)
                    .map(|(setting, known)| json::setting(setting, known));
                print_json(ui, &serde_json::Value::Array(items.collect()))?;
                return Ok(ExitCode::SUCCESS);
            }
            let mut table = Table::new(["KEY", "VALUE", "DESCRIPTION"]);
            for (setting, known) in settings.into_iter().zip(settings::KNOWN) {
                let value = if setting.is_default {
                    Cell::new(format!("{} (default)", setting.value), THEME.muted)
                } else {
                    Cell::plain(setting.value)
                };
                table.push(vec![
                    Cell::new(setting.key, THEME.name),
                    value,
                    Cell::new(known.help, THEME.muted),
                ]);
            }
            ui.table(&table);
        }
    }
    Ok(ExitCode::SUCCESS)
}

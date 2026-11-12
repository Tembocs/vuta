use std::process::ExitCode;

use clap::Parser;
use vuta::cli::Cli;
use vuta::commands;
use vuta::ui::{self, Ui};

fn main() -> ExitCode {
    let cli = Cli::parse();
    ui::apply_color(cli.color);
    let ui = Ui::new(cli.verbosity());
    match commands::run(&cli, &ui) {
        Ok(code) => code,
        Err(err) => {
            ui.error(&err);
            err.exit_code()
        }
    }
}

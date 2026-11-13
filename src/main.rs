use std::process::ExitCode;

use clap::Parser;
use vuta::cli::Cli;
use vuta::commands;
use vuta::ui::{self, Ui};

fn main() -> ExitCode {
    // A shell asking for completions gets them here, before any parsing.
    commands::completions::handle_request();
    let cli = Cli::parse();
    ui::apply_color(cli.color);
    let mut ui = Ui::new(cli.verbosity());
    match commands::run(&cli, &mut ui) {
        Ok(code) => code,
        Err(err) => {
            ui.error(&err);
            err.exit_code()
        }
    }
}

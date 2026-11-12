//! `vuta completions <shell>`: a completion script generated from `cli.rs`.
//!
//! The script completes subcommands, flags and their fixed values. It does
//! not read the database, so repository and root names are not completed.

use std::process::ExitCode;

use anyhow::anyhow;
use clap::{CommandFactory, ValueEnum};
pub use clap_complete::Shell;

use crate::Error;
use crate::cli::Cli;
use crate::ui::Ui;

/// The binary name the scripts complete.
const BIN: &str = "vuta";

pub fn run(ui: &Ui, shell: Shell) -> Result<ExitCode, Error> {
    ui.verbatim(&script(shell))
        .map_err(|err| anyhow!("cannot write the completion script: {err}"))?;
    Ok(ExitCode::SUCCESS)
}

/// The function clap_complete's fish script defines to test which
/// subcommand is being completed.
const FISH_USING_SUBCOMMAND: &str = "__fish_vuta_using_subcommand";

/// The completion script for `shell`. Packaging calls this too.
pub fn script(shell: Shell) -> String {
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), BIN, &mut buf);
    let mut text = String::from_utf8(buf).expect("clap_complete writes UTF-8");
    if shell == Shell::Fish {
        // The fish generator completes the values of options but not of
        // positional arguments, so `vuta completions <TAB>` would offer
        // files. This is the only positional with a fixed set of values.
        let shells: Vec<String> = Shell::value_variants()
            .iter()
            .map(|s| s.to_string())
            .collect();
        text.push_str(&format!(
            "complete -c {BIN} -n \"{FISH_USING_SUBCOMMAND} completions\" -f -a \"{}\"\n",
            shells.join(" ")
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shell_gets_a_script_naming_the_subcommands() {
        for shell in Shell::value_variants() {
            let text = script(*shell);
            for word in ["sync", "scan", "completions", "fetch-only"] {
                assert!(text.contains(word), "{shell}: no {word}");
            }
        }
    }

    #[test]
    fn fish_completes_the_shell_names() {
        let text = script(Shell::Fish);
        // The extra line relies on the generator's own helper function.
        assert!(text.contains(&format!("function {FISH_USING_SUBCOMMAND}")));
        assert!(text.ends_with(
            "-n \"__fish_vuta_using_subcommand completions\" -f -a \"bash elvish fish powershell zsh\"\n"
        ));
    }
}

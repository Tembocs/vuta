//! Repository tasks, run with `cargo xtask <task>`.
//!
//! - `cargo xtask deb` builds a Debian package with `dpkg-deb` (on Linux).
//! - `cargo xtask msix` builds an MSIX package with the Windows SDK's
//!   `makeappx` and, given a certificate, signs it with `signtool` (on
//!   Windows).
//!
//! Packages land in `target/package/`. See docs/PACKAGING.md.

mod deb;
mod meta;
mod msix;
mod png;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "Vuta repository tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Build a Debian package (.deb) with dpkg-deb
    Deb(deb::DebArgs),
    /// Build an MSIX package with makeappx, and sign it with signtool
    Msix(msix::MsixArgs),
}

/// Options every package task shares.
#[derive(Debug, Args)]
pub struct Common {
    /// Rust target triple to build for [default: the host]
    #[arg(long, value_name = "TRIPLE")]
    pub target: Option<String>,
    /// Package the existing release binary instead of building it
    #[arg(long)]
    pub no_build: bool,
    /// Directory to write the package to [default: target/package]
    #[arg(long, value_name = "DIR")]
    pub out_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.task {
        Task::Deb(args) => deb::run(args),
        Task::Msix(args) => msix::run(args),
    };
    match result {
        Ok(package) => {
            println!("{}", package.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

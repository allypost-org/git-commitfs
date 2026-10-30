mod app;
mod checkout;
mod cli;
mod copy;
mod git;
mod ignored;
mod runner;
mod snapshot;
mod workspace;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let outcome = match std::env::current_dir() {
        Ok(cwd) => app::execute(cli.command, &cwd),
        Err(error) => app::AppOutcome::failure(error),
    };
    for message in outcome.diagnostics {
        eprintln!("git-commitfs: {message}");
    }
    if let Some(path) = outcome.retained {
        eprintln!("git-commitfs: kept workspace at {}", path.display());
    }
    ExitCode::from(outcome.code)
}

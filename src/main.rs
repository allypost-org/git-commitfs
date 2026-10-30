mod app;
mod checkout;
mod cli;
mod git;
mod runner;
mod snapshot;
mod workspace;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let result = std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|cwd| app::execute(cli.command, &cwd));
    match result {
        Ok(status) => ExitCode::from(status.shell_code()),
        Err(error) => {
            eprintln!("git-commitfs: {error:#}");
            ExitCode::from(125)
        }
    }
}

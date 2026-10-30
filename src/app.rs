use std::path::Path;

use anyhow::Result;

use crate::{checkout, cli::Commands, runner, snapshot, workspace};

pub fn execute(command: Commands, invocation_dir: &Path) -> Result<runner::RunStatus> {
    let Commands::Run(options) = command;
    let source = snapshot::SourceRepository::discover(invocation_dir)?;
    let run = workspace::RunDirectory::allocate(&source.root)?;
    let result = (|| {
        let candidate = snapshot::capture(&source, &run)?;
        checkout::materialize(&candidate, &run)?;
        runner::run_command(&options.argv, run.checkout(), &source.child_environment())
    })();
    let cleanup = run.close();
    match result {
        Ok(status) if status.shell_code() == 0 => {
            cleanup?;
            Ok(status)
        }
        other => {
            if let Err(error) = cleanup {
                eprintln!("git-commitfs: {error:#}");
            }
            other
        }
    }
}

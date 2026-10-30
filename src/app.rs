use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::{checkout, cli::Commands, ignored, runner, snapshot, workspace};

pub struct AppOutcome {
    pub code: u8,
    pub retained: Option<PathBuf>,
    pub diagnostics: Vec<String>,
}

impl AppOutcome {
    pub fn failure(error: impl std::fmt::Display) -> Self {
        Self {
            code: 125,
            retained: None,
            diagnostics: vec![format!("{error:#}")],
        }
    }
}

pub fn execute(command: Commands, invocation_dir: &Path) -> AppOutcome {
    let Commands::Run(options) = command;
    let source = match snapshot::SourceRepository::discover(invocation_dir) {
        Ok(source) => source,
        Err(error) => return AppOutcome::failure(error),
    };
    let run = match workspace::RunDirectory::allocate(&source.root) {
        Ok(run) => run,
        Err(error) => return AppOutcome::failure(error),
    };
    let result = (|| {
        let candidate = snapshot::capture(&source, &run)?;
        checkout::materialize(&candidate, &run)?;
        if options.include_ignored {
            ignored::import(&source, &candidate, &run, options.copy_mode)?;
        }
        run.clear_scratch()?;
        runner::run_command(&options.argv, run.checkout(), &source.child_environment())
    })();
    let finalized = run.finalize(options.keep, &result);
    finish(result, finalized)
}

fn finish(result: Result<runner::RunStatus>, finalized: workspace::Finalization) -> AppOutcome {
    let mut diagnostics = Vec::new();
    let mut code = match result {
        Ok(status) => status.shell_code(),
        Err(error) => {
            diagnostics.push(format!("{error:#}"));
            125
        }
    };
    if let Some(error) = finalized.cleanup_error {
        diagnostics.push(format!("{error:#}"));
        if code == 0 {
            code = 125;
        }
    }
    AppOutcome {
        code,
        retained: finalized.retained,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_failure_never_masks_a_command_failure() {
        for (child, expected) in [(0, 125), (17, 17), (125, 125)] {
            let outcome = finish(
                Ok(runner::RunStatus::Exited(child)),
                workspace::Finalization {
                    retained: Some("retained".into()),
                    cleanup_error: Some(anyhow::anyhow!("cleanup failed")),
                },
            );
            assert_eq!(outcome.code, expected);
            assert_eq!(outcome.diagnostics, ["cleanup failed"]);
            assert!(outcome.retained.is_some());
        }
    }
}

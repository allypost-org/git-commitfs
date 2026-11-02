use std::{ffi::OsString, io, path::Path, process::Command};

use anyhow::{Context, Result};

use crate::git::ChildEnvironment;

pub enum RunStatus {
    Exited(u8),
    Signaled(i32),
    NotFound,
    NotExecutable,
}

impl RunStatus {
    pub fn shell_code(&self) -> u8 {
        match self {
            Self::Exited(code) => *code,
            Self::Signaled(signal) => u8::try_from(128 + signal).unwrap_or(125),
            Self::NotFound => 127,
            Self::NotExecutable => 126,
        }
    }
}

pub fn run_command(
    argv: &[OsString],
    cwd: &Path,
    env: &ChildEnvironment,
    supervisor: &crate::process::Supervisor,
) -> Result<RunStatus> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).current_dir(cwd).env("PWD", cwd);
    env.apply(&mut command);
    match supervisor.status(&mut command) {
        Ok(status) => Ok(status.code().map_or_else(
            || RunStatus::Signaled(crate::platform::process::termination_signal(status)),
            |code| RunStatus::Exited(u8::try_from(code).unwrap_or(125)),
        )),
        Err(error) => match classify_start_error(&error) {
            Some(RunStatus::NotFound) => {
                eprintln!("git-commitfs: command not found: {:?}", argv[0]);
                Ok(RunStatus::NotFound)
            }
            Some(RunStatus::NotExecutable) => {
                eprintln!("git-commitfs: command is not executable: {:?}", argv[0]);
                Ok(RunStatus::NotExecutable)
            }
            _ => Err(error).context("run command"),
        },
    }
}

fn classify_start_error(error: &anyhow::Error) -> Option<RunStatus> {
    error.downcast_ref::<crate::process::SpawnFailure>()?;
    let error = error.downcast_ref::<io::Error>()?;
    if error.kind() == io::ErrorKind::NotFound {
        Some(RunStatus::NotFound)
    } else if crate::platform::process::not_executable(error) {
        Some(RunStatus::NotExecutable)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervision_errors_are_not_misreported_as_command_launch_errors() {
        let error = anyhow::Error::new(io::Error::from_raw_os_error(libc::EACCES));
        assert!(classify_start_error(&error).is_none());
        let error = error.context(crate::process::SpawnFailure);
        assert!(matches!(
            classify_start_error(&error),
            Some(RunStatus::NotExecutable)
        ));
    }
}

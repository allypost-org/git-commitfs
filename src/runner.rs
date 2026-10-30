use std::{ffi::OsString, io, os::unix::process::ExitStatusExt, path::Path, process::Command};

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

pub fn run_command(argv: &[OsString], cwd: &Path, env: &ChildEnvironment) -> Result<RunStatus> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).current_dir(cwd).env("PWD", cwd);
    env.apply(&mut command);
    match command.status() {
        Ok(status) => Ok(status.code().map_or_else(
            || RunStatus::Signaled(status.signal().unwrap_or(0)),
            |code| RunStatus::Exited(u8::try_from(code).unwrap_or(125)),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("git-commitfs: command not found: {:?}", argv[0]);
            Ok(RunStatus::NotFound)
        }
        Err(error)
            if error.kind() == io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(libc::ENOEXEC) =>
        {
            eprintln!("git-commitfs: command is not executable: {:?}", argv[0]);
            Ok(RunStatus::NotExecutable)
        }
        Err(error) => Err(error).context("start command"),
    }
}

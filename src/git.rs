use std::{
    ffi::{OsStr, OsString},
    fs::File,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
};

use anyhow::{Context, Result, bail};

use crate::process::Supervisor;

#[derive(Clone, Default)]
pub struct ChildEnvironment {
    pub remove: Vec<OsString>,
    pub set: Vec<(OsString, OsString)>,
}

impl ChildEnvironment {
    pub fn apply(&self, command: &mut Command) {
        command.env_remove("GIT_ATTR_SOURCE");
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }
}

#[derive(Clone)]
pub struct Git {
    pub cwd: PathBuf,
    pub env: ChildEnvironment,
    pub supervisor: Arc<Supervisor>,
}

impl Git {
    pub fn source(cwd: &Path, supervisor: Arc<Supervisor>) -> Self {
        Self {
            cwd: cwd.to_owned(),
            env: ChildEnvironment::default(),
            supervisor,
        }
    }

    pub fn private(cwd: &Path, remove: &[OsString], supervisor: Arc<Supervisor>) -> Self {
        Self {
            cwd: cwd.to_owned(),
            supervisor,
            env: ChildEnvironment {
                remove: remove.to_owned(),
                set: vec![
                    ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                    ("GIT_CONFIG_SYSTEM".into(), "/dev/null".into()),
                    ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ],
            },
        }
    }

    pub fn command(&self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Command {
        let mut command = Command::new("git");
        command.current_dir(&self.cwd).stdin(Stdio::null());
        self.env.apply(&mut command);
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0");
        command.args(["-c", "core.fsmonitor=false"]).args(args);
        command
    }

    pub fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = self
            .supervisor
            .capture(&mut self.command(args), None, None)
            .context("execute Git (Git >= 2.43 is required)")?;
        checked(output, args[0])
    }

    pub fn os_output(&self, args: &[&OsStr]) -> Result<Vec<u8>> {
        let output = self
            .supervisor
            .capture(&mut self.command(args), None, None)
            .context("execute Git")?;
        checked(output, "operation")
    }

    pub fn optional_output(&self, args: &[&str]) -> Result<Option<Vec<u8>>> {
        let output = self
            .supervisor
            .capture(&mut self.command(args), None, None)?;
        if output.status.code() == Some(1) {
            Ok(None)
        } else {
            checked(output, args[0]).map(Some)
        }
    }

    pub fn with_env(&self, name: &str, value: impl Into<OsString>) -> Self {
        let mut git = self.clone();
        git.env.set.push((name.into(), value.into()));
        git
    }

    pub fn input_output(&self, args: &[&str], input: Vec<u8>) -> Result<Output> {
        self.supervisor
            .capture(&mut self.command(args), Some(input), None)
    }

    pub fn pack(&self, input: Vec<u8>, path: &Path) -> Result<()> {
        let output = self.supervisor.capture(
            &mut self.command(["pack-objects", "--stdout", "--revs"]),
            Some(input),
            Some(File::create(path)?),
        )?;
        checked(output, "pack-objects")?;
        Ok(())
    }

    pub fn import_pack(&self, path: &Path) -> Result<()> {
        let mut command = self.command(["index-pack", "--stdin"]);
        command.stdin(File::open(path)?);
        let output = self.supervisor.capture(&mut command, None, None)?;
        checked(output, "index-pack")?;
        Ok(())
    }
}

pub fn checked(output: Output, operation: &str) -> Result<Vec<u8>> {
    if !output.status.success() {
        bail!(
            "Git {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    Ok(output.stdout)
}

pub fn record(mut bytes: Vec<u8>) -> OsString {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    crate::platform::fs::os_string(bytes)
}

pub fn quote_path(path: &OsStr) -> OsString {
    let mut text = String::from("\"");
    for &byte in path.as_encoded_bytes() {
        match byte {
            b'"' => text.push_str("\\\""),
            b'\\' => text.push_str("\\\\"),
            b' '..=b'~' => text.push(char::from(byte)),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(text, "\\{byte:03o}");
            }
        }
    }
    text.push('"');
    text.into()
}

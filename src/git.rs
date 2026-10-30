use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Write,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail};

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
}

impl Git {
    pub fn source(cwd: &Path) -> Self {
        Self {
            cwd: cwd.to_owned(),
            env: ChildEnvironment::default(),
        }
    }

    pub fn private(cwd: &Path, remove: &[OsString]) -> Self {
        Self {
            cwd: cwd.to_owned(),
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
        command.current_dir(&self.cwd);
        self.env.apply(&mut command);
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0");
        command.args(["-c", "core.fsmonitor=false"]).args(args);
        command
    }

    pub fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = self
            .command(args)
            .stdin(Stdio::null())
            .output()
            .context("execute Git (Git >= 2.43 is required)")?;
        checked(output, args[0])
    }

    pub fn os_output(&self, args: &[&OsStr]) -> Result<Vec<u8>> {
        let output = self
            .command(args)
            .stdin(Stdio::null())
            .output()
            .context("execute Git")?;
        checked(output, "operation")
    }

    pub fn optional_output(&self, args: &[&str]) -> Result<Option<Vec<u8>>> {
        let output = self
            .command(args)
            .stdin(Stdio::null())
            .output()
            .context("execute optional Git query")?;
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

    pub fn pack(&self, tree: &OsStr, path: &Path) -> Result<()> {
        let mut command = self.command(["pack-objects", "--stdout", "--revs"]);
        command
            .stdin(Stdio::piped())
            .stdout(File::create(path)?)
            .stderr(Stdio::piped());
        let mut child = command.spawn().context("export candidate Git objects")?;
        let mut input = child.stdin.take().context("Git pack input")?;
        input.write_all(tree.as_encoded_bytes())?;
        input.write_all(b"\n")?;
        drop(input);
        checked(child.wait_with_output()?, "pack-objects")?;
        Ok(())
    }

    pub fn import_pack(&self, path: &Path) -> Result<()> {
        let output = self
            .command(["index-pack", "--stdin"])
            .stdin(File::open(path)?)
            .output()
            .context("import candidate Git objects")?;
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
    OsString::from_vec(bytes)
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

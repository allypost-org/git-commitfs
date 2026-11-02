#![allow(dead_code)]

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

pub struct Repo {
    pub storage: tempfile::TempDir,
    pub root: PathBuf,
}

impl Repo {
    pub fn new() -> Self {
        Self::with_format("sha1")
    }

    pub fn with_format(format: &str) -> Self {
        let storage = tempfile::tempdir().expect("fixture storage");
        let root = fs::canonicalize(storage.path())
            .expect("canonical fixture root")
            .join("repo");
        fs::create_dir(&root).expect("fixture directory");
        let repo = Self { storage, root };
        repo.git(&[
            "init",
            "-q",
            "--template=",
            &format!("--object-format={format}"),
        ]);
        repo.git(&["config", "user.name", "Test user"]);
        repo.git(&["config", "user.email", "test@localhost"]);
        repo.git(&["config", "core.hooksPath", "/dev/null"]);
        repo
    }

    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("HOME", self.storage.path());
        cmd
    }

    pub fn git(&self, args: &[&str]) -> Output {
        let output = self
            .command("git")
            .args(args)
            .output()
            .expect("run fixture Git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    pub fn git_os(&self, args: &[OsString]) -> Output {
        let output = self
            .command("git")
            .args(args)
            .output()
            .expect("run fixture Git");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    pub fn git_input(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self
            .command("git")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fixture Git");
        child
            .stdin
            .take()
            .expect("Git stdin")
            .write_all(input)
            .expect("write Git input");
        let output = child.wait_with_output().expect("fixture Git output");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    pub fn write(&self, path: impl AsRef<Path>, bytes: &[u8]) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().expect("file parent")).expect("create parents");
        fs::write(path, bytes).expect("write fixture file");
    }

    pub fn commit_all(&self) {
        self.git(&["add", "."]);
        self.git(&["commit", "-qm", "Fixture"]);
    }

    pub fn run_command(&self) -> Command {
        self.command(env!("CARGO_BIN_EXE_git-commitfs"))
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_command()
            .args(args)
            .output()
            .expect("run git-commitfs")
    }

    pub fn source_state(&self) -> Vec<(PathBuf, Vec<u8>, u32, std::time::SystemTime)> {
        let mut result = Vec::new();
        collect_state(&self.root, &self.root, &mut result);
        result.sort();
        result
    }
}

fn collect_state(
    root: &Path,
    dir: &Path,
    result: &mut Vec<(PathBuf, Vec<u8>, u32, std::time::SystemTime)>,
) {
    for item in fs::read_dir(dir).expect("read fixture directory") {
        let path = item.expect("directory entry").path();
        let meta = fs::symlink_metadata(&path).expect("fixture metadata");
        if meta.is_dir() {
            collect_state(root, &path, result);
        } else {
            let content = if meta.is_symlink() {
                fs::read_link(&path)
                    .expect("symlink")
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else {
                fs::read(&path).expect("fixture content")
            };
            result.push((
                path.strip_prefix(root).expect("relative path").to_owned(),
                content,
                meta.permissions().mode(),
                meta.modified().expect("fixture mtime"),
            ));
        }
    }
}

// Tests that scrub PATH must keep a Git meeting the app's requirement
// reachable: the app discovers Git through the environment it is given, and
// system Git on some hosts (hosted macOS) is older than it supports.
pub fn git_dir() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .expect("fixture Git on PATH")
        .parent()
        .expect("Git with a parent directory")
        .to_owned()
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure(output: &Output, code: i32, message: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn inode(path: &Path) -> u64 {
    fs::metadata(path).expect("file metadata").ino()
}

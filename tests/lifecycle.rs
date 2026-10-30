mod common;

use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::PathBuf,
    process::Stdio,
};

use common::{Repo, assert_failure, assert_success};

fn kept(output: &std::process::Output) -> Option<PathBuf> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| {
            line.strip_prefix("git-commitfs: kept workspace at ")
                .map(PathBuf::from)
        })
}

fn cleanup(path: &std::path::Path) {
    fs::remove_dir_all(path.parent().expect("run root")).expect("remove retained run");
}

#[test]
fn retention_policy_covers_success_failure_and_failed_preparation() {
    for (policy, code, retain) in [
        ("never", 0, false),
        ("never", 17, false),
        ("on-failure", 0, false),
        ("on-failure", 17, true),
        ("always", 0, true),
        ("always", 17, true),
    ] {
        let repo = Repo::new();
        let output = repo.run(&[
            "run",
            &format!("--keep={policy}"),
            "--",
            "sh",
            "-c",
            &format!("pwd; exit {code}"),
        ]);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = PathBuf::from(
            String::from_utf8(output.stdout.clone())
                .expect("workspace path")
                .trim_end(),
        );
        assert_eq!(kept(&output).is_some(), retain);
        assert_eq!(path.exists(), retain);
        if retain {
            assert_eq!(kept(&output), Some(path.clone()));
            cleanup(&path);
        }
    }
    let repo = Repo::new();
    repo.write("file", b"intent only");
    repo.git(&["add", "-N", "file"]);
    let output = repo.run(&["run", "--keep=on-failure", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "intent-to-add");
    let path = kept(&output).expect("retained failed preparation");
    assert!(path.exists());
    assert_eq!(output.stdout, [] as [u8; 0]);
    cleanup(&path);
}

#[test]
fn retained_repository_survives_source_deletion_and_has_no_scratch() {
    let repo = Repo::new();
    repo.write("file", b"source independent");
    repo.git(&["add", "."]);
    let output = repo.run(&["run", "--keep=always", "--", "true"]);
    assert_success(&output);
    let path = kept(&output).expect("retained checkout");
    fs::remove_dir_all(&repo.root).expect("remove source");
    let output = repo
        .command("git")
        .current_dir(&path)
        .args(["show", "HEAD:file"])
        .output()
        .expect("Git in retained repo");
    assert_success(&output);
    assert_eq!(output.stdout, b"source independent");
    assert!(!path.parent().expect("run root").join("scratch").exists());
    cleanup(&path);
}

#[test]
fn statuses_and_command_errors_are_preserved() {
    let repo = Repo::new();
    for code in [0, 17, 125] {
        let output = repo.run(&["run", "--", "sh", "-c", &format!("exit {code}")]);
        assert_eq!(output.status.code(), Some(code));
    }
    assert_eq!(
        repo.run(&["run", "--", "sh", "-c", "kill -TERM $$"])
            .status
            .code(),
        Some(143)
    );
    assert_failure(
        &repo.run(&["run", "--", "git-commitfs-missing-executable-941251"]),
        127,
        "not found",
    );
    repo.write("not-executable", b"script");
    repo.git(&["add", "."]);
    assert_failure(
        &repo.run(&["run", "--", "./not-executable"]),
        126,
        "not executable",
    );
    fs::set_permissions(
        repo.root.join("not-executable"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("executable without format");
    repo.git(&["add", "."]);
    assert_failure(
        &repo.run(&["run", "--", "./not-executable"]),
        126,
        "not executable",
    );
    assert_failure(&repo.run(&["run", "--"]), 2, "required");
}

#[test]
fn argv_and_stdio_are_byte_exact_without_a_shell() {
    let repo = Repo::new();
    let argument = OsString::from_vec(b"raw ; $(not-a-shell)\n\xff".to_vec());
    let output = repo
        .run_command()
        .args(["run", "--", "printf", "%s"])
        .arg(&argument)
        .output()
        .expect("raw argv");
    assert_success(&output);
    assert_eq!(output.stdout, argument.as_encoded_bytes());
    let mut child = repo
        .run_command()
        .args(["run", "--", "sh", "-c", "cat; printf diagnostic >&2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("stdio command");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"stdin\0\xff")
        .expect("stdin bytes");
    let output = child.wait_with_output().expect("stdio result");
    assert_success(&output);
    assert_eq!(output.stdout, b"stdin\0\xff");
    assert_eq!(output.stderr, b"diagnostic");
}

#[test]
fn commands_always_start_at_snapshot_root_with_correct_pwd() {
    let repo = Repo::new();
    repo.write("sub/file", b"root cwd");
    repo.git(&["add", "."]);
    let output = repo
        .run_command()
        .current_dir(repo.root.join("sub"))
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "set -e; cat sub/file; test \"$(printenv PWD)\" = \"$(pwd)\"",
        ])
        .output()
        .expect("subdirectory invocation");
    assert_success(&output);
    assert_eq!(output.stdout, b"root cwd");
}

#[test]
fn concurrent_runs_never_share_workspaces() {
    let repo = Repo::new();
    let first = repo
        .run_command()
        .args(["run", "--keep=always", "--", "pwd"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first concurrent run");
    let second = repo
        .run_command()
        .args(["run", "--keep=always", "--", "pwd"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("second concurrent run");
    let first = first.wait_with_output().expect("first output");
    let second = second.wait_with_output().expect("second output");
    assert_success(&first);
    assert_success(&second);
    let first = kept(&first).expect("first retained");
    let second = kept(&second).expect("second retained");
    assert_ne!(first, second);
    fs::write(first.join("private"), b"one run").expect("first workspace mutation");
    assert!(!second.join("private").exists());
    cleanup(&first);
    cleanup(&second);
}

mod common;

use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
};

use common::{Repo, assert_failure, assert_success};

#[test]
fn staged_attributes_win_and_export_ignore_does_not_remove_files() {
    let repo = Repo::new();
    repo.write(".gitattributes", b"file text eol=lf\nfile export-ignore\n");
    repo.write("file", b"a\nb\n");
    repo.git(&["add", "."]);
    repo.write(".gitattributes", b"file text eol=crlf\n");
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"a\nb\n");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn local_checkout_settings_are_preserved() {
    let repo = Repo::new();
    repo.write("file", b"line\n");
    repo.git(&["add", "."]);
    repo.git(&["config", "core.autocrlf", "true"]);
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"line\r\n");
}

#[test]
fn info_attributes_are_preserved() {
    let repo = Repo::new();
    repo.write("file", b"line\n");
    repo.git(&["add", "."]);
    repo.write(".git/info/attributes", b"file text eol=crlf\n");
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"line\r\n");
}

#[test]
fn external_attributes_are_captured() {
    let repo = Repo::new();
    repo.write("file", b"line\n");
    repo.git(&["add", "."]);
    let attributes = repo.storage.path().join("external attributes");
    fs::write(&attributes, b"file text eol=crlf\n").expect("external attributes");
    repo.git_os(&[
        "config".into(),
        "core.attributesFile".into(),
        attributes.into_os_string(),
    ]);
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"line\r\n");
}

fn filtered_repo() -> Repo {
    let repo = Repo::new();
    repo.git(&[
        "config",
        "filter.demo.clean",
        "sed 's/WORKTREE/CANONICAL/g'",
    ]);
    repo.git(&[
        "config",
        "filter.demo.smudge",
        "sed 's/CANONICAL/WORKTREE/g'",
    ]);
    repo.git(&["config", "filter.demo.required", "true"]);
    repo.write(".gitattributes", b"file filter=demo\n");
    repo.write("file", b"WORKTREE\n");
    repo.git(&["add", "."]);
    repo
}

#[test]
fn required_smudge_clean_filters_run_and_the_baseline_is_clean() {
    let repo = filtered_repo();
    let before = repo.source_state();
    let output = repo.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "set -e; git diff --exit-code; test -z \"$(git status --porcelain)\"; cat file",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"WORKTREE\n");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn failed_required_filter_prevents_the_command() {
    let repo = filtered_repo();
    repo.git(&["config", "filter.demo.smudge", "exit 42"]);
    let output = repo.run(&["run", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "filter");
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn binary_empty_executable_and_symlink_entries_are_preserved() {
    let repo = Repo::new();
    repo.write("binary", b"\0\xff\r\n");
    repo.write("empty", b"");
    repo.write("executable", b"#!/bin/sh\nprintf executed\n");
    fs::set_permissions(
        repo.root.join("executable"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("executable mode");
    symlink("binary", repo.root.join("link")).expect("fixture symlink");
    repo.git(&["add", "."]);
    let output = repo.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "set -e; test -L link; test ! -s empty; ./executable; cat link",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"executed\0\xff\r\n");
}

#[test]
fn newline_paths_round_trip_without_trimming() {
    byte_paths_round_trip(b"repo with spaces\n", b"name\n");
}

#[test]
fn non_utf8_paths_round_trip_when_supported_by_the_filesystem() {
    byte_paths_round_trip(b"repo with spaces\n\xff\n", b"name\n\xff\n");
}

fn byte_paths_round_trip(root_name: &[u8], file_name: &[u8]) {
    let mut repo = Repo::new();
    let root = repo
        .storage
        .path()
        .join(OsString::from_vec(root_name.to_vec()));
    if let Err(error) = fs::rename(&repo.root, &root) {
        if error.raw_os_error() == Some(libc::EILSEQ) && !root_name.is_ascii() {
            eprintln!(
                "filesystem rejects non-UTF-8 names: {error}; byte argv coverage remains active"
            );
            return;
        }
        panic!("rename source root: {error}");
    }
    repo.root = root;
    let name = OsString::from_vec(file_name.to_vec());
    repo.write(&name, b"path content");
    repo.git(&["add", "."]);
    let output = repo
        .run_command()
        .args(["run", "--", "cat"])
        .arg(&name)
        .output()
        .expect("byte path run");
    assert_success(&output);
    assert_eq!(output.stdout, b"path content");
}

#[test]
fn prepared_git_objects_do_not_depend_on_the_source() {
    let repo = Repo::new();
    repo.write("file", b"independent");
    repo.git(&["add", "."]);
    let output = repo
        .run_command()
        .env("SOURCE", &repo.root)
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "set -e; mv \"$SOURCE/.git/objects\" \"$SOURCE/objects-away\"; git fsck \
             --no-progress; test \"$(git rev-list --count HEAD)\" = 1; git show HEAD:file",
        ])
        .output()
        .expect("independent object test");
    assert_success(&output);
    assert_eq!(output.stdout, b"independent");
}

#[test]
fn source_routing_is_not_inherited_by_command_git_operations() {
    let repo = Repo::new();
    repo.write("file", b"staged");
    repo.git(&["add", "."]);
    let before = repo.source_state();
    let output = repo
        .run_command()
        .env("GIT_DIR", repo.root.join(".git"))
        .env("GIT_INDEX_FILE", repo.root.join(".git/index"))
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "set -e; test -z \"$(git status --porcelain)\"; printf changed > file; git add file; \
             git -c user.name=Command -c user.email=command@localhost commit -qm private; git \
             show HEAD:file",
        ])
        .output()
        .expect("routed command");
    assert_success(&output);
    assert_eq!(output.stdout, b"changed");
    assert_eq!(repo.source_state(), before);
}

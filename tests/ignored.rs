mod common;

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use common::{Repo, assert_failure, assert_success};

fn ignored_repo() -> Repo {
    let repo = Repo::new();
    repo.write(".gitignore", b"target/\n*.tmp\n");
    repo.git(&["add", ".gitignore"]);
    repo.write("target/cache", b"original");
    repo
}

#[test]
fn ignored_files_are_excluded_by_default() {
    let repo = ignored_repo();
    let output = repo.run(&["run", "--", "sh", "-c", "test ! -e target && printf absent"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"absent");
}

#[test]
fn imported_files_and_build_writes_are_private() {
    let repo = ignored_repo();
    let before = repo.source_state();
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "set -e; cat target/cache; printf changed > target/cache; printf output > target/new; \
         test -z \"$(git status --porcelain)\"",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"original");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn staged_ignore_rules_win_over_unstaged_and_untracked_rules() {
    let repo = ignored_repo();
    repo.write(".gitignore", b"untracked\n");
    repo.write("sub/.gitignore", b"untracked\n");
    repo.write("sub/untracked", b"not eligible");
    repo.write("sub/cache.tmp", b"eligible");
    repo.write("untracked", b"not eligible");
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "set -e; test ! -e untracked; test ! -e sub/untracked; test ! -e sub/.gitignore; cat \
         sub/cache.tmp; cat target/cache",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"eligibleoriginal");
}

#[test]
fn nested_negation_and_local_global_excludes_are_respected() {
    let repo = ignored_repo();
    repo.write("sub/.gitignore", b"*.log\n!keep.log\n");
    repo.git(&["add", "sub/.gitignore"]);
    repo.write("sub/a.log", b"nested");
    repo.write("sub/keep.log", b"not ignored");
    repo.write(".git/info/exclude", b"local\n");
    repo.write("local", b"local");
    let global = repo.storage.path().join("global ignore");
    fs::write(&global, b"global\n").expect("global excludes");
    repo.git_os(&[
        "config".into(),
        "core.excludesFile".into(),
        global.into_os_string(),
    ]);
    repo.write("global", b"global");
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "set -e; test ! -e sub/keep.log; cat sub/a.log local global; test -z \"$(git status \
         --porcelain)\"",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"nestedlocalglobal");
}

#[test]
fn force_added_tracked_files_win_over_ignored_working_content() {
    let repo = ignored_repo();
    repo.write("tracked.tmp", b"staged");
    repo.git(&["add", "-f", "tracked.tmp"]);
    repo.write("tracked.tmp", b"unstaged");
    let output = repo.run(&["run", "--include-ignored", "--", "cat", "tracked.tmp"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"staged");
}

#[test]
fn staged_deletions_are_not_resurrected_as_ignored_files() {
    let repo = ignored_repo();
    repo.write("gone.tmp", b"tracked");
    repo.git(&["add", "-f", "gone.tmp"]);
    repo.git(&["commit", "-qm", "Tracked ignored file"]);
    repo.git(&["rm", "--cached", "gone.tmp"]);
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "test ! -e gone.tmp && printf absent",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"absent");
}

#[test]
fn nested_git_metadata_is_never_imported() {
    let repo = ignored_repo();
    repo.write("target/dependency/.git/config", b"private metadata");
    repo.write("target/dependency/file", b"dependency");
    repo.write("target/other/.git", b"gitdir: private\n");
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "set -e; test ! -e target/dependency/.git; test ! -e target/other/.git; cat \
         target/dependency/file",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"dependency");
}

#[test]
fn ignored_symlinks_are_preserved_without_traversal() {
    let repo = ignored_repo();
    let outside = repo.storage.path().join("outside");
    fs::create_dir(&outside).expect("external directory");
    fs::write(outside.join("file"), b"outside").expect("external file");
    symlink(&outside, repo.root.join("target/link")).expect("ignored symlink");
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--",
        "sh",
        "-c",
        "test -L target/link && readlink target/link",
    ]);
    assert_success(&output);
    let mut expected = outside.as_os_str().as_encoded_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(output.stdout, expected);
}

#[test]
fn tracked_symlink_ancestors_block_ignored_directory_imports() {
    let repo = ignored_repo();
    symlink("nowhere", repo.root.join("node.tmp")).expect("tracked symlink");
    repo.git(&["add", "-f", "node.tmp"]);
    fs::remove_file(repo.root.join("node.tmp")).expect("replace source symlink");
    repo.write("node.tmp/child", b"must not overwrite symlink");
    let output = repo.run(&["run", "--include-ignored", "--", "readlink", "node.tmp"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"nowhere\n");
}

#[test]
fn ignored_special_files_are_errors_but_unrelated_ones_are_not() {
    let repo = ignored_repo();
    assert!(
        repo.command("mkfifo")
            .arg("unrelated")
            .status()
            .expect("untracked FIFO")
            .success()
    );
    assert_success(&repo.run(&["run", "--include-ignored", "--", "true"]));
    assert!(
        repo.command("mkfifo")
            .arg("target/pipe")
            .status()
            .expect("ignored FIFO")
            .success()
    );
    let output = repo.run(&["run", "--include-ignored", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "unsupported");
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn unreadable_eligible_files_fail_before_execution() {
    let repo = ignored_repo();
    fs::set_permissions(
        repo.root.join("target/cache"),
        fs::Permissions::from_mode(0o0),
    )
    .expect("unreadable cache");
    if fs::File::open(repo.root.join("target/cache")).is_ok() {
        eprintln!("skipping unreadable-file assertion: this user can read mode-000 files");
        return;
    }
    let output = repo.run(&["run", "--include-ignored", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "target/cache");
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn read_only_imported_directories_are_cleaned_without_changing_source_modes() {
    let repo = ignored_repo();
    let directory = repo.root.join("target");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).expect("read-only cache");
    let output = repo.run(&["run", "--include-ignored", "--", "pwd"]);
    let source_mode = fs::metadata(&directory)
        .expect("source directory metadata")
        .permissions()
        .mode();
    // Restore fixture permissions even if the assertion fails.
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).expect("restore fixture");
    assert_success(&output);
    assert_eq!(source_mode & 0o777, 0o555);
    let path = String::from_utf8(output.stdout).expect("workspace path");
    assert!(!std::path::Path::new(path.trim_end()).exists());
}

#[test]
fn temporary_workspaces_cannot_be_allocated_inside_the_source() {
    let repo = ignored_repo();
    let before = repo.source_state();
    let output = repo
        .run_command()
        .env("TMPDIR", &repo.root)
        .args(["run", "--include-ignored", "--", "printf", "should not run"])
        .output()
        .expect("nested temporary directory");
    assert_failure(&output, 125, "TMPDIR");
    assert_eq!(repo.source_state(), before);
}

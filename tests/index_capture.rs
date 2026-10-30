mod common;

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::Stdio,
};

use common::{Repo, assert_failure, assert_success};

#[test]
fn alternate_index_wins_and_neither_index_changes() {
    let repo = Repo::new();
    repo.write("file", b"normal\n");
    repo.git(&["add", "."]);
    let alternate = repo.storage.path().join("alternate");
    let mut empty = repo.command("git");
    assert!(
        empty
            .args(["read-tree", "--empty"])
            .env("GIT_INDEX_FILE", &alternate)
            .status()
            .expect("alternate index")
            .success()
    );
    repo.write("file", b"alternate\n");
    assert!(
        repo.command("git")
            .args(["add", "file"])
            .env("GIT_INDEX_FILE", &alternate)
            .status()
            .expect("stage alternate")
            .success()
    );
    repo.write("file", b"working\n");
    let before = repo.source_state();
    let index = fs::read(&alternate).expect("alternate index bytes");
    let output = repo
        .run_command()
        .env("GIT_INDEX_FILE", &alternate)
        .args(["run", "--", "cat", "file"])
        .output()
        .expect("run alternate");
    assert_success(&output);
    assert_eq!(output.stdout, b"alternate\n");
    assert_eq!(fs::read(alternate).expect("alternate unchanged"), index);
    assert_eq!(repo.source_state(), before);
}

#[test]
fn split_index_is_read_without_refreshing_shared_source_data() {
    let repo = Repo::new();
    repo.write("file", b"staged");
    repo.git(&["add", "."]);
    repo.git(&["update-index", "--split-index"]);
    let shared: Vec<_> = fs::read_dir(repo.root.join(".git"))
        .expect("Git directory")
        .map(|entry| entry.expect("Git entry").path())
        .filter(|path| {
            path.file_name()
                .expect("filename")
                .as_encoded_bytes()
                .starts_with(b"sharedindex.")
        })
        .collect();
    let old_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    for path in &shared {
        fs::File::open(path)
            .expect("shared index")
            .set_times(fs::FileTimes::new().set_modified(old_time))
            .expect("set old shared-index mtime");
    }
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"staged");
    assert_eq!(repo.source_state(), before);
    for path in shared {
        assert_eq!(
            fs::metadata(path)
                .expect("shared metadata")
                .modified()
                .expect("mtime"),
            old_time
        );
    }
}

#[test]
fn linked_worktree_uses_its_own_index() {
    let repo = Repo::new();
    repo.write("file", b"base");
    repo.commit_all();
    let linked = repo.storage.path().join("linked");
    repo.git_os(&[
        "worktree".into(),
        "add".into(),
        "-qb".into(),
        "linked".into(),
        linked.as_os_str().to_owned(),
    ]);
    fs::write(linked.join("file"), b"linked").expect("linked file");
    assert!(
        repo.command("git")
            .current_dir(&linked)
            .args(["add", "file"])
            .status()
            .expect("linked stage")
            .success()
    );
    let output = repo
        .run_command()
        .current_dir(linked)
        .args(["run", "--", "cat", "file"])
        .output()
        .expect("linked run");
    assert_success(&output);
    assert_eq!(output.stdout, b"linked");
}

#[test]
fn sparse_index_still_materializes_the_entire_tree() {
    let repo = Repo::new();
    repo.write("visible/a", b"visible");
    repo.write("hidden/b", b"hidden");
    repo.commit_all();
    repo.git(&["sparse-checkout", "init", "--cone", "--sparse-index"]);
    repo.git(&["sparse-checkout", "set", "visible"]);
    assert!(!repo.root.join("hidden/b").exists());
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "cat", "hidden/b"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"hidden");
    assert_eq!(repo.source_state(), before);
    repo.write("hidden/b", b"unstaged outside the sparse cone");
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "cat", "hidden/b"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"hidden");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn sha256_object_format_is_preserved() {
    let repo = Repo::with_format("sha256");
    repo.write("file", b"sha256");
    repo.git(&["add", "."]);
    let output = repo.run(&["run", "--", "git", "rev-parse", "--show-object-format"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"sha256\n");
}

#[test]
fn absent_index_is_an_empty_candidate_even_with_head() {
    let repo = Repo::new();
    repo.write("file", b"head");
    repo.commit_all();
    fs::remove_file(repo.root.join(".git/index")).expect("remove index");
    let output = repo.run(&["run", "--", "git", "ls-files"]);
    assert_success(&output);
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn empty_unborn_repository_runs_commands() {
    let repo = Repo::new();
    let output = repo.run(&["run", "--", "printf", "empty"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"empty");
}

#[test]
fn intent_to_add_is_rejected_before_execution() {
    let repo = Repo::new();
    repo.write("new", b"not staged");
    repo.git(&["add", "-N", "new"]);
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "intent-to-add");
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert_eq!(repo.source_state(), before);
}

#[test]
fn conflicts_are_rejected_before_execution() {
    let repo = Repo::new();
    let oid = repo
        .git_input(&["hash-object", "-w", "--stdin"], b"content")
        .stdout;
    let oid = String::from_utf8(oid).expect("hex object");
    repo.git_input(
        &["update-index", "--index-info"],
        format!(
            "100644 {} 1\tconflict\n100644 {} 2\tconflict\n",
            oid.trim(),
            oid.trim()
        )
        .as_bytes(),
    );
    let output = repo.run(&["run", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "conflict");
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn submodules_are_explicitly_rejected() {
    let repo = Repo::new();
    repo.write("file", b"base");
    repo.commit_all();
    let head = String::from_utf8(repo.git(&["rev-parse", "HEAD"]).stdout).expect("head");
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{},module", head.trim()),
    ]);
    let output = repo.run(&["run", "--", "printf", "should not run"]);
    assert_failure(&output, 125, "submodule");
    assert_eq!(output.stdout, [] as [u8; 0]);
}

#[test]
fn existing_source_index_lock_is_not_touched() {
    let repo = Repo::new();
    repo.write("file", b"staged");
    repo.git(&["add", "."]);
    repo.write(".git/index.lock", b"another Git operation");
    let before = repo.source_state();
    assert_success(&repo.run(&["run", "--", "cat", "file"]));
    assert_eq!(repo.source_state(), before);
}

#[test]
fn later_staging_does_not_change_a_running_candidate() {
    let repo = Repo::new();
    repo.write("file", b"captured");
    repo.git(&["add", "."]);
    let mut child = repo
        .run_command()
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "printf 'ready\\n'; read -r go; cat file",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start command");
    let mut output = BufReader::new(child.stdout.take().expect("child stdout"));
    let mut line = String::new();
    output.read_line(&mut line).expect("ready marker");
    assert_eq!(line, "ready\n");
    repo.write("file", b"later");
    repo.git(&["add", "file"]);
    child
        .stdin
        .take()
        .expect("child input")
        .write_all(b"go\n")
        .expect("release child");
    let mut remaining = Vec::new();
    std::io::Read::read_to_end(&mut output, &mut remaining).expect("read result");
    assert!(child.wait().expect("child status").success());
    assert_eq!(remaining, b"captured");
}

#[test]
fn renames_type_changes_and_unstaged_missing_files_come_from_the_index() {
    let repo = Repo::new();
    repo.write("old", b"renamed");
    repo.write("kind", b"ordinary file");
    repo.write("missing", b"restored");
    repo.commit_all();
    repo.git(&["mv", "old", "new"]);
    fs::remove_file(repo.root.join("kind")).expect("old type");
    std::os::unix::fs::symlink("new", repo.root.join("kind")).expect("new symlink");
    repo.git(&["add", "kind"]);
    fs::remove_file(repo.root.join("missing")).expect("unstaged deletion");
    let before = repo.source_state();
    let output = repo.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "set -e; test ! -e old; test -L kind; cat kind missing; test -z \"$(git status \
         --porcelain)\"",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"renamedrestored");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn missing_objects_do_not_fetch_from_a_source_promisor_remote() {
    let repo = Repo::new();
    repo.write("file", b"missing locally");
    repo.git(&["add", "."]);
    let oid = String::from_utf8(repo.git(&["rev-parse", ":file"]).stdout).expect("object ID");
    let oid = oid.trim();
    fs::remove_file(
        repo.root
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..]),
    )
    .expect("remove blob");
    repo.git(&["config", "remote.origin.promisor", "true"]);
    repo.git(&["config", "extensions.partialClone", "origin"]);
    repo.git(&["config", "protocol.ext.allow", "always"]);
    let helper = repo.storage.path().join("fetch-helper");
    fs::write(&helper, b"#!/bin/sh\ntouch \"$FETCH_MARKER\"\nexit 1\n").expect("fetch helper");
    fs::set_permissions(&helper, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("executable fetch helper");
    repo.git(&[
        "config",
        "remote.origin.url",
        &format!("ext::{}", helper.display()),
    ]);
    let marker = repo.storage.path().join("fetch-marker");
    let before = repo.source_state();
    let output = repo
        .run_command()
        .env("FETCH_MARKER", &marker)
        .args(["run", "--", "printf", "must not run"])
        .output()
        .expect("missing object run");
    assert_failure(&output, 125, "Git");
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(
        !marker.exists(),
        "preparation invoked a source promisor remote"
    );
    assert_eq!(repo.source_state(), before);
}

#[test]
fn relative_environment_object_alternates_keep_their_source_meaning() {
    let repo = Repo::new();
    repo.write("file", b"from alternate objects");
    repo.git(&["add", "."]);
    let objects = repo.storage.path().join("alternate-objects");
    fs::rename(repo.root.join(".git/objects"), &objects).expect("move source objects");
    fs::create_dir(repo.root.join(".git/objects")).expect("empty source object store");
    fs::create_dir(repo.root.join("sub")).expect("source subdirectory");
    let mut source_git = repo.command("git");
    let output = source_git
        .current_dir(repo.root.join("sub"))
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", "../alternate-objects")
        .args(["show", ":file"])
        .output()
        .expect("Git with relative alternate");
    assert_success(&output);
    let before = repo.source_state();
    let output = repo
        .run_command()
        .current_dir(repo.root.join("sub"))
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", "../alternate-objects")
        .args(["run", "--", "cat", "file"])
        .output()
        .expect("relative alternate capture");
    assert_success(&output);
    assert_eq!(output.stdout, b"from alternate objects");
    assert_eq!(repo.source_state(), before);
}

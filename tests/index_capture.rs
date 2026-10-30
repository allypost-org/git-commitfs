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
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "cat", "file"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"staged");
    assert_eq!(repo.source_state(), before);
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

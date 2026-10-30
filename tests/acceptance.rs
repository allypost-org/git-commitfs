mod common;

use std::{fs, os::unix::fs::PermissionsExt};

use common::{Repo, assert_failure, assert_success};

fn install_hook(repo: &Repo) {
    let hooks = repo.root.join(".git/hooks");
    fs::create_dir_all(&hooks).expect("hooks directory");
    repo.git(&[
        "config",
        "core.hooksPath",
        hooks.to_str().expect("hook path"),
    ]);
    fs::write(
        hooks.join("pre-commit"),
        br#"#!/bin/sh
set -u
index=$(git rev-parse --path-format=absolute --git-path index)
cp "$index" "$INDEX_BEFORE"
status=0
"$BIN" run -- sh -c "$CHECK" > "$HOOK_OUTPUT" || status=$?
cmp "$index" "$INDEX_BEFORE" || exit 99
exit "$status"
"#,
    )
    .expect("hook script");
    fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755))
        .expect("executable hook");
}

fn commit(repo: &Repo, args: &[&str], check: &str) -> std::process::Output {
    repo.command("git")
        .args(args)
        .env("BIN", env!("CARGO_BIN_EXE_git-commitfs"))
        .env("CHECK", check)
        .env("HOOK_OUTPUT", repo.storage.path().join("hook-output"))
        .env(
            "INDEX_BEFORE",
            repo.storage.path().join("hook-index-before"),
        )
        .output()
        .expect("real Git commit")
}

#[test]
fn rejecting_a_real_commit_preserves_staged_entries_refs_and_working_files() {
    let repo = Repo::new();
    repo.write("file", b"base");
    repo.commit_all();
    repo.write("file", b"staged");
    repo.git(&["add", "file"]);
    repo.write("file", b"working");
    install_hook(&repo);
    let entries = repo.git(&["ls-files", "--stage", "-z"]).stdout;
    let head = repo.git(&["rev-parse", "HEAD"]).stdout;
    let output = commit(
        &repo,
        &["commit", "-qm", "Rejected"],
        "cat file; printf disposable > file; exit 17",
    );
    assert!(!output.status.success());
    assert_eq!(
        fs::read(repo.storage.path().join("hook-output")).expect("hook output"),
        b"staged"
    );
    assert_eq!(repo.git(&["ls-files", "--stage", "-z"]).stdout, entries);
    assert_eq!(repo.git(&["rev-parse", "HEAD"]).stdout, head);
    assert_eq!(
        fs::read(repo.root.join("file")).expect("working file"),
        b"working"
    );
}

#[test]
fn path_limited_commit_uses_gits_temporary_index() {
    let repo = Repo::new();
    repo.write("a", b"base-a");
    repo.write("b", b"base-b");
    repo.commit_all();
    repo.write("a", b"staged-a");
    repo.write("b", b"staged-b");
    repo.git(&["add", "."]);
    repo.write("a", b"working-a");
    install_hook(&repo);
    let output = commit(
        &repo,
        &["commit", "-qm", "Only a", "--only", "a"],
        "set -e; cat a b; test -z \"$(git status --porcelain)\"; printf disposable > a",
    );
    assert_success(&output);
    assert_eq!(
        fs::read(repo.storage.path().join("hook-output")).expect("hook output"),
        b"working-abase-b"
    );
    assert_eq!(repo.git(&["show", "HEAD:a"]).stdout, b"working-a");
    assert_eq!(repo.git(&["show", "HEAD:b"]).stdout, b"base-b");
    assert_eq!(repo.git(&["show", ":b"]).stdout, b"staged-b");
    assert_eq!(
        fs::read(repo.root.join("a")).expect("original a"),
        b"working-a"
    );
}

#[test]
fn help_describes_the_implemented_contract_and_old_git_fails_early() {
    let repo = Repo::new();
    let output = repo.run(&["--help"]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("absolute paths"));
    let output = repo.run(&["run", "--help"]);
    assert_success(&output);
    let help = String::from_utf8_lossy(&output.stdout);
    for text in [
        "root",
        "discard",
        "--include-ignored",
        "--copy-mode",
        "--keep",
    ] {
        assert!(help.contains(text), "missing help text {text}: {help}");
    }
    let shim = repo.storage.path().join("bin");
    fs::create_dir(&shim).expect("shim directory");
    fs::write(
        shim.join("git"),
        b"#!/bin/sh\nprintf 'git version 2.42.4\\n'\n",
    )
    .expect("old Git shim");
    fs::set_permissions(shim.join("git"), fs::Permissions::from_mode(0o755))
        .expect("executable shim");
    let output = repo
        .run_command()
        .env("PATH", shim)
        .args(["run", "--", "printf", "must not run"])
        .output()
        .expect("old Git invocation");
    assert_failure(
        &output,
        125,
        "Git >= 2.43 is required; found git version 2.42.4",
    );
    assert_eq!(output.stdout, [] as [u8; 0]);
}

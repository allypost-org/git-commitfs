mod common;

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use common::{Repo, assert_success};

struct Hook {
    repo: Repo,
    tools: PathBuf,
    state: PathBuf,
}

fn executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("fixture executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable mode");
}

impl Hook {
    fn new(installed: bool) -> Self {
        let repo = Repo::new();
        let tools = repo.storage.path().join("tools");
        let state = repo.storage.path().join("state");
        fs::create_dir(&tools).expect("tool directory");
        fs::create_dir(&state).expect("state directory");
        for name in [
            "bash", "sh", "git", "realpath", "mkdir", "cp", "rm", "mktemp", "cat",
        ] {
            let path = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
                .map(|directory| directory.join(name))
                .find(|path| path.is_file())
                .expect("system fixture tool");
            symlink(path, tools.join(name)).expect("system tool link");
        }
        if installed {
            symlink(
                env!("CARGO_BIN_EXE_git-commitfs"),
                tools.join("git-commitfs"),
            )
            .expect("installed CLI fixture");
        }
        // Only the external toolchain is substituted; Git, the CLI, both hook
        // scripts, file mutations, and cleanup all execute for real. This also
        // lets the suite run in the restricted image without mise or Rust.
        executable(&tools.join("mise"), b"#!/bin/sh\nset -eu\ntest \"$1\" = exec; shift\ntest \"$1\" = --; shift\nexec \"$@\"\n");
        executable(
            &tools.join("just"),
            br#"#!/bin/sh
set -eu
case "$1" in
check)
    if test -e .git/objects/info/alternates; then
        echo 'hook checkout still borrows source objects' >&2
        exit 88
    fi
    test -z "$(git status --porcelain)"
    if test "$EXPECTED" = absent; then test ! -e src/input.rs
    else test "$(cat src/input.rs)" = "$EXPECTED"; fi
    test ! -e local-only
    printf '%s' "$PWD" > "$STATE/workspace"
    printf '%s' "$CARGO_TARGET_DIR" > "$STATE/check-cache"
    exit "${CHECK_EXIT:-0}"
    ;;
fmt-dev)
    test "$CARGO_TARGET_DIR" != "$(cat "$STATE/check-cache")"
    case "$CARGO_TARGET_DIR" in "$PWD"/*) ;; *) exit 89;; esac
    printf '%s' "$CARGO_TARGET_DIR" > "$STATE/fix-cache"
    case "${FORMAT_MODE:-clean}" in
        tracked) printf changed > src/input.rs;;
        staged) printf changed > src/input.rs; git add src/input.rs;;
        index-only) printf changed > src/input.rs; git add src/input.rs; printf '%s' "$EXPECTED" > src/input.rs;;
        untracked) printf generated > unexpected;;
        clean) ;;
        *) exit 90;;
    esac
    ;;
*) exit 91;;
esac
"#,
        );
        executable(
            &tools.join("cargo"),
            br#"#!/bin/sh
set -eu
test "$1" = build; shift
target_dir=
target=
locked=false
while test "$#" -gt 0; do
    case "$1" in
        --target-dir) target_dir=$2; shift;;
        --target) target=$2; shift;;
        --bin) test "$2" = git-commitfs; shift;;
        --locked) locked=true;;
        *) exit 92;;
    esac
    shift
done
test -n "$target_dir"; test "$target" = x86_64-unknown-linux-musl
test "$locked" = true
mkdir -p "$target_dir/$target/debug"
cp "$REAL_BINARY" "$target_dir/$target/debug/git-commitfs"
printf built > "$STATE/bootstrap"
"#,
        );
        repo.write(
            "scripts/check-staged-tree",
            include_bytes!("../scripts/check-staged-tree"),
        );
        repo.write(
            "scripts/check-hook-workspace",
            include_bytes!("../scripts/check-hook-workspace"),
        );
        repo.write(".gitignore", b"/target\n");
        repo.write("src/input.rs", b"base");
        repo.commit_all();
        repo.write("src/input.rs", b"staged");
        repo.git(&["add", "src/input.rs"]);
        repo.write("src/input.rs", b"working");
        repo.write("local-only", b"untracked input");
        Self { repo, tools, state }
    }

    fn command(&self) -> Command {
        let mut command = self.repo.command("bash");
        command
            .arg(self.repo.root.join("scripts/check-staged-tree"))
            .env("PATH", &self.tools)
            .env("STATE", &self.state)
            .env("EXPECTED", "staged")
            .env("REAL_BINARY", env!("CARGO_BIN_EXE_git-commitfs"));
        command
    }

    fn run(&self) -> Output {
        self.command().output().expect("run staged hook")
    }
}

#[test]
fn installed_cli_checks_only_staged_content_without_bootstrapping() {
    let hook = Hook::new(true);
    let before = hook.repo.source_state();
    assert_success(&hook.run());
    assert_eq!(hook.repo.source_state(), before);
    assert!(!hook.state.join("bootstrap").exists());
    let path = PathBuf::from(fs::read_to_string(hook.state.join("workspace")).expect("workspace"));
    assert!(!path.exists(), "disposable hook checkout survived");
}

#[test]
fn hook_preserves_alternate_index_semantics() {
    let hook = Hook::new(true);
    let alternate = hook.repo.storage.path().join("alternate-index");
    fs::copy(hook.repo.root.join(".git/index"), &alternate).expect("alternate index");
    hook.repo.write("src/input.rs", b"alternate");
    assert!(
        hook.repo
            .command("git")
            .env("GIT_INDEX_FILE", &alternate)
            .args(["add", "src/input.rs"])
            .status()
            .expect("stage alternate")
            .success()
    );
    hook.repo.write("src/input.rs", b"working");
    let before = hook.repo.source_state();
    let index = fs::read(&alternate).expect("alternate bytes");
    let output = hook
        .command()
        .env("GIT_INDEX_FILE", &alternate)
        .env("EXPECTED", "alternate")
        .output()
        .expect("alternate hook");
    assert_success(&output);
    assert_eq!(fs::read(alternate).expect("alternate after run"), index);
    assert_eq!(hook.repo.source_state(), before);
}

#[test]
fn subdirectory_invocation_preserves_relative_git_routing() {
    let hook = Hook::new(true);
    let alternate = hook.repo.storage.path().join("alternate-index");
    fs::copy(hook.repo.root.join(".git/index"), &alternate).expect("alternate index");
    let subdirectory = hook.repo.root.join("sub");
    fs::create_dir(&subdirectory).expect("invocation directory");
    hook.repo.write("src/input.rs", b"alternate");
    assert!(
        hook.repo
            .command("git")
            .env("GIT_INDEX_FILE", &alternate)
            .args(["add", "src/input.rs"])
            .status()
            .expect("stage alternate")
            .success()
    );
    hook.repo.write("src/input.rs", b"working");
    let output = hook
        .command()
        .current_dir(subdirectory)
        .env("GIT_DIR", "../.git")
        .env("GIT_WORK_TREE", "..")
        .env("GIT_INDEX_FILE", "../alternate-index")
        .env("EXPECTED", "alternate")
        .output()
        .expect("relative alternate hook");
    assert_success(&output);
    assert!(
        hook.state.join("workspace").exists(),
        "hook skipped the effective index"
    );
}

#[test]
fn failed_checks_and_formatter_changes_reject_without_source_changes() {
    for (check_exit, mode, code, diagnostic) in [
        ("17", "clean", 17, ""),
        ("0", "tracked", 1, "changes staged files"),
        ("0", "staged", 1, "changes staged files"),
        ("0", "index-only", 1, "changes staged files"),
        ("0", "untracked", 1, "untracked files"),
    ] {
        let hook = Hook::new(true);
        let before = hook.repo.source_state();
        let output = hook
            .command()
            .env("CHECK_EXIT", check_exit)
            .env("FORMAT_MODE", mode)
            .output()
            .expect("rejecting hook");
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic));
        assert_eq!(hook.repo.source_state(), before);
        if code == 17 {
            assert!(!hook.state.join("fix-cache").exists());
        }
    }
}

#[test]
fn bootstrap_builds_a_cli_when_path_has_none() {
    let hook = Hook::new(false);
    assert_success(&hook.run());
    assert!(hook.state.join("bootstrap").exists());
    assert_eq!(
        fs::read(hook.repo.root.join("src/input.rs")).expect("working file"),
        b"working"
    );
}

#[test]
fn staged_deletions_trigger_checks_and_documentation_only_changes_skip_them() {
    let hook = Hook::new(true);
    hook.repo.git(&["rm", "--force", "src/input.rs"]);
    let output = hook
        .command()
        .env("EXPECTED", "absent")
        .output()
        .expect("deletion hook");
    assert_success(&output);
    assert!(hook.state.join("workspace").exists());

    let hook = Hook::new(false);
    hook.repo.git(&["reset", "--hard", "HEAD"]);
    hook.repo.write("README.md", b"documentation only");
    hook.repo.git(&["add", "README.md"]);
    assert_success(&hook.run());
    assert!(!hook.state.join("workspace").exists());
    assert!(!hook.state.join("bootstrap").exists());
}

#[test]
fn a_dangling_mise_shim_does_not_prevent_bootstrapping() {
    let hook = Hook::new(false);
    executable(&hook.tools.join("git-commitfs"), b"#!/bin/sh\nexit 127\n");
    assert_success(&hook.run());
    assert!(hook.state.join("bootstrap").exists());
}

#[test]
fn repository_paths_ending_in_newlines_are_preserved() {
    let mut hook = Hook::new(true);
    let renamed = hook.repo.storage.path().join("repo with spaces\n");
    fs::rename(&hook.repo.root, &renamed).expect("rename source");
    hook.repo.root = renamed;
    let before = hook.repo.source_state();
    assert_success(&hook.run());
    assert_eq!(hook.repo.source_state(), before);
}

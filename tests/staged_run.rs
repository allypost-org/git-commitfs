mod common;

use common::{Repo, assert_success};

#[test]
fn partially_staged_content_is_used_and_writes_are_discarded() {
    let repo = Repo::new();
    repo.write("version.txt", b"base\n");
    repo.commit_all();
    repo.write("version.txt", b"staged\n");
    repo.git(&["add", "version.txt"]);
    repo.write("version.txt", b"unstaged\n");
    repo.write("untracked", b"not committed");
    let before = repo.source_state();
    let output = repo.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "set -e; test ! -e untracked; cat version.txt; printf 'changed\\n' > version.txt; touch \
         generated",
    ]);
    assert_success(&output);
    assert_eq!(output.stdout, b"staged\n");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn staged_deletion_is_absent_even_if_working_file_remains() {
    let repo = Repo::new();
    repo.write("gone", b"base");
    repo.commit_all();
    repo.git(&["rm", "--cached", "gone"]);
    let before = repo.source_state();
    let output = repo.run(&["run", "--", "sh", "-c", "test ! -e gone && printf absent"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"absent");
    assert_eq!(repo.source_state(), before);
}

#[test]
fn private_git_is_clean_and_command_failure_passes_through() {
    let repo = Repo::new();
    repo.write("file", b"staged");
    repo.git(&["add", "."]);
    let before = repo.source_state();
    let output = repo.run(&[
        "run",
        "--",
        "sh",
        "-c",
        "git status --porcelain; git diff --exit-code; printf changed > file; exit 17",
    ]);
    assert_eq!(output.status.code(), Some(17));
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert_eq!(repo.source_state(), before);
}

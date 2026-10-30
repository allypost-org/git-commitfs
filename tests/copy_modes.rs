mod common;

use std::{
    fs::{self, File},
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
};

use common::{Repo, assert_failure, assert_success, inode};

fn repo() -> Repo {
    let repo = Repo::new();
    repo.write(".gitignore", b"cache\n");
    repo.git(&["add", ".gitignore"]);
    repo.write("cache", b"original");
    fs::set_permissions(repo.root.join("cache"), fs::Permissions::from_mode(0o640))
        .expect("cache mode");
    repo
}

#[test]
fn copy_and_auto_keep_writes_private_and_preserve_permissions() {
    for mode in ["copy", "auto"] {
        let repo = repo();
        let original_inode = inode(&repo.root.join("cache"));
        let output = repo.run(&[
            "run",
            "--include-ignored",
            &format!("--copy-mode={mode}"),
            "--",
            "sh",
            "-c",
            "set -e; test \"$(stat -c %a cache)\" = 640; stat -c %i cache; printf changed > cache",
        ]);
        assert_success(&output);
        let new_inode: u64 = String::from_utf8(output.stdout)
            .expect("inode output")
            .trim()
            .parse()
            .expect("inode number");
        assert_ne!(original_inode, new_inode);
        assert_eq!(
            fs::read(repo.root.join("cache")).expect("original cache"),
            b"original"
        );
    }
}

#[test]
fn required_reflink_agrees_with_actual_filesystem_capability() {
    let repo = repo();
    let input = File::open(repo.root.join("cache")).expect("probe input");
    let probe = File::create(repo.storage.path().join("clone-probe")).expect("probe output");
    // SAFETY: Both descriptors refer to live regular files and FICLONE takes a source descriptor.
    let supported =
        unsafe { libc::ioctl(probe.as_raw_fd(), libc::FICLONE, input.as_raw_fd()) } == 0;
    let output = repo.run(&[
        "run",
        "--include-ignored",
        "--copy-mode=reflink",
        "--",
        "cat",
        "cache",
    ]);
    if supported {
        assert_success(&output);
        assert_eq!(output.stdout, b"original");
    } else {
        assert_failure(&output, 125, "reflink");
        assert_eq!(output.stdout, [] as [u8; 0]);
        eprintln!("reflink success path unavailable on this filesystem; strict failure verified");
    }
}

#[test]
fn copy_mode_without_imports_is_a_noop_and_bad_modes_are_usage_errors() {
    let repo = repo();
    let output = repo.run(&["run", "--copy-mode=reflink", "--", "printf", "no imports"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"no imports");
    assert_failure(
        &repo.run(&["run", "--copy-mode=unknown", "--", "true"]),
        2,
        "invalid value",
    );
}

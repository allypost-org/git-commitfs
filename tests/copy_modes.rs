mod common;

use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
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
            "--keep=always",
            "--include-ignored",
            &format!("--copy-mode={mode}"),
            "--",
            "sh",
            "-c",
            "set -e; printf changed > cache; pwd",
        ]);
        assert_success(&output);
        let path = std::path::PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("workspace path")
                .trim_end(),
        );
        let metadata = fs::metadata(path.join("cache")).expect("copied cache");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
        assert_ne!(original_inode, inode(&path.join("cache")));
        fs::remove_dir_all(path.parent().expect("run root")).expect("remove retained run");
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
    let probe_path = repo.storage.path().join("clone-probe");
    #[cfg(target_os = "linux")]
    let supported = {
        let probe = File::create(&probe_path).expect("probe output");
        rustix::fs::ioctl_ficlone(&probe, &input).is_ok()
    };
    #[cfg(target_os = "macos")]
    let supported = rustix::fs::fclonefileat(
        &input,
        rustix::fs::CWD,
        &probe_path,
        rustix::fs::CloneFlags::empty(),
    )
    .is_ok();
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

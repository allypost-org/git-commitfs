use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

use crate::runner::RunStatus;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum KeepPolicy {
    #[default]
    Never,
    OnFailure,
    Always,
}

pub struct Finalization {
    pub retained: Option<PathBuf>,
    pub cleanup_error: Option<anyhow::Error>,
}

pub struct RunDirectory {
    storage: tempfile::TempDir,
    checkout: PathBuf,
    scratch: PathBuf,
}

impl RunDirectory {
    pub fn allocate(source_root: &Path) -> Result<Self> {
        let storage = tempfile::Builder::new()
            .prefix("git-commitfs-")
            .tempdir()
            .context("allocate temporary workspace")?;
        ensure!(
            !fs::canonicalize(storage.path())?.starts_with(fs::canonicalize(source_root)?),
            "temporary directory is inside the source checkout; set TMPDIR outside the repository"
        );
        let checkout = storage.path().join("worktree");
        let scratch = storage.path().join("scratch");
        fs::create_dir(&checkout)?;
        fs::create_dir(&scratch)?;
        Ok(Self {
            storage,
            checkout,
            scratch,
        })
    }

    pub fn checkout(&self) -> &Path {
        &self.checkout
    }
    pub fn scratch(&self) -> &Path {
        &self.scratch
    }

    pub fn clear_scratch(&self) -> Result<()> {
        if self.scratch.exists() {
            remove_owned_tree(&self.scratch).context("remove capture scratch data")?;
        }
        Ok(())
    }

    pub fn finalize(self, keep: KeepPolicy, result: &Result<RunStatus>) -> Finalization {
        let failed = result
            .as_ref()
            .map_or(true, |status| status.shell_code() != 0);
        let retain =
            matches!(keep, KeepPolicy::Always) || matches!(keep, KeepPolicy::OnFailure) && failed;
        let checkout = self.checkout.clone();
        let cleared = self.clear_scratch();
        let root = self.storage.keep();
        if retain {
            Finalization {
                retained: Some(checkout),
                cleanup_error: cleared.err(),
            }
        } else {
            match remove_owned_tree(&root) {
                Ok(()) => Finalization {
                    retained: None,
                    cleanup_error: None,
                },
                Err(error) => Finalization {
                    retained: Some(checkout),
                    cleanup_error: Some(
                        anyhow::Error::new(error)
                            .context(format!("remove disposable workspace {}", root.display())),
                    ),
                },
            }
        }
    }
}

fn remove_owned_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // Imported directories and command output may be read-only. Only relax owned
            // disposable directories, never files or symlink targets, before retrying.
            make_directories_removable(path)?;
            fs::remove_dir_all(path)
        }
        result => result,
    }
}

fn make_directories_removable(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )?;
        for entry in fs::read_dir(path)? {
            make_directories_removable(&entry?.path())?;
        }
    }
    Ok(())
}

use std::{
    fs,
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
            fs::remove_dir_all(&self.scratch).context("remove capture scratch data")?;
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
            match fs::remove_dir_all(&root) {
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

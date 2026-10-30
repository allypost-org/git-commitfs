use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

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

    pub fn close(self) -> Result<()> {
        self.storage.close().context("remove disposable workspace")
    }
}

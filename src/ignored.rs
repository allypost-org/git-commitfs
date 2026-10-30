use std::{
    collections::HashSet,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    copy,
    snapshot::{Candidate, SourceRepository},
    workspace::RunDirectory,
};

struct Entry {
    path: PathBuf,
    metadata: fs::Metadata,
}

pub fn import(
    source: &SourceRepository,
    candidate: &Candidate,
    run: &RunDirectory,
    mode: copy::CopyMode,
) -> Result<()> {
    let mut entries = Vec::new();
    enumerate(&source.root, Path::new(""), candidate, &mut entries)?;
    if entries.is_empty() {
        return Ok(());
    }
    let mut input = Vec::new();
    for entry in &entries {
        input.extend_from_slice(entry.path.as_os_str().as_encoded_bytes());
        if entry.metadata.is_dir() {
            input.push(b'/');
        }
        input.push(0);
    }
    let output = candidate
        .git
        .input_output(&["check-ignore", "--no-index", "--stdin", "-z"], input)?;
    ensure!(
        matches!(output.status.code(), Some(0 | 1)),
        "Git ignore evaluation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selected: HashSet<PathBuf> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let path = p.strip_suffix(b"/").unwrap_or(p);
            PathBuf::from(std::ffi::OsString::from_vec(path.to_vec()))
        })
        .collect();
    let mut directory_modes = Vec::new();
    for entry in entries.iter().filter(|e| selected.contains(&e.path)) {
        candidate.git.supervisor.check()?;
        let destination = run.checkout().join(&entry.path);
        ensure_safe_parents(run.checkout(), &entry.path)?;
        if entry.metadata.is_dir() {
            if !destination.exists() {
                fs::create_dir_all(&destination)?;
                directory_modes.push((destination, entry.metadata.permissions()));
            }
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_entry(
                &source.root.join(&entry.path),
                &destination,
                &entry.metadata,
                mode,
            )?;
        }
    }
    for (path, permissions) in directory_modes.into_iter().rev() {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn enumerate(
    root: &Path,
    relative: &Path,
    candidate: &Candidate,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    for item in fs::read_dir(root.join(relative))
        .with_context(|| format!("read source directory {}", root.join(relative).display()))?
    {
        let item = item?;
        candidate.git.supervisor.check()?;
        if item.file_name() == ".git" {
            continue;
        }
        let path = relative.join(item.file_name());
        if blocked(&path, candidate) {
            continue;
        }
        let metadata = fs::symlink_metadata(item.path())
            .with_context(|| format!("inspect ignored candidate {}", path.display()))?;
        let directory = metadata.is_dir();
        entries.push(Entry {
            path: path.clone(),
            metadata,
        });
        if directory {
            enumerate(root, &path, candidate, entries)?;
        }
    }
    Ok(())
}

fn blocked(path: &Path, candidate: &Candidate) -> bool {
    path.ancestors().any(|ancestor| {
        candidate.tracked.contains(ancestor)
            || (candidate.head_tracked.contains(ancestor)
                && !candidate
                    .tracked
                    .iter()
                    .any(|tracked| tracked.starts_with(ancestor)))
    })
}

fn ensure_safe_parents(root: &Path, relative: &Path) -> Result<()> {
    for parent in relative.ancestors().skip(1) {
        match fs::symlink_metadata(root.join(parent)) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.is_symlink(),
                "ignored file would traverse a tracked symlink or file: {}",
                relative.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn copy_entry(
    source: &Path,
    destination: &Path,
    metadata: &fs::Metadata,
    mode: copy::CopyMode,
) -> Result<()> {
    if metadata.is_symlink() {
        symlink(fs::read_link(source)?, destination)
            .with_context(|| format!("copy ignored symlink {}", source.display()))?;
    } else if metadata.is_file() {
        copy::regular_file(source, destination, mode)?;
    } else {
        bail!("unsupported ignored file type: {}", source.display());
    }
    Ok(())
}

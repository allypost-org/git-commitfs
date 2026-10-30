use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
};

use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum CopyMode {
    #[default]
    Auto,
    Copy,
    Reflink,
}

pub fn regular_file(source: &Path, destination: &Path, mode: CopyMode) -> Result<()> {
    regular_file_with(source, destination, mode, reflink)
}

fn regular_file_with(
    source: &Path,
    destination: &Path,
    mode: CopyMode,
    clone_file: impl FnOnce(&File, &File) -> io::Result<()>,
) -> Result<()> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .with_context(|| format!("open ignored file {}", source.display()))?;
    let metadata = input.metadata()?;
    ensure!(
        metadata.is_file(),
        "unsupported ignored file {}",
        source.display()
    );
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create private file {}", destination.display()))?;
    let result = (|| {
        copy_with(&mut input, &mut output, mode, clone_file).with_context(|| {
            format!(
                "copy ignored file {} using {mode:?} (reflink requires filesystem support)",
                source.display()
            )
        })?;
        output.set_permissions(metadata.permissions())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

fn copy_with(
    input: &mut File,
    output: &mut File,
    mode: CopyMode,
    clone_file: impl FnOnce(&File, &File) -> io::Result<()>,
) -> io::Result<()> {
    if !matches!(mode, CopyMode::Copy) {
        match clone_file(input, output) {
            Ok(()) => return Ok(()),
            Err(error) if matches!(mode, CopyMode::Auto) && unsupported(&error) => {
                output.set_len(0)?;
                output.seek(SeekFrom::Start(0))?;
                input.seek(SeekFrom::Start(0))?;
            }
            Err(error) => return Err(error),
        }
    }
    io::copy(input, output)?;
    Ok(())
}

fn unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EOPNOTSUPP | libc::EXDEV | libc::ENOTTY | libc::EINVAL | libc::ENOSYS)
    )
}

fn reflink(input: &File, output: &File) -> io::Result<()> {
    // SAFETY: Both descriptors are live regular files; FICLONE's third argument is a descriptor.
    if unsafe { libc::ioctl(output.as_raw_fd(), libc::FICLONE, input.as_raw_fd()) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn a_successful_clone_is_not_followed_by_a_copy() {
        let dir = tempfile::tempdir().expect("copy fixture");
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        fs::write(&source, b"original").expect("source");
        regular_file_with(
            &source,
            &destination,
            CopyMode::Auto,
            |mut input, mut output| {
                io::copy(&mut input, &mut output)?;
                // Changing the input after the injected clone exposes an accidental second copy.
                fs::write(&source, b"changed")?;
                Ok(())
            },
        )
        .expect("successful clone policy");
        assert_eq!(fs::read(destination).expect("clone result"), b"original");
    }

    #[test]
    fn partial_clone_is_truncated_before_fallback() {
        let dir = tempfile::tempdir().expect("copy fixture");
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        fs::write(&source, b"original").expect("source");
        regular_file_with(&source, &destination, CopyMode::Auto, |_, mut output| {
            output.write_all(b"a long partial clone")?;
            Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP))
        })
        .expect("copy fallback");
        assert_eq!(fs::read(destination).expect("copied file"), b"original");
    }

    #[test]
    fn operational_errors_and_strict_clone_failures_are_not_hidden() {
        for (mode, errno) in [
            (CopyMode::Auto, libc::EACCES),
            (CopyMode::Auto, libc::EIO),
            (CopyMode::Auto, libc::ENOSPC),
            (CopyMode::Reflink, libc::EOPNOTSUPP),
        ] {
            let dir = tempfile::tempdir().expect("copy fixture");
            let source = dir.path().join("source");
            let destination = dir.path().join("destination");
            fs::write(&source, b"original").expect("source");
            let result = regular_file_with(&source, &destination, mode, |_, mut output| {
                output.write_all(b"partial")?;
                Err(io::Error::from_raw_os_error(errno))
            });
            assert!(result.is_err());
            assert!(!destination.exists());
            assert_eq!(fs::read(source).expect("original"), b"original");
        }
    }

    #[test]
    fn cross_device_clones_fall_back_but_copy_mode_never_attempts_clone() {
        for mode in [CopyMode::Auto, CopyMode::Copy] {
            let dir = tempfile::tempdir().expect("copy fixture");
            let source = dir.path().join("source");
            let destination = dir.path().join("destination");
            fs::write(&source, b"original").expect("source");
            regular_file_with(&source, &destination, mode, |_, _| {
                assert!(matches!(mode, CopyMode::Auto));
                Err(io::Error::from_raw_os_error(libc::EXDEV))
            })
            .expect("independent copy");
            assert_eq!(fs::read(destination).expect("copied file"), b"original");
        }
    }
}

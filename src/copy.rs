use std::{
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result, ensure};

use crate::platform::fs::{clone_file, clone_unsupported as unsupported, open_source};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum CopyMode {
    #[default]
    Auto,
    Copy,
    Reflink,
}

pub fn regular_file(source: &Path, destination: &Path, mode: CopyMode) -> Result<()> {
    regular_file_with(source, destination, mode, clone_file)
}

fn regular_file_with(
    source: &Path,
    destination: &Path,
    mode: CopyMode,
    clone_file: impl FnOnce(&File, &mut File, &Path) -> io::Result<()>,
) -> Result<()> {
    let mut input =
        open_source(source).with_context(|| format!("open ignored file {}", source.display()))?;
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
        copy_with(&mut input, &mut output, destination, mode, clone_file).with_context(|| {
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
    destination: &Path,
    mode: CopyMode,
    clone_file: impl FnOnce(&File, &mut File, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if !matches!(mode, CopyMode::Copy) {
        match clone_file(input, output, destination) {
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
            |mut input, output, _| {
                io::copy(&mut input, output)?;
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
        regular_file_with(&source, &destination, CopyMode::Auto, |_, output, _| {
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
            let result = regular_file_with(&source, &destination, mode, |_, output, _| {
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
            regular_file_with(&source, &destination, mode, |_, _, _| {
                assert!(matches!(mode, CopyMode::Auto));
                Err(io::Error::from_raw_os_error(libc::EXDEV))
            })
            .expect("independent copy");
            assert_eq!(fs::read(destination).expect("copied file"), b"original");
        }
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    #[test]
    fn an_existing_destination_or_symlink_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("copy fixture");
        let source = dir.path().join("source");
        let existing = dir.path().join("existing");
        let link = dir.path().join("link");
        fs::write(&source, b"source").expect("source");
        fs::write(&existing, b"keep").expect("existing destination");
        std::os::unix::fs::symlink(&existing, &link).expect("destination symlink");
        for destination in [&existing, &link] {
            for mode in [CopyMode::Auto, CopyMode::Copy, CopyMode::Reflink] {
                assert!(regular_file(&source, destination, mode).is_err());
                assert_eq!(fs::read(&existing).expect("existing content"), b"keep");
                assert!(
                    fs::symlink_metadata(&link)
                        .expect("link metadata")
                        .is_symlink()
                );
            }
        }
    }

    #[test]
    fn cloning_uses_the_open_handle_after_source_path_replacement() {
        let dir = tempfile::tempdir().expect("copy fixture");
        let source = dir.path().join("source");
        let moved = dir.path().join("moved");
        let destination = dir.path().join("destination");
        fs::write(&source, b"original").expect("original source");
        let input = crate::platform::fs::open_source(&source).expect("source handle");
        fs::rename(&source, &moved).expect("move source pathname");
        fs::write(&source, b"replacement").expect("replacement source");
        let mut output = File::create_new(&destination).expect("private destination");
        match crate::platform::fs::clone_file(&input, &mut output, &destination) {
            Ok(()) => assert_eq!(fs::read(&destination).expect("clone"), b"original"),
            Err(error) if unsupported(&error) => {
                eprintln!(
                    "actual clone unavailable: {error}; handle-stability path requires clone \
                     support"
                );
            }
            Err(error) => panic!("clone failed operationally: {error}"),
        }
        assert_eq!(fs::read(&moved).expect("original source"), b"original");
        assert_eq!(
            fs::read(&source).expect("replacement source"),
            b"replacement"
        );
    }
}

#[cfg(test)]
mod unsupported_tests {
    use super::*;

    #[test]
    fn both_not_supported_errors_allow_auto_fallback() {
        for errno in [libc::ENOTSUP, libc::EOPNOTSUPP] {
            let dir = tempfile::tempdir().expect("copy fixture");
            let source = dir.path().join("source");
            let destination = dir.path().join("destination");
            fs::write(&source, b"fallback").expect("source");
            regular_file_with(&source, &destination, CopyMode::Auto, |_, _, _| {
                Err(io::Error::from_raw_os_error(errno))
            })
            .expect("unsupported clone fallback");
            assert_eq!(fs::read(destination).expect("copied file"), b"fallback");
        }
    }
}

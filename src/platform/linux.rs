use std::{fs::File, io, path::Path, time::Duration};

#[allow(
    clippy::unnecessary_wraps,
    reason = "same fallible interface as macOS command preparation"
)]
pub const fn prepare_spawn(_command: &mut std::process::Command) -> io::Result<()> {
    Ok(())
}

pub const COMPLETION_INTERVAL: Option<Duration> = None;

pub fn enable_supervision() -> io::Result<()> {
    // The rustix API encodes this Boolean prctl attribute as a nonzero Pid.
    rustix::process::set_child_subreaper(Some(rustix::process::Pid::INIT))?;
    Ok(())
}

pub fn clone_file(input: &File, output: &mut File, _destination: &Path) -> io::Result<()> {
    rustix::fs::ioctl_ficlone(output, input)?;
    Ok(())
}

pub fn group_exists(group: rustix::process::Pid) -> io::Result<bool> {
    match rustix::process::test_kill_process_group(group) {
        Ok(()) | Err(rustix::io::Errno::PERM) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn signal_denied_is_exit(group: rustix::process::Pid) -> io::Result<bool> {
    group_exists(group).map(|exists| !exists)
}

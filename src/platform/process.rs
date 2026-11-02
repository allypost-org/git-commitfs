use std::{
    io,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use rustix::{
    io::Errno,
    process::{Pid, Signal, WaitOptions},
};

pub use super::terminal::TerminalGuard;

#[allow(
    clippy::missing_const_for_fn,
    reason = "Linux initialization performs a syscall"
)]
pub fn enable_supervision() -> io::Result<()> {
    super::sys::enable_supervision()
}

pub const fn completion_interval() -> Option<Duration> {
    super::sys::COMPLETION_INTERVAL
}

pub fn termination_signal(status: ExitStatus) -> i32 {
    status.signal().unwrap_or(0)
}

pub fn not_executable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(libc::ENOEXEC)
}

#[derive(Clone, Copy)]
pub struct ProcessGroup(Pid);

impl ProcessGroup {
    pub fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        super::sys::prepare_spawn(command)?;
        let child = command.process_group(0).spawn()?;
        let group = Self(Pid::from_child(&child));
        Ok((child, group))
    }

    pub fn foreground(self) -> anyhow::Result<TerminalGuard> {
        TerminalGuard::give_to(self.0.as_raw_pid())
    }

    pub fn signal(self, signal: i32) -> io::Result<()> {
        let signal = match signal {
            libc::SIGINT => Signal::INT,
            libc::SIGTERM => Signal::TERM,
            libc::SIGHUP => Signal::HUP,
            libc::SIGKILL => Signal::KILL,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported termination signal",
                ));
            }
        };
        for attempt in 0..2 {
            match rustix::process::kill_process_group(self.0, signal) {
                Ok(()) | Err(Errno::SRCH) => return Ok(()),
                Err(Errno::PERM) if super::sys::signal_denied_is_exit(self.0)? => return Ok(()),
                // A newly forked member may now be signalable; retry a live
                // group once before treating the denial as operational.
                Err(Errno::PERM) if attempt == 0 => (),
                Err(error) => return Err(error.into()),
            }
        }
        Err(Errno::PERM.into())
    }

    pub fn exists(self) -> io::Result<bool> {
        super::sys::group_exists(self.0)
    }

    // Only call after the main child has been reaped. Linux additionally adopts
    // orphaned group members; on macOS their reaping belongs to the OS.
    pub fn reap(self) -> io::Result<()> {
        loop {
            match rustix::process::waitpgid(self.0, WaitOptions::NOHANG) {
                Ok(Some(_)) | Err(Errno::INTR) => (),
                Ok(None) | Err(Errno::CHILD) => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use rustix::process::{WaitId, WaitIdOptions, waitid};

    use super::*;

    #[test]
    fn an_exited_group_waiting_for_reaping_does_not_fail_signaling_or_block_cleanup() {
        let (mut child, group) =
            ProcessGroup::spawn(&mut Command::new("/usr/bin/true")).expect("fixture process");
        waitid(
            WaitId::Pid(group.0),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        )
        .expect("observe exit without reaping");
        let signaled = group.signal(libc::SIGTERM);
        let exists = group.exists();
        child.wait().expect("reap fixture");
        signaled.expect("signaling an exited group is harmless");
        assert!(
            !exists.expect("group state"),
            "zombies cannot continue workspace writes"
        );
    }
}

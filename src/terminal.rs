use std::io;

use anyhow::{Context, Result};

#[derive(Default)]
pub struct TerminalGuard {
    original: Option<i32>,
}

impl TerminalGuard {
    pub fn give_to(group: i32) -> Result<Self> {
        // SAFETY: These calls inspect the inherited stdin descriptor and current process group.
        let original = unsafe {
            if libc::isatty(libc::STDIN_FILENO) != 1 {
                return Ok(Self::default());
            }
            let foreground = libc::tcgetpgrp(libc::STDIN_FILENO);
            if foreground != libc::getpgrp() {
                return Ok(Self::default());
            }
            foreground
        };
        match set_foreground(group) {
            Ok(()) => {
                // SAFETY: The target is the managed child's group; resume any initial background read stop.
                unsafe {
                    libc::kill(-group, libc::SIGCONT);
                }
                Ok(Self {
                    original: Some(original),
                })
            }
            Err(error) if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOTTY)) => {
                Ok(Self::default())
            }
            Err(error) => Err(error).context("give terminal to command"),
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Some(group) = self.original {
            let _ = set_foreground(group);
        }
    }
}

fn set_foreground(group: i32) -> io::Result<()> {
    // SAFETY: sigset_t is initialized before use, the mask applies only to this thread, and stdin is a live terminal.
    unsafe {
        let mut blocked = std::mem::zeroed();
        let mut previous = std::mem::zeroed();
        libc::sigemptyset(&raw mut blocked);
        libc::sigaddset(&raw mut blocked, libc::SIGTTOU);
        let error = libc::pthread_sigmask(libc::SIG_BLOCK, &raw const blocked, &raw mut previous);
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        let result = libc::tcsetpgrp(libc::STDIN_FILENO, group);
        let error = if result == -1 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        libc::pthread_sigmask(libc::SIG_SETMASK, &raw const previous, std::ptr::null_mut());
        error.map_or(Ok(()), Err)
    }
}

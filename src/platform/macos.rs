use std::{
    fs::{self, File},
    io,
    path::Path,
    time::Duration,
};

use rustix::fs::{CWD, CloneFlags, fclonefileat};

// Orphaned descendants belong to launchd and do not send us SIGCHLD. Only
// group draining needs periodic observation; event waits still sleep normally.
pub fn prepare_spawn(command: &mut std::process::Command) -> io::Result<()> {
    super::exec::prepare(command)
}

pub const COMPLETION_INTERVAL: Option<Duration> = Some(Duration::from_millis(20));

#[allow(
    clippy::unnecessary_wraps,
    reason = "same fallible interface as Linux subreaper initialization"
)]
pub const fn enable_supervision() -> io::Result<()> {
    Ok(())
}

pub fn clone_file(input: &File, output: &mut File, destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("clone destination has no parent"))?;
    // Apple requires a nonexistent destination. Clone from the validated handle
    // into a unique sibling, then replace only the destination we already own.
    let cloned = tempfile::Builder::new()
        .prefix(".git-commitfs-clone-")
        .make_in(parent, |path| {
            fclonefileat(input, CWD, path, CloneFlags::NOOWNERCOPY)?;
            File::open(path).inspect_err(|_| {
                let _ = fs::remove_file(path);
            })
        })?;
    *output = cloned.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

// sys/proc_info.h: a process that has committed to exit no longer accepts
// signals, but can remain visible until teardown finishes.
const PROC_FLAG_INEXIT: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum GroupState {
    Gone,
    Exiting,
    Running,
}

const fn member_state(status: u32, flags: u32) -> GroupState {
    if status == libc::SZOMB {
        GroupState::Gone
    } else if flags & PROC_FLAG_INEXIT != 0 {
        GroupState::Exiting
    } else {
        GroupState::Running
    }
}

pub fn group_exists(group: rustix::process::Pid) -> io::Result<bool> {
    // A successful kernel probe is positive evidence of liveness. Never
    // discard it in favor of a PID snapshot whose members may already exit.
    match rustix::process::test_kill_process_group(group) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(rustix::io::Errno::PERM) => Ok(group_state(group)? != GroupState::Gone),
        Err(error) => Err(error.into()),
    }
}

pub fn signal_denied_is_exit(group: rustix::process::Pid) -> io::Result<bool> {
    Ok(group_state(group)? != GroupState::Running)
}

fn group_state(group: rustix::process::Pid) -> io::Result<GroupState> {
    use libproc::processes::{ProcFilter, pids_by_type};
    use rustix::{io::Errno, process::test_kill_process_group};
    match test_kill_process_group(group) {
        Err(Errno::SRCH) => return Ok(GroupState::Gone),
        Ok(()) | Err(Errno::PERM) => (),
        Err(error) => return Err(error.into()),
    }
    inspect_group(
        || {
            // libproc uses errno to distinguish an empty list from failure.
            errno::set_errno(errno::Errno(0));
            pids_by_type(ProcFilter::ByProgramGroup {
                pgrpid: group.as_raw_nonzero().get().cast_unsigned(),
            })
        },
        |pid| {
            let raw = i32::try_from(pid).map_err(io::Error::other)?;
            Ok(process_info(raw)?.map_or(GroupState::Gone, |info| {
                member_state(info.pbi_status, info.pbi_flags)
            }))
        },
    )
}

fn inspect_group(
    mut members: impl FnMut() -> io::Result<Vec<u32>>,
    mut member: impl FnMut(u32) -> io::Result<GroupState>,
) -> io::Result<GroupState> {
    let snapshot = members()?;
    let mut state = GroupState::Gone;
    for &pid in &snapshot {
        state = state.max(member(pid)?);
    }
    if state != GroupState::Running {
        // A member may fork before exiting while we inspect it. Any newly
        // observed PID keeps the group live until a later, complete inspection.
        let known: std::collections::HashSet<_> = snapshot.into_iter().collect();
        if members()?.iter().any(|pid| !known.contains(pid)) {
            return Ok(GroupState::Running);
        }
    }
    Ok(state)
}

// libproc's high-level pidinfo returns String errors. Preserve errno here so
// disappearance/exit races can be distinguished from permission and I/O errors.
fn process_info(pid: i32) -> io::Result<Option<libc::proc_bsdinfo>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let length = i32::try_from(size).map_err(io::Error::other)?;
    // SAFETY: The flavor matches the allocated structure, its exact byte size
    // is supplied, and a nonzero argument includes zombies as well as live PIDs.
    let result = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            1,
            info.as_mut_ptr().cast(),
            length,
        )
    };
    if result <= 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        return Err(error);
    }
    if result != length {
        return Err(io::Error::other("incomplete BSD process info"));
    }
    // SAFETY: A successful call filled the complete structure with kernel data.
    Ok(Some(unsafe { info.assume_init() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_member_after_the_snapshot_keeps_the_group_alive() {
        let mut snapshots = vec![vec![10], vec![11]].into_iter();
        let state = inspect_group(
            || Ok(snapshots.next().expect("membership snapshot")),
            |_| Ok(GroupState::Gone),
        )
        .expect("group inspection");
        assert_eq!(state, GroupState::Running);
    }

    #[test]
    fn committed_exit_blocks_cleanup_without_requiring_more_signals() {
        let state = member_state(libc::SRUN, PROC_FLAG_INEXIT);
        assert_ne!(state, GroupState::Gone);
        assert_ne!(state, GroupState::Running);
        assert_eq!(member_state(libc::SRUN, 0), GroupState::Running);
        assert_eq!(
            member_state(libc::SZOMB, PROC_FLAG_INEXIT),
            GroupState::Gone
        );
        assert_eq!(state.max(GroupState::Running), GroupState::Running);
    }
}

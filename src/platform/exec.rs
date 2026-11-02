//! Direct macOS exec: libc's `execvp`/`posix_spawnp` otherwise interpret `ENOEXEC` as a shell script.
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr, OsString},
    io,
    os::unix::process::CommandExt,
    process::Command,
};

struct PreparedExec {
    candidates: Vec<CString>,
    // Own every allocation referenced by the pointer tables, including after fork.
    _arguments: Vec<CString>,
    _environment: Vec<CString>,
    arguments: Vec<*const libc::c_char>,
    environment: Vec<*const libc::c_char>,
}

// SAFETY: Pointers reference immutable CString allocations owned by this object.
// Moving the object does not move those allocations; no pointers escape exec().
unsafe impl Send for PreparedExec {}
// SAFETY: All pointed-to bytes and pointer tables are immutable after preparation.
unsafe impl Sync for PreparedExec {}

pub fn prepare(command: &mut Command) -> io::Result<()> {
    let program = command.get_program();
    if program.is_empty() {
        return Err(io::Error::from_raw_os_error(libc::ENOENT));
    }
    // Managed commands inherit the environment with explicit overrides. Build
    // everything before fork; no allocation or environment access is safe there.
    let mut environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            environment.insert(key.to_owned(), value.to_owned());
        } else {
            environment.remove(key);
        }
    }
    let candidates = if program.as_encoded_bytes().contains(&b'/') {
        vec![cstring(program)?]
    } else {
        let path = environment.get(OsStr::new("PATH")).map_or_else(
            || OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin"),
            OsString::as_os_str,
        );
        std::env::split_paths(path)
            .map(|directory| cstring(directory.join(program).as_os_str()))
            .collect::<io::Result<Vec<_>>>()?
    };
    let arguments = std::iter::once(program)
        .chain(command.get_args())
        .map(cstring)
        .collect::<io::Result<Vec<_>>>()?;
    let environment = environment
        .into_iter()
        .map(|(mut key, value)| {
            key.push("=");
            key.push(value);
            cstring(&key)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let prepared = PreparedExec {
        candidates,
        arguments: pointers(&arguments),
        environment: pointers(&environment),
        _arguments: arguments,
        _environment: environment,
    };
    // SAFETY: The callback only invokes execve and reads errno with preallocated
    // strings/pointer tables. std has already set up cwd, stdio and process group.
    unsafe {
        command.pre_exec(move || Err(prepared.exec()));
    }
    Ok(())
}

fn cstring(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in command or environment"))
}

fn pointers(strings: &[CString]) -> Vec<*const libc::c_char> {
    strings
        .iter()
        .map(|string| string.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect()
}

impl PreparedExec {
    fn exec(&self) -> io::Error {
        let mut denied = false;
        for candidate in &self.candidates {
            // SAFETY: All tables are NUL-terminated, point to live C strings,
            // and remain owned by this callback until exec succeeds or returns.
            unsafe {
                libc::execve(
                    candidate.as_ptr(),
                    self.arguments.as_ptr(),
                    self.environment.as_ptr(),
                );
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EACCES) => denied = true,
                Some(libc::ENOENT | libc::ENOTDIR) => (),
                // In particular, ENOEXEC must return to the wrapper, never a shell.
                _ => return error,
            }
        }
        io::Error::from_raw_os_error(if denied { libc::EACCES } else { libc::ENOENT })
    }
}

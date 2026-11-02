pub use std::os::unix::fs::symlink;
use std::{
    ffi::OsString,
    fs::{self, File, Permissions},
    io,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::Path,
};

use rustix::fs::{Mode, OFlags};

pub fn os_string(bytes: Vec<u8>) -> OsString {
    OsString::from_vec(bytes)
}

pub fn open_source(path: &Path) -> io::Result<File> {
    Ok(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}

pub fn clone_file(input: &File, output: &mut File, destination: &Path) -> io::Result<()> {
    super::sys::clone_file(input, output, destination)
}

pub fn clone_unsupported(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ENOTSUP)
        || matches!(
            error.raw_os_error(),
            Some(libc::EOPNOTSUPP | libc::EXDEV | libc::ENOTTY | libc::EINVAL | libc::ENOSYS)
        )
}

pub fn make_directory_writable(path: &Path, permissions: &Permissions) -> io::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(permissions.mode() | 0o700))
}

//! OS capabilities used by copying and process supervision; product policy stays with callers.

#[cfg(target_os = "macos")]
mod exec;
pub mod fs;
pub mod process;
mod terminal;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod sys;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod sys;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("git-commitfs currently supports Linux and macOS");

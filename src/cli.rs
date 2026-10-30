use std::ffi::OsString;

use clap::{Parser, Subcommand};

/// Run a command in a disposable checkout of the Git index.
#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Workspace files are isolated, not the whole machine: absolute paths, escaping \
                  symlinks, and environment variables can still write elsewhere."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Execute at the root of the staged checkout; discard workspace writes.
    Run(RunOptions),
}

#[derive(clap::Args)]
pub struct RunOptions {
    /// Include local ignored files using captured ignore rules; keep writes private.
    #[arg(long)]
    pub include_ignored: bool,

    /// How to copy included ignored files; auto falls back to independent copies.
    #[arg(long, value_enum, default_value_t = crate::copy::CopyMode::Auto)]
    pub copy_mode: crate::copy::CopyMode,

    /// Retain the disposable checkout for inspection; this never publishes writes.
    #[arg(long, value_enum, default_value_t = crate::workspace::KeepPolicy::Never)]
    pub keep: crate::workspace::KeepPolicy,

    /// Executable and arguments, passed directly without an implicit shell.
    #[arg(last = true, required = true, num_args = 1..)]
    pub argv: Vec<OsString>,
}

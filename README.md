# git-commitfs

Run commands against a clean, disposable checkout of the Git index. Partially
staged files use their staged content; unstaged edits and ordinary untracked files
are absent. Writes stay in the disposable checkout and are removed afterward.

V1 supports Linux and Git >= 2.43. It needs no root privileges, FUSE, user
namespaces, or mount permissions. macOS and Windows support are deferred.

## Install and run

With the mise-managed toolchain installed (see development setup below):

```sh
mise exec -- just install-cli
git commitfs run -- cargo test
git-commitfs run -- cargo fmt --check
git commitfs run --keep=on-failure -- cargo clippy -- -D warnings
```

Linux v1 builds default to `x86_64-unknown-linux-musl`. `just build` produces a
statically linked executable at
`target/x86_64-unknown-linux-musl/release/git-commitfs`, with no dynamic loader or
shared-library dependencies. You can copy it to another compatible x86-64 Linux
system without installing a particular libc. Git >= 2.43 and the commands and
filters you run must still be available there.

`just install-cli` also selects musl explicitly. For a different target, pass
`--target <triple>` to Cargo (or to `just build` / `just dev-build`) and install
that target's standard library first. macOS and Windows support remain deferred.

The binary works directly or as a Git subcommand. Commands start at the snapshot
repository root, even when invoked from a subdirectory. Everything after `--`
is passed as argv; use an explicit shell for pipelines or multiple commands:

```sh
git commitfs run -- sh -c 'cargo fmt --check && cargo test'
```

A formatter that changes disposable files and returns 0 succeeds. To persist
changes, run a separate command in your normal working tree:

```sh
git commitfs run -- cargo fmt --check
cargo fmt
```

## Ignored files and retained checkouts

```sh
git commitfs run --include-ignored -- cargo build
git commitfs run --include-ignored --copy-mode=copy -- npm test
git commitfs run --keep=always -- sh
```

`--include-ignored` imports local files selected by staged `.gitignore` rules,
source `.git/info/exclude`, and configured global excludes. Dirty or untracked
`.gitignore` files do not supply rules. Tracked snapshot paths take precedence;
staged deletions are not resurrected. Source `.git` metadata is excluded at every
depth. Eligible special files (sockets, FIFOs, devices), unreadable files, and
copy failures stop preparation before your command starts.

Imported files have independent storage. A relative `target/` may be reused as
input, but build changes remain disposable and do not update the source cache.
Importing a large cache can be expensive; selective imports are deferred.
Concurrent writes to source ignored files can produce a mixed copy or an error.

| Option                       | Behavior                                                              |
| ---------------------------- | --------------------------------------------------------------------- |
| `--copy-mode=auto` (default) | Try reflinks; independently copy when unsupported or cross-filesystem |
| `--copy-mode=copy`           | Always independently copy ignored regular files                       |
| `--copy-mode=reflink`        | Require reflinks; fail when unavailable                               |
| `--keep=never` (default)     | Remove the workspace on success or failure                            |
| `--keep=on-failure`          | Keep after a nonzero command result, interruption, or setup error     |
| `--keep=always`              | Keep on every outcome after workspace allocation                      |

Copy mode affects imported ignored files only. Tracked files are materialized
by Git into a fresh directory on every run; there is no persistent checkout cache.
Retention prints `git-commitfs: kept workspace at <path>` on stderr. You can inspect
or remove that directory independently of the original repository. A retained
failed preparation may be incomplete.

## Git behavior and isolation

The effective index is captured without refreshing or rewriting it, including
`GIT_INDEX_FILE` used by hooks and path-limited commits. Linked worktrees, split
indexes, sparse indexes, SHA-1/SHA-256 repositories, and unborn repositories are
supported. A missing index produces an empty candidate. Resolve merge conflicts
and fully stage intent-to-add entries before running. Submodule gitlinks are
currently rejected.

The workspace owns its Git objects and a detached synthetic root commit with the
candidate tree. Local `git status`, `diff`, and ordinary Git writes work. Original
history, branches, tags, remotes, and the future real commit ID are absent.
Source hooks, maintenance, and fsmonitor are not installed in the workspace.
Required checkout filters and staged attributes are respected; filters must be
available locally and complete successfully. Missing objects cause a setup error;
preparation does not fetch them into the original repository. External filter
programs remain local inputs and can have their own side effects.

This is working-copy isolation, not a security sandbox. Absolute paths, escaping
symlinks, and inherited application variables can write elsewhere. For example,
`CARGO_TARGET_DIR=/original/repo/target` sends build output outside the workspace.
Use relative paths for disposable output. Git repository-routing variables are
cleared for the command; other tool configuration and normal environment are
inherited. stdin/stdout/stderr are inherited, with wrapper diagnostics on stderr.

SIGINT, SIGTERM, and SIGHUP are forwarded to managed process groups, including
Git/filter helpers during preparation. Remaining group members are stopped before
cleanup. A second signal or expiration of the two-second grace period escalates
to SIGKILL. Interactive commands receive foreground terminal input. Deliberately
detached processes, SIGKILL of the wrapper, and machine failures can leave files
or processes behind; there is no automatic sweep of unknown temporary directories.
`TMPDIR` must be outside the source checkout.

| Exit code           | Meaning                                                         |
| ------------------- | --------------------------------------------------------------- |
| Command's exit code | Preserved, including 125                                        |
| 2                   | Invalid CLI usage                                               |
| 125                 | Preparation, wrapper, or cleanup failure (diagnostic on stderr) |
| 126 / 127           | Command not executable / not found                              |
| 128 + signal        | Command or wrapper interrupted by a signal                      |

A cleanup failure never masks a failed command's exit code; its diagnostic and
retained path are still reported.

## Hook example

A plain pre-commit hook can run:

```sh
#!/bin/sh
exec git commitfs run -- sh -c 'cargo fmt --check && cargo test --locked'
```

For hk, configure a read-only step with a `check` command such as
`git commitfs run -- cargo test --locked`, without a fixing/staging step or stashing.
The CLI honors the index supplied by Git rather than assuming `.git/index`.

The [approved design](docs/superpowers/specs/2026-10-30-staged-command-design.md)
and [implementation plan](docs/superpowers/plans/2026-10-30-staged-command-execution.md)
record the scope and deferred optimizations.

## Development setup

Install [mise](https://mise.jdx.dev/), Git >= 2.43, Bash, and the usual Linux build
tools. Then run:

```sh
mise trust
mise exec -- just setup
```

Tool versions are pinned in `mise.toml` and resolved in `mise.lock`. Stable Rust
is used for builds/Clippy; the nightly formatter is separately managed through
mise tasks. mise also installs the stable musl target. `just setup` installs both
toolchains and enables hk for this clone, without changing global Git hooks.
Outside an activated mise shell, prefix recipes with `mise exec --`.

```sh
just dev-build
just dev-run --help
just build
just test
just fmt-dev       # Clippy fixes, nightly/stable Rust formatting, Tombi Cargo formatting
just check         # Read-only lockfile, lint, Rust/Cargo format checks, and tests
just check-staged  # The pre-commit checks, against the index
just dev-watch
```

`just fmt-cargo` and `just fmt-cargo-check` find every non-ignored `Cargo.toml`
with fd and pass paths directly to Tombi, including paths containing spaces.
The nightly Rust formatting pass applies the nightly-only options in
`rustfmt.toml`, then the stable pass applies `rustfmt-stable.toml`. The read-only
Rust check validates the final stable layout; the staged hook additionally checks
that the full nightly-then-stable `fmt-dev` pipeline leaves files unchanged.

## Pre-commit checks

hk checks diff hygiene and decorative typographic glyphs. Rust/manifests/tooling
changes also trigger checks against a fresh disposable checkout of the index:
Cargo.lock synchronization, Clippy, Rust formatting, Tombi Cargo formatting,
tests, and verification that `just fmt-dev` leaves that checkout unchanged.
hk neither stashes nor stages changes. Unstaged/untracked source files do not
enter the temporary checkout. The helper uses `target/hk` as an ignored build
cache for read-only checks; the fix pass uses separate temporary build output.
Temporary snapshots and fix artifacts are removed on exit.

The bootstrap script borrows source Git objects and has intentionally narrower
compatibility than the CLI. Migrating these development hooks to the new binary
is a separate self-hosting change. Run a separate normal formatter/version-bump command when
you want its output to persist in the working tree.

## Verification

Linux CI runs the mise-managed checks, the complete nightly-then-stable formatter
pipeline, a release build with ELF checks for static linking, and package inspection.
Separate jobs run the suite with Git 2.43.7 and as a non-root user in a restricted
container with no FUSE, capabilities, or network. Run the container check locally
with Docker and Python 3:

```sh
mise exec -- bash scripts/test-restricted
```

[Validation notes](docs/validation.md) record local compatibility runs and initial
preparation measurements. No speed threshold or persistent-cache speedup is claimed.

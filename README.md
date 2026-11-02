# git-commitfs

Run commands against a clean, disposable checkout of the Git index. Partially
staged files use their staged content; unstaged edits and ordinary untracked files
are absent. Writes stay in the disposable checkout and are removed afterward.

Supports Linux and macOS with Git >= 2.43. No root privileges, FUSE, user
namespaces, or mount permissions are needed. macOS is validated on Apple Silicon;
Intel macOS is not yet validated. Windows and BSD support remain future work.

## Install

Release binaries are published on the
[GitHub releases page](https://github.com/allypost-org/git-commitfs/releases) for
x86-64 Linux (static musl) and Apple Silicon macOS. Git >= 2.43 is still required.

```sh
# mise
mise use --global github:allypost-org/git-commitfs

# cargo-binstall (downloads the release binary instead of compiling)
cargo binstall --git https://github.com/allypost-org/git-commitfs git-commitfs

# from source with Cargo
cargo install --locked --git https://github.com/allypost-org/git-commitfs
```

Each archive is named `git-commitfs-<target>-v<version>.tar.gz`; the release also
carries `SHA256SUMS`, and the archives have GitHub build-provenance attestations:

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify git-commitfs-<target>-v<version>.tar.gz --repo allypost-org/git-commitfs
```

## Run

With a build from the mise-managed toolchain (see development setup below), or one
of the installs above:

```sh
mise exec -- just install-cli   # only when building from a checkout
git commitfs run -- cargo test
git-commitfs run -- cargo fmt --check
git commitfs run --keep=on-failure -- cargo clippy -- -D warnings
```

On x86-64 Linux, mise selects `x86_64-unknown-linux-musl`. `just build` produces a
statically linked executable at
`target/x86_64-unknown-linux-musl/release/git-commitfs`, with no dynamic loader or
shared-library dependencies. You can copy it to another compatible x86-64 Linux
system without installing a particular libc. Git >= 2.43 and the commands and
filters you run must still be available there.

On macOS, mise selects the native Apple target (`aarch64-apple-darwin` on
Apple Silicon); the release binary is under `target/<target>/release/`.
`just install-cli` and the hook bootstrap use the same selected target.
Override it with `CARGO_BUILD_TARGET=<triple> mise exec -- just build` (or
`just install-cli`), after installing that target's standard library. Plain
Cargo outside mise uses the native host target.

Filesystem behavior follows the local disk: APFS supports copy-on-write clones,
but commonly rejects non-UTF-8 filenames and is case insensitive. The CLI keeps
Unix path/argument bytes intact; it cannot materialize filenames the destination
filesystem rejects. Non-UTF-8 arguments remain supported on macOS.

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

Install [mise](https://mise.jdx.dev/) >= 2026.10.24, Git >= 2.43, Bash, and the
usual Linux build tools or macOS Xcode Command Line Tools (`xcode-select --install`).
The CI mise version matches the project's minimum so
older lockfile writers cannot strip tool metadata or task-scoped tools. Then run:

```sh
mise trust
mise exec -- just setup
```

The pinned hk release provides macOS binaries for Apple Silicon only; the
complete `just setup`/hook workflow on Intel Macs is therefore not supported by
this toolchain configuration.

Tool versions are pinned in `mise.toml` and resolved in `mise.lock`. Stable Rust
is used for builds/Clippy; the nightly formatter is separately managed through
mise tasks. mise also installs the stable musl standard library for Linux builds
(this extra target is unused by native macOS builds). `just setup` installs both
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

`just fmt-toml` and `just fmt-toml --check` format and check TOML files with
Tombi. The nightly Rust pass applies the nightly-only options in `rustfmt.toml`,
then stable rustfmt determines the final layout using its supported settings.
`just fmt-stable --check` verifies that final layout; the staged hook additionally
checks that the full nightly-then-stable `fmt-dev` pipeline leaves files unchanged.

## Releasing

```sh
just release 0.2.0   # bumps Cargo.toml/Cargo.lock, commits, and tags v0.2.0 locally
git push origin main v0.2.0
```

Pushing the tag runs `.github/workflows/release.yml`: it tests, builds the Linux
and macOS binaries, checks the version matches the tag, attests the archives and
creates the GitHub release with generated notes (tags with a `-suffix` are marked
as pre-releases). If the repository variable `PUBLISH_CRATE` is `true` and a
crates.io trusted publisher is configured for that workflow, it also publishes the
crate. `just package` builds and packages the current platform's archive into
`dist/` for local inspection.

## Pre-commit checks

hk checks diff hygiene and decorative typographic glyphs. Rust/manifests/tooling
changes, including staged deletions, trigger the self-hosted staged check:

1. `scripts/check-staged-tree` finds a working `git-commitfs` on PATH. If none is
   available, it builds a debug binary into `target/hk-bootstrap` using mise and
   the mise-selected platform target. Cargo reuses those build artifacts on later runs.
2. The wrapper invokes `git-commitfs run -- bash scripts/check-hook-workspace ...`.
   The app captures the effective index and creates the independent checkout.
3. The helper from that staged checkout runs `just check`, then `just fmt-dev`.
   It rejects tracked/index changes or non-ignored untracked files left by the
   modifying pass. These changes stay disposable.

hk neither stashes nor stages changes. Unstaged/untracked source files do not
enter the checks. The fallback bootstrap compiles the current working tree, so
it needs that tree to build; install a working CLI with `just install-cli` to
avoid bootstrap compilation during hooks.

The read-only pass explicitly uses `target/hk` as an ignored compiler cache.
The modifying pass uses fresh build output inside the disposable Git directory,
which the app removes with the checkout, including after interruption. This hook
cache choice does not change the CLI's default isolation. Run a separate normal
formatter/version-bump command when you want its output to persist.

## Verification

Linux and Apple Silicon macOS CI run the mise-managed checks, the complete
nightly-then-stable formatter pipeline, a release build, and package inspection.
Linux also verifies static ELF linking. Separate Linux jobs run the suite with
Git 2.43.7 and as a non-root user in a restricted
container with no FUSE, capabilities, or network. Run the container check locally
with Docker and Python 3:

```sh
mise exec -- bash scripts/test-restricted
```

[Validation notes](docs/validation.md) record local compatibility runs and initial
preparation measurements. No speed threshold or persistent-cache speedup is claimed.

## License

Apache-2.0; see [LICENSE](LICENSE).

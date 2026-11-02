# Local validation

## macOS port (2026-10-31)

The macOS port uses platform capability modules with std/rustix primitives,
plus macOS libproc enumeration. The Linux static target is now selected by mise;
plain Cargo outside mise uses the native host. Copy fallback recognizes both
macOS not-supported error codes. macOS group draining distinguishes running,
committed-to-exit and zombie members, and uses sleeping event waits because the
OS reaps orphaned grandchildren. A direct exec adapter prevents Apple's libc
from interpreting executable text through an implicit shell.

Validation in this change:

- Linux: `mise exec -- just fmt-dev`, `just check` (84 tests), and the
  restricted non-root/container suite (84 tests) pass.
- Real Apple Silicon Mac: macOS 27.0.1, native aarch64-apple-darwin Rust 1.99.1,
  Apple Git 2.50.1. `just fmt-dev` and `just check` pass (87 tests, including three
  macOS-only process-state regressions). The signal/PTY suite passed ten
  consecutive runs after the exiting-process fix.
- Whole-change review found a process-group membership race. An injected
  regression reproduced a newly forked worker missing from an earlier snapshot.
  Completion now preserves positive kernel liveness evidence and rechecks for
  new members before accepting an exiting/gone snapshot; signaling retries a
  newly live group once. The regression and full suites pass after the fix.
- The same real-Git suites cover alternate/split/sparse indexes, partial commits,
  source-state invariance, checkout filters, ignored imports, retention and
  foreground terminal input/restoration on both hosts.
- Native Mac `just dev-run --help`, release build, and `just install-cli` into a
  temporary install root pass. The release is a Mach-O arm64 executable.
- The real hk `just check-staged` pipeline passes on the Mac, using native-target
  bootstrap and the complete read-only/modifying formatter passes. Linux commits
  also execute the real hk pipeline without bypassing hooks.
- Linux release remains a static PIE: no ELF INTERP or NEEDED entries.
  ShellCheck passes the hook scripts; actionlint passes the CI workflow.

The Mac validation uses an isolated temporary checkout. The user's original Mac
clone remains untouched, including its existing mise.lock edits. macOS CI is
configured with the Apple Silicon macos-15 runner; hosted execution of the new
matrix has not been observed in this session. Intel macOS is not validated, and
the pinned hk release has no Intel macOS binary.

APFS rejects the invalid-byte filename fixture with EILSEQ. That capability
branch reports the filesystem limitation; separate newline-path and non-UTF-8
argv tests still execute on macOS. Real clone success remains filesystem-dependent;
Linux tmpfs exercises strict failure and injected policy branches.

The glyph hook now uses POSIX extended regular expressions with UTF-8 byte
escapes. A real BSD-grep regression proves it rejects decorative glyphs while
allowing ASCII and ordinary language Unicode; grep failures no longer silently
pass the check.


## Self-hosted staged checks

The hk staged check now delegates checkout creation and cleanup to
`git-commitfs run`. The wrapper selects an installed executable or builds a
musl debug binary in `target/hk-bootstrap`; the staged
`scripts/check-hook-workspace` runs the validation and modifying formatter passes.
The latter pass has fresh build output inside the disposable Git directory.

Local verification on 2026-10-31 covers `just check` (74 tests), the restricted
non-root container suite with Git 2.43.0, the release build, ShellCheck, and the
real hk `just check-staged` pipeline using the freshly built CLI on PATH. A
separate real invocation with no installed CLI on PATH successfully compiled
the bootstrap binary and completed the same checks.

Regression tests cover installed/built executables, a dangling mise shim,
partial staging, alternate indexes, relative Git routing from subdirectories,
staged deletions, documentation-only skips, paths ending in newlines, and
formatter changes to tracked/index/untracked files.
The toolchain is substituted only in these focused fixtures so they do not
recursively run the suite and can run without Rust/mise in the restricted image.
The complete toolchain was exercised by the real hook runs above.

Self-hosting exposed Git contracting a full index while listing it and refreshing
unchanged source tree-object mtimes through alternates. Index listing now runs
without source alternates; graph export borrows them afterward. A direct
regression compares all source bytes, modes, and file mtimes. Code review also
caught index-only formatter changes canceled by restored working bytes; separate
working-content and cached diffs now reject that case.

## CI mise version fix

The [first hosted run](https://github.com/allypost-org/git-commitfs/actions/runs/41193837453)
passed all tests, including the Git 2.43 and restricted-container jobs. Its
formatting step failed because mise 2026.10.4 rewrote a lockfile generated by
2026.10.24, removing checksum metadata and the task-scoped nightly Rust entry.
The project minimum and all CI jobs now use mise 2026.10.24. The full
nightly-then-stable formatting pipeline and `git diff --exit-code` check remain
in place; lockfile changes are not excluded from verification.

Local reproduction with the verified 2026.10.4 release produced the same lockfile
diff. With the corrected config, `mise install --include-task-tools` under
2026.10.24 leaves the lockfile byte-for-byte unchanged, while 2026.10.4 fails the
minimum-version check before writing it. `just check` passes all 65 tests.

## Static Linux build follow-up

The default target is now `x86_64-unknown-linux-musl`, installed by mise for the
pinned stable toolchain. `mise exec -- just build` produces
`target/x86_64-unknown-linux-musl/release/git-commitfs`. `file` identifies it as
static PIE; `readelf --program-headers --wide` has no `INTERP` entry and
`readelf --dynamic --wide` has no `NEEDED` entries. CI enforces both ELF checks.
The installed executable from `just install-cli` was also checked in a temporary
install root. Git and user commands remain external runtime dependencies.

`mise exec -- just check` and the restricted non-root container check both pass
all 65 tests with musl, including signals and PTY interaction. The original
validation and measurements below predate this target change.

Original local validation was performed on 2026-10-30. The checks below describe
those local runs; hosted CI results and the subsequent correction are recorded above.

## Checks and compatibility

- `mise exec -- just fmt-dev`: Clippy fixes, pinned nightly formatting, stable
  final formatting, then Tombi. The installed hk staged hook also verifies that
  the complete modifying pipeline produces no changes.
- `mise exec -- just check`: locked metadata, Clippy with warnings denied,
  Rust/Cargo formatting checks, and all 65 tests pass.
- `mise exec -- just build`: optimized release binary builds.
- `mise exec -- cargo package --list`: inspect the clean committed package;
  source, tests, tooling, and documentation belong in it; build output and local
  retained workspaces do not. Publication and license selection are outside this work.
- `mise exec actionlint@1.7.12 -- actionlint .github/workflows/ci.yml` and
  `shellcheck scripts/test-restricted .github/containers/run-tests` pass.

Host: Linux 7.2.11-arch1-1, x86-64, UID 1000, Git 2.56.0, Rust 1.99.1
(`58cf8f9c3`, 2026-10-13). Formatter: nightly-2026-10-30 followed by stable
using `rustfmt-stable.toml`.

Minimum-series test: the entire suite passes under Git 2.43.7, built from the
kernel.org source archive with SHA-256
`657e2374455d9e62f6cdb3e7c55d867b6db5404d744e97e112cc5b0db687a19f`.
The build uses `-std=gnu99` for compatibility with this host's newer C compiler.
No alternate Rust toolchain is used:

```sh
mise exec -- bash -c 'export PATH="/path/to/git243/bin:$PATH"; git --version; rustc --version; just test'
```

Restricted test: `mise exec -- bash scripts/test-restricted` passes all 65 tests
as UID 1000 in the pinned Ubuntu 24.04 image with Git 2.43.0. The runner asserts
that effective capabilities are zero and `/dev/fuse` is absent. Runtime options
include `--cap-drop ALL`, `--security-opt no-new-privileges`, `--network none`,
a read-only root filesystem/project/build output, and private writable `/tmp`.
The test executables are built with mise on the host, then run inside this
container at their original paths. No privileged mode or added capability is used.

Real successful reflinks are unavailable on the host's tmpfs temporary storage.
The strict-mode integration test checks the actual filesystem capability and
verifies the expected rejection there. Injected clone-policy tests cover success,
unsupported/cross-device fallback, partial destinations, permission, I/O, and
out-of-space errors. Actual reflink success remains dependent on a capable
filesystem. The unreadable-file test runs as non-root; it explicitly reports a
skip if the current user can read mode-000 files.

## Preparation observations

Release binary, AMD Ryzen 9 9950X3D (32 logical CPUs), source and disposable
workspace on `/tmp` tmpfs. Synthetic repository: 4,000 regular files in 100
directories, each 8,192 bytes, for 32,768,000 tracked bytes. Contents are Python
`random.Random(731).randbytes(8192)` generated in filename order. The baseline is
committed, with no filters or ignored imports. Each timing wraps a complete
`git-commitfs run -- true`, including capture, private Git setup, materialization,
process execution, and cleanup, measured with Python `time.perf_counter()`.

- first invocation: 0.6245 seconds.
- repeat 1: 0.6118 seconds.
- repeat 2: 0.6279 seconds.
- one staged file changed: 0.6306 seconds.

Every run starts with a new workspace and object database. "First invocation"
means no prior git-commitfs run for that fixture; the OS page cache was not flushed
and the repository had just been written. The changed-file case replaces and
stages one 8 KiB file before timing. These are initial observations on one local
fixture, without a speed threshold, cold-disk claim, statistical estimate, or
promise for large repositories. No checkout cache or mount backend was added.

## Review findings resolved

Source-state assertions compare bytes, modes, and file mtimes. Additional
regressions verify that split-index reads never refresh source shared-index
mtimes, sparse expansion never refreshes source objects (including dirty files
outside the cone), and relative object-alternate environment paths retain their
source meaning. Preparation imports independent objects before expanding sparse
entries or writing trees. Read-only imported directories are removable without
changing source modes or following symlink targets.

Real pre-commit tests compare effective index bytes immediately around the CLI
inside the hook, including a successful path-limited commit. A rejected commit
preserves logical staged entries, refs, and working content. Process tests cover
signals during checkout filters, descendant shutdown/escalation, interrupted
retention, and foreground PTY input/Ctrl-C/restoration, with watchdog deadlines.

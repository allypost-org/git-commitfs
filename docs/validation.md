# Local validation

Validated on 2026-10-30. Hosted GitHub Actions jobs are configured but have not
been run remotely. The checks below were executed locally against the implementation.

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

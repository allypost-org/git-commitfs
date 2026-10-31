# Staged Command Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Run arbitrary commands against a clean, disposable checkout of the effective Git index, optionally including independently copied ignored files.

**Architecture:** Discover and capture the candidate, allocate per-run storage, prepare a private Git checkout, and execute the command with explicit lifetime management. V1 uses direct Git checkout without mounts or a persistent checkout cache; optional reflinks apply to imported ignored files. Keep responsibility boundaries concrete so a future workspace provider can reuse the execution contract.

**Tech Stack:** Rust edition 2024; Git CLI >= 2.43; `clap` derive for CLI parsing, `tempfile` for owned storage, `anyhow` for contextual errors, and Linux `libc`/`signal-hook` for reflinks, process groups, and terminal handling. Use standard-library process-based integration tests; no asynchronous runtime or Git library is needed. Resolve dependency versions and commit `Cargo.lock` during Task 1, adding signal dependencies when their task needs them.

**Spec:** [Approved design](../specs/2026-10-30-staged-command-design.md). Read both documents before execution. This plan is the requested deliverable; do not start implementation without a subsequent user instruction.

## Execution record (2026-10-30)

Implementation was subsequently authorized by the user and completed sequentially
in this repository. The checkboxes track delivered behavior and validation, not a
literal transcript of the proposed commands or commit boundaries. Related tasks
were combined into commits where they shared the same working implementation.

The final implementation adds `process.rs` and `terminal.rs` for supervision,
shared with Git helpers through an `Arc<Supervisor>`. `Candidate` retains tracked
path sets rather than public index-entry objects. Capture imports blobs and sparse
trees before expanding the copied index: Git can refresh shared-index timestamps
and alternate-object mtimes during otherwise read-only operations. Possible split
index dependencies are copied before Git reads them; candidate records are then
rebuilt into a fresh private index. Source-state assertions include file mtimes.
No source alternates remain in the finished checkout.

All 65 tests pass locally, with Git 2.43.7, and in the non-root restricted container
using distro Git 2.43.0. The release build, package inspection, formatter pipeline,
Clippy, actionlint, and shellcheck were checked locally. Hosted CI has been
configured but has not run remotely. See [validation notes](../../validation.md)
for commands, environment, measurements, and capability-dependent limitations.
The subsequent self-hosting change keeps a small hk bootstrap wrapper and delegates
checkout preparation and lifetime to git-commitfs; `scripts/check-hook-workspace`
runs the staged validation pipeline. See the validation notes for that follow-up.

## Global Constraints

- Implementation: the existing Rust package, edition 2024, calling the installed Git CLI for Git operations rather than implementing checkout with a Git library.
- Supported platform: Linux, without requiring root, FUSE, user namespaces, or mount permissions. macOS is a later validation target, not a v1 guarantee.
- Proposed Git minimum: 2.43, with an actionable check before preparation.
- The effective index is honored, including `GIT_INDEX_FILE` supplied by a hook or a partial/path-limited commit. Do not assume `.git/index` is the candidate.
- Working tree, index, refs, configuration, hooks, and object files in the source repository are not written by git-commitfs's own preparation or execution logic.
- Commands start at the snapshot repository root; argv uses OS strings and no implicit shell. Unstaged tracked edits and ordinary untracked files are absent.
- Included ignored files use independent storage or reflinks; never source hardlinks or writable source-directory links.
- The workspace has independent Git storage and a detached synthetic root commit. Original history, tags, branches, remotes, and submodules are outside v1.
- Preserve ordinary child exit codes. Wrapper errors: 125; usage: 2; not found: 127; not executable: 126; signal: 128 + signal. Cleanup failure never masks a child failure.
- No writeback, implicit source-cache sharing, mounts, background daemon, checkout cache, or performance threshold in v1.
- Follow the spec's retention, environment, checkout conversion, ignored selection, and lifecycle policies in every task.

## Review Focus

These additional failure conditions have explicit tests in the owning tasks:

1. A source `index.lock` already exists: still read the index without trying to acquire or remove its lock (Task 2).
2. The source disappears after preparation: private Git operations still work; no alternate-object dependency remains (Task 3).
3. Source symlinks or file/directory collisions could redirect ignored copying: never overwrite tracked content or traverse symlink ancestors (Task 4).
4. A reflink attempt creates a destination before failing: fallback replaces the partial file safely; permission/I/O errors remain errors (Task 5).
5. A signal arrives during a checkout filter or the child exits leaving grandchildren: helpers/group members end before cleanup (Task 7).

---

## Current repository and file structure

The repository has a minimal Rust scaffold plus the development setup requested
after design approval: `mise.toml`/`mise.lock`, `justfile`, `hk.pkl`, formatting
configuration, `AGENTS.md`, and a development README. Read `AGENTS.md` before
implementation and preserve any subsequent user changes. Use explicit paths
when staging implementation commits.

Tool versions are managed by mise. Bootstrap with `mise trust` and
`mise exec -- just setup`. Prefer `mise exec -- just <recipe>` for work in an
unactivated shell. `just fmt-dev` applies Clippy fixes followed by nightly Rust,
stable Rust, and Tombi Cargo-manifest formatting. `just check` verifies the
final stable Rust style, Cargo formatting, lockfile, Clippy, and tests; the staged
hook also verifies the full modifying formatter pipeline leaves files unchanged.
Keep version pins in mise rather than introducing another Rust toolchain file.

`scripts/check-staged-tree` is a bootstrap hook helper, not a product backend.
Leave it in place until the new CLI can run the equivalent staged checks; a
later self-hosting change must preserve its Cargo-lock, lint, formatting,
test, diff-hygiene, and no-formatting-changes checks.

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Module declarations, CLI parsing, final diagnostic/exit mapping |
| `src/cli.rs` | `run` arguments and help |
| `src/app.rs` | Stage orchestration and final result precedence |
| `src/git.rs` | Git subprocess environment, byte-safe output, private overrides |
| `src/snapshot.rs` | Source discovery, frozen index, private candidate object graph |
| `src/workspace.rs` | Per-run allocation and retention/cleanup ownership |
| `src/checkout.rs` | Configuration, tracked materialization, synthetic clean commit |
| `src/ignored.rs` | Source traversal and Git-based ignored selection |
| `src/copy.rs` | Independent copy/reflink policy for ignored files |
| `src/runner.rs` | Child status, process groups, signals, terminal ownership |
| `tests/common/mod.rs` | Disposable Git fixtures and source-state assertions |
| `tests/fixtures/` | Small shell filters and child-process fixtures, only as needed |
| `tests/staged_run.rs`, `tests/index_capture.rs` | Core behavior and index variants |
| `tests/checkout.rs`, `tests/ignored.rs`, `tests/copy_modes.rs` | Filesystem fidelity and ignored imports |
| `tests/lifecycle.rs`, `tests/signals.rs` | Exit/retention policies and process lifetime |
| `README.md`, `.github/workflows/ci.yml` | User contract, hook examples, Linux checks |

Keep types internal to this binary; do not add a public library API just for tests.
Unit tests may live beside internal policy helpers; integration tests exercise
the built binary via `env!("CARGO_BIN_EXE_git-commitfs")`.

### Shared interface decisions

These are the final module boundaries. Introduce fields/options in the task
that owns them, retaining the function names and ownership contracts below.
Types not listed here remain local implementation choices.

- `cli::RunOptions { argv: Vec<OsString>, include_ignored: bool, copy_mode: copy::CopyMode, keep: workspace::KeepPolicy }`.
- `snapshot::SourceRepository::discover(cwd: &Path) -> anyhow::Result<SourceRepository>`.
- `snapshot::capture(source: &SourceRepository, run: &workspace::RunDirectory) -> anyhow::Result<Candidate>`.
- `Candidate { tree: OsString, entries: Vec<IndexEntry>, head_tracked: HashSet<PathBuf>, checkout: CheckoutSettings, ignores: IgnoreSettings }`;
  `IndexEntry { path: PathBuf, mode: u32, oid: OsString }`.
  `CheckoutSettings { config: Vec<(OsString, OsString)>, info_attributes: Option<Vec<u8>>, external_attributes: Option<Vec<u8>> }`;
  `IgnoreSettings { info_exclude: Option<Vec<u8>>, global_exclude: Option<Vec<u8>> }`.
  Tree/object IDs are opaque; paths/config values retain OS bytes. The private
  Git directory is `run.checkout().join(".git")`; auxiliary capture files are
  below `run.scratch()`. External attributes/excludes must be relocated into
  private `.git` files and their private config paths updated before execution.
  No live source index/object references remain in the prepared workspace.
- `workspace::RunDirectory::allocate(source_root: &Path) -> anyhow::Result<RunDirectory>`;
  `root(&self)`, `checkout(&self)`, and `scratch(&self)` return `&Path`.
- `checkout::materialize(candidate: &Candidate, run: &RunDirectory) -> anyhow::Result<()>`.
- `ignored::import(source: &SourceRepository, candidate: &Candidate, run: &RunDirectory, mode: CopyMode) -> anyhow::Result<()>`.
- `copy::CopyMode::{Auto, Copy, Reflink}`;
  `copy::regular_file(source: &Path, destination: &Path, mode: CopyMode) -> anyhow::Result<()>`.
- `runner::RunStatus::{Exited(u8), Signaled(i32), NotFound, NotExecutable}`;
  `shell_code(&self) -> u8` implements the spec's status mapping.
- `git::ChildEnvironment` owns repository-routing variable names to remove and
  private settings to apply. `apply(&self, command: &mut std::process::Command)`
  does not alter the wrapper process's global environment.
- `runner::run_command(argv: &[OsString], cwd: &Path, env: &ChildEnvironment) -> anyhow::Result<RunStatus>`.
- `workspace::KeepPolicy::{Never, OnFailure, Always}`;
  `RunDirectory::finalize(self, keep: KeepPolicy, result: &anyhow::Result<RunStatus>) -> Finalization`,
  where `Finalization { retained: Option<PathBuf>, cleanup_error: Option<anyhow::Error> }`.
- `app::execute(options: RunOptions, invocation_dir: &Path) -> AppOutcome`,
  where `AppOutcome { code: u8, retained: Option<PathBuf>, diagnostics: Vec<String> }`.
  This function owns finalization after allocation, including preparation failure.

## Task 1: Deliver a minimal staged-command vertical slice

**Files:** Modify `Cargo.toml`, `src/main.rs`, `.gitignore` only as needed;
create/update `Cargo.lock`; create `src/cli.rs`, `src/app.rs`, `src/git.rs`, `src/snapshot.rs`,
`src/workspace.rs`, `src/checkout.rs`, `src/runner.rs`,
`tests/common/mod.rs`, and `tests/staged_run.rs`.

**Interfaces:** Produce the discovery/capture, allocation/materialization,
basic command/status, and orchestration interfaces above. Initially expose
plain `run -- <argv>`; add later options in their owning tasks. The fixture
offers `Repo::new()`, `write(path, bytes)`, `commit_all()`, `git(&[&str])`,
`run(&[&str]) -> Output`, and `source_state() -> SourceState`.
`SourceState` compares working files/symlinks/modes and relevant Git file contents,
including objects, index, refs, config, and hooks, without refreshing the index.

- [x] **1. Write `partially_staged_content_is_used_and_writes_are_discarded`.**

  ```rust
  let repo = Repo::new();
  repo.write("version.txt", b"base\n");
  repo.commit_all();
  repo.write("version.txt", b"staged\n");
  repo.git(&["add", "version.txt"]);
  repo.write("version.txt", b"unstaged\n");
  let before = repo.source_state();
  let output = repo.run(&["run", "--", "sh", "-c",
      "cat version.txt; printf 'changed\\n' > version.txt; touch generated"]);
  assert_eq!(output.status.code(), Some(0));
  assert_eq!(output.stdout, b"staged\n");
  assert_eq!(repo.source_state(), before);
  ```

  Also assert ordinary untracked files are invisible, tracked deletions are
  absent, `git status --porcelain` is initially empty, and exit 17 preserves
  the source and returns 17. Remove ambient Git configuration from fixtures,
  set fixture identity explicitly, and keep fixture outputs outside the source.
- [x] **2. Run `cargo test --test staged_run`.** Expect a failure because the
  current binary prints `Hello, world!` and does not run the command.
- [x] **3. Implement the plain end-to-end path and add required dependencies.**
  Check Git >= 2.43; reject bare/outside-repository invocation. Allocate a unique
  external run root with `worktree/` and `scratch/`. For an ordinary index,
  copy it and produce its tree using private storage; read source objects only
  during preparation. Stream the candidate tree's full reachable object graph
  through Git pack/index-pack plumbing into the private object store, without
  hardlinks, history, source writes, or buffering a whole pack in memory.
  Materialize with private `read-tree`/`checkout-index`, then set a detached
  unsigned root commit. Use author/committer `git-commitfs <git-commitfs@localhost>`,
  date `2000-01-01T00:00:00+00:00`, and message `git-commitfs staged snapshot`.
  Use an empty template/hooks directory and disable private fsmonitor/automatic
  maintenance. Run argv directly at the workspace root, then explicitly close
  storage. Retain RAII cleanup as a fallback, not the only error-reporting path.
- [x] **4. Run `cargo test --test staged_run`.** Expect all tests to pass,
  including private object ownership and both successful and failed commands.
- [x] **5. Commit only this task's files:** `feat: run commands against a disposable staged checkout`.

## Task 2: Freeze the effective index across Git repository variants

**Files:** Modify `src/snapshot.rs`, `src/git.rs`, `tests/common/mod.rs`;
create `tests/index_capture.rs`.

**Interfaces:** Extend `SourceRepository` and `Candidate` behind the existing
discovery/capture signatures. Produce captured HEAD tracked paths and
self-contained candidate storage; checkout must not reread the live source index.
Extend fixture subprocess construction to accept OS-string argv/environment.

- [x] **1. Add index-capture tests:** an alternate index selects `alternate\n`
  while the normal index selects `normal\n`; linked worktrees resolve their own
  index; split indexes still work; sparse/sparse-index sources populate omitted
  tracked files; SHA-256 candidates retain the source object format. Test unborn
  and absent-index repositories, staged additions/renames/type changes, and an
  unstaged deletion of a tracked file that must reappear from the candidate.
  For conflicts, intent-to-add, or gitlinks, assert exit 125, a specific diagnostic,
  no command marker, and unchanged source state. **Review focus:** hold a source
  `index.lock` and assert a normal run succeeds without changing/removing it.
  Use a child-ready rendezvous to stage again before the child reads its files;
  it must still see the frozen candidate. Add a unit test that replaces the
  index pathname after opening its handle: copying that handle still yields
  the original complete index. Do not use timing sleeps for these tests.
  A missing candidate object must fail setup without invoking a configured
  promisor remote or writing fetched objects into the source.
- [x] **2. Run `cargo test --test index_capture`.** Expect failures in the newly
  unsupported cases; record which expectations fail before changing the code.
- [x] **3. Implement byte-safe discovery and normalization.** Resolve paths with
  Git and preserve non-UTF-8 bytes; remove only record delimiters, never arbitrary
  trailing whitespace. Read one stable index-file handle. Resolve/copy the split
  dependency selected by that exact private index into private storage before
  asking Git to normalize it; no source index update or shared-index refresh.
  Read HEAD once for deletion exclusions. Expand the private tree into a full
  private index; reject invalid states before checkout. Perform object-graph
  reads in a private Git context with borrowed source object directories but
  no promisor/remote configuration; avoid depending on newer Git no-fetch
  options unavailable at the supported minimum. Report missing local objects
  rather than triggering a source-repository fetch.
- [x] **4. Run `cargo test --test index_capture` and `cargo test --test staged_run`.**
  Expect pass with unchanged source files/index/refs/objects in every case.
- [x] **5. Commit:** `feat: capture alternate split and sparse Git indexes safely`.

## Task 3: Preserve checkout conversions and source-independent Git state

**Files:** Modify `src/snapshot.rs`, `src/git.rs`, `src/checkout.rs`;
create `tests/checkout.rs` and filter fixtures under `tests/fixtures/`.

**Interfaces:** Populate `Candidate`'s checkout settings; preserve
`checkout::materialize` and `git::ChildEnvironment::apply` signatures.
Export the sanitized child environment to execution through orchestration.

- [x] **1. Add checkout tests:** binary/empty files, executable bits, symlinks,
  source roots and filenames containing spaces/newlines/non-UTF-8 bytes, and
  filenames ending in newlines.
  `export-ignore` must not hide a tracked file. Stage `.gitattributes`, then edit
  it without staging: only staged rules apply. Exercise CRLF, `info/attributes`,
  an external attributes file, and a local reversible smudge/clean filter;
  private status must stay clean. A failing required filter returns 125 without
  running the user command. A command-side `git add`/commit cannot alter source
  refs/index. Supply hostile inherited source routing variables and assert
  command-side Git discovers only the workspace. **Review focus:** once the
  child starts, move the fixture's source object storage aside and assert
  private `git fsck`/`git show HEAD:path` still work. This deliberate fixture
  mutation is not a source-invariance assertion. Assert the synthetic commit has
  no parent and requires neither ambient identity nor signing.
- [x] **2. Run `cargo test --test checkout`.** Expect failures proving the
  missing conversion/environment cases.
- [x] **3. Implement conversion input capture and private configuration.** Copy
  the exact settings whitelist from the spec, preserving byte-valued config.
  Snapshot external attributes inputs and resolve paths relative to their Git
  configuration context. Force attributes to the candidate tree for checkout;
  never consult dirty tracked attributes. Keep corresponding clean-filter
  configuration in the private repository. Apply private environment settings
  after removing inherited routing variables; do not clear unrelated tool vars.
  Remove all source object alternates before the command, and verify private
  connectivity and the clean tracked baseline. Reject unsupported/missing filter
  prerequisites clearly. Do not claim automatic LFS installation/downloads.
- [x] **4. Run `cargo test --test checkout` and `cargo test --tests`.** Expect
  pass for conversion fidelity, private Git writes, and all earlier behavior.
- [x] **5. Commit:** `feat: preserve staged checkout attributes and private Git state`.

## Task 4: Include ignored local files without importing unstaged content

**Files:** Create `src/ignored.rs`, `src/copy.rs`, `tests/ignored.rs`;
modify `src/cli.rs`, `src/app.rs`, `src/snapshot.rs`, `tests/common/mod.rs`.

**Interfaces:** Add `RunOptions.include_ignored` and `--include-ignored`.
Implement `ignored::import(...)` and `copy::regular_file(...)` with `Copy` mode;
add `CopyMode` variants now, with the optimization implemented in Task 5.
Capture ignore settings and HEAD deletion exclusions in `Candidate`.

- [x] **1. Add ignored-file tests:** default exclusion, opt-in inclusion, nested
  staged rules, negation, staged versus dirty/untracked `.gitignore`, local/global
  excludes, force-added tracked files, and newly ignored staged deletions.
  Verify `target/cache` is private after command edits/removal and remains
  ignored by private Git. Exclude `.git` files/directories inside nested ignored
  repositories. Preserve ignored symlink text without traversal. Eligible FIFOs,
  sockets, unreadable files, or a failed copy cause 125 and no child marker;
  unrelated non-imported special files do not prevent a run. **Review focus:**
  tracked symlink/file/directory ancestors cannot redirect imports or be replaced.
  Ensure `TMPDIR` inside the source cannot produce recursive self-import.
- [x] **2. Run `cargo test --test ignored`.** Expect option/visibility failures.
- [x] **3. Implement Git-based eligibility before copying any source ignore files.**
  Traverse with symlink-aware metadata and OS-string paths; exclude `.git` at
  every depth. Ask private Git to evaluate candidate paths against captured rules,
  using NUL-delimited `check-ignore --no-index` input/output. Interpret negations
  and exit 1 (no matches) correctly. Select all entries before copying so an
  imported `.gitignore` cannot change subsequent eligibility. Give candidate
  tracked paths and captured HEAD deletion exclusions precedence, checking
  ancestors for symlinks/type conflicts. Copy regular files independently and
  preserve modes; never follow directory symlinks. Empty ignored directories may
  be reproduced. Fail atomically with respect to starting the command, not by
  silently skipping eligible unreadable/unsupported inputs.
- [x] **4. Run `cargo test --test ignored` and `cargo test --tests`.** Expect
  pass with unchanged original ignored data, source index, and tracked content.
- [x] **5. Commit:** `feat: optionally import ignored files into disposable storage`.

## Task 5: Add selectable reflink copying with safe fallback

**Files:** Modify `src/copy.rs`, `src/cli.rs`, `src/app.rs`;
create `tests/copy_modes.rs` and unit tests beside copy-policy code.

**Interfaces:** Add `RunOptions.copy_mode` and
`--copy-mode=auto|copy|reflink`, default `Auto`; retain `regular_file` and
`ignored::import` signatures. This option controls ignored regular files only.

- [x] **1. Test all policy branches.** `Copy` creates different inodes and
  independent bytes; `Auto` falls back on unsupported/cross-device cloning;
  `Reflink` returns 125 when unsupported. Both successful paths preserve mode
  and isolate subsequent writes. Inject clone results into an internal policy
  helper to deterministically test unsupported errors, permission failure,
  storage/I/O failure, and cleanup after a partially created destination.
  **Review focus:** partial destination content never survives fallback.
- [x] **2. Run `cargo test --test copy_modes` and `cargo test copy::`.** Expect
  failures for the missing option and clone-policy implementation.
- [x] **3. Implement Linux `FICLONE` with private destination creation.**
  Fallback only for recognized lack-of-support/cross-filesystem errors; propagate
  permission, storage, and I/O failures with source/destination context. Remove
  or safely reset the destination before ordinary-copy fallback. On a filesystem
  with reflinks, exercise a real successful clone; explicitly report a skipped
  success-path integration test when unavailable, while keeping injected policy
  tests mandatory. `--copy-mode` with no imported regular files is a harmless no-op.
- [x] **4. Run `cargo test --test copy_modes`, `cargo test copy::`, and
  `cargo test --test ignored`.** Expect pass; review any capability-dependent skip.
- [x] **5. Commit:** `feat: support reflink copying with explicit fallback policy`.

## Task 6: Finish retention, diagnostics, and command-result precedence

**Files:** Modify `src/workspace.rs`, `src/app.rs`, `src/runner.rs`,
`src/cli.rs`, `src/main.rs`; create `tests/lifecycle.rs`.

**Interfaces:** Add `RunOptions.keep` and `--keep=never|on-failure|always`,
default `Never`. Complete `RunDirectory::finalize`, `Finalization`,
`RunStatus::shell_code`, and `AppOutcome` as specified above.

- [x] **1. Add lifecycle tests:** preserve exit codes 0, 17, and 125; report
  missing/non-executable commands as 127/126; usage as 2 and preparation errors
  as 125. Assert byte-exact argv (including shell metacharacters/non-UTF-8),
  stdin/stdout/stderr inheritance, root cwd from a source subdirectory, and no
  wrapper text on stdout. Cover every retention policy for success, command
  failure, and a failed partial preparation. A retained path is on stderr and
  contains an independently usable repository after deleting the source fixture.
  Two concurrent runs must have distinct paths and independent mutations.
  Deterministically inject finalization failure: success becomes 125; command
  exit 17 remains 17 and the cleanup error is still reported.
- [x] **2. Run `cargo test --test lifecycle`.** Expect failures for the newly
  introduced retention and error-precedence cases.
- [x] **3. Implement the final CLI/orchestration contract.** Only successfully
  prepared workspaces reach `run_command`. Keep all per-run resources under one
  owner; clean temporary capture artifacts before handing over a retained
  checkout, without leaving source-dependent metadata. Finalize every post-
  allocation path explicitly and print kept paths even after preparation failure.
  Treat a nonzero result or wrapper error as failure for retention. Preserve the
  primary result while reporting secondary cleanup errors. Do not expose an
  unimplemented persistence/backend flag or automatically fail on command writes.
- [x] **4. Run `cargo test --test lifecycle` and `cargo test --tests`.** Expect
  pass and no leaked directories except those explicitly retained by tests.
- [x] **5. Commit:** `feat: retain failed workspaces and preserve command results`.

## Task 7: Supervise signals, process groups, and interactive terminals

**Files:** Modify `src/runner.rs`, `src/git.rs`, `src/app.rs`, `Cargo.toml`,
`Cargo.lock`; create `tests/signals.rs` and process fixtures as needed.

**Interfaces:** Keep `run_command` unchanged. Introduce internal
`ProcessSupervisor` and terminal-ownership guards within `runner.rs`, shared
with Git helper execution in `git.rs`; their exact private helpers are builder
choices. Signals must be handled throughout preparation and execution.

- [x] **1. Add deterministic process tests.** A ready marker/pipe confirms the
  child and grandchild exist before sending SIGINT, SIGTERM, or SIGHUP. Assert
  130/143/129 respectively, dead group members, removed workspace, and live caller.
  If the main child exits 17 leaving a worker, terminate the worker and preserve
  17. An ignoring worker is killed after the bounded grace period. **Review
  focus:** interrupt a blocking checkout-filter fixture before the user command;
  its helpers end, exit is 130/143/129 for the corresponding signal, no command
  marker appears, and cleanup occurs. Use a real PTY to check foreground stdin,
  terminal-generated Ctrl-C, and restoration of the
  original foreground process group. Put hard watchdog deadlines around tests
  so a bug fails instead of hanging CI.
- [x] **2. Run `cargo test --test signals -- --test-threads=1`.** Expect failures
  for unsupported group/TTY behavior; never signal a shared test-runner group.
- [x] **3. Implement safe supervision.** Register `signal-hook` handling before
  preparation; handlers only notify normal code. An interrupted preparation
  returns `RunStatus::Signaled` for retention/result mapping, not a generic
  setup-error 125. Spawn managed groups for user commands and blocking Git/filter
  helpers. Coordinate spawn/cancellation so
  interruption cannot leave an unowned child. Transfer foreground terminal
  ownership only when appropriate, handle SIGTTOU around that transfer, and
  restore it with a guard. Forward signals to the managed group, reap the main
  child, then terminate leftovers before cleanup. Proposed grace period: 2 seconds
  followed by SIGKILL; a second termination request escalates immediately. Observe
  completion using process/signal events, not application-level busy polling.
  Document the SIGKILL/crash and deliberately detached-process exceptions.
- [x] **4. Run `cargo test --test signals -- --test-threads=1` and
  `cargo test --all-targets`.** Expect pass with no surviving fixture processes.
- [x] **5. Commit:** `feat: supervise command groups and restore terminal state`.

## Task 8: Document and verify the mount-free Linux release

**Files:** Extend the development `README.md`; create `.github/workflows/ci.yml`;
modify CLI help and existing integration tests only where documentation examples
need automated coverage. Commit any resulting lockfile changes explicitly.

**Interfaces:** The completed CLI and lifecycle contract; no additional product
flags, configuration language, hook installer, or backend framework.

- [x] **1. Add final acceptance assertions.** Help lists only implemented flags
  and explains root cwd, disposable ignored writes, and the non-sandbox boundary.
  Test a real pre-commit invocation with mixed staged/unstaged content and a
  path-limited commit that supplies an alternate index. A rejected commit leaves
  the original staged entries, refs, and working files intact. Check byte-exact
  index invariance immediately around git-commitfs inside the hook; do not
  mistake the outer Git commit's own stat-cache refresh for a wrapper write.
  Test that a formatter-like command may
  change its disposable files and return 0. Test the minimum-version diagnostic
  using a controlled Git version fixture.
- [x] **2. Run `cargo test --all-targets`.** Expect failures for missing final
  help/integration coverage before fixing the remaining contract gaps.
- [x] **3. Write installation and usage documentation plus Linux CI.** Include
  `cargo install --path . --bin git-commitfs`, direct/Git-subcommand examples,
  ignored copy policies, retention, unsupported inputs, and exit statuses.
  Show `git commitfs run -- cargo fmt --check`, followed separately by a normal
  formatter/version-bump command when users want persistent changes. Explain
  that absolute `CARGO_TARGET_DIR` values and escaping symlinks can write outside
  the workspace. Do not reuse the sample's shared writable `target` convention.
  Configure CI to install the mise-pinned tools (including task tools) and run
  `just check` on Linux, using stable Rust for builds/Clippy and the pinned
  nightly for the first formatting pass followed by stable for the final layout.
  In CI's clean checkout, also run `just fmt-dev` followed by
  `git diff --exit-code` to enforce the complete formatting/fix pipeline;
  include an actual Git 2.43.x compatibility job, and a non-root restricted
  container job without `/dev/fuse`, added capabilities, or privileged mode.
  Pin action revisions and build images when implementing the workflow, and
  record exact Git/Rust versions in job output. No mount-based CI setup is needed.
- [x] **4. Run final checks:**

  ```sh
  mise exec -- just check
  mise exec -- just build
  mise exec -- cargo package --list
  ```

  Expect exit 0 from each; inspect package contents for test fixtures and absence
  of retained workspaces/local data. Repeat the test suite under the promised
  minimum Git and restricted non-root environment; until those run, report that
  coverage as pending rather than claiming compatibility. Record cold, repeated,
  and one-staged-file-change measurements as observations with environment and
  repository details, not a pass/fail speed threshold or a reason to add a cache.
- [x] **5. Commit:** `docs: document staged command execution and Linux compatibility`.

## Coverage and handoff

| Design requirement | Owning task |
| --- | --- |
| Staged visibility, disposable writes, private baseline | 1 |
| Effective/alternate/split/sparse indexes, unsupported index states | 2 |
| Checkout filters, attributes, bytes/modes/symlinks, Git independence | 3 |
| Ignored rule sources, collisions, deletion exclusions, special files | 4 |
| Independent copy/reflink selection and error policy | 5 |
| Exit/argv/I/O, retention, concurrent runs, cleanup precedence | 6 |
| Signals, helper cancellation, descendants, PTY restoration | 7 |
| Real hooks/partial commits, help, installation, CI, measurement | 8 |

Before handing off an implemented branch, run the verification commands from
Task 8, inspect the final diff against this spec, and obtain the code review
required by the execution workflow. Do not publish, push, or create a PR as part
of writing this plan. Persistence and optimized checkout providers remain
separate follow-up designs; the interfaces above must not silently enable them.

For later execution, sequential implementation in this session is the simpler
default: the tasks incrementally refine the same capture/preparation/lifetime
contracts. Choose the execution method with the user before starting.

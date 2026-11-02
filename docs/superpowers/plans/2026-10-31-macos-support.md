# macOS Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans for this authorized inline implementation. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Run the existing git-commitfs contract natively on macOS while isolating platform operations for later ports.

**Architecture:** Keep snapshot and product policy separate from filesystem, process and terminal primitives under src/platform. Use std and rustix through small capability interfaces. Keep Linux static builds and validate native macOS behavior on real hardware.

**Tech Stack:** Rust edition 2024, Git >= 2.43, rustix, signal-hook, tempfile, mise and just.

**Spec:** [macOS support design](../specs/2026-10-31-macos-support-design.md). The user authorized the hybrid approach and inline implementation; no additional design-choice handoff is required.

## Global Constraints

- Support native Linux and macOS execution with Git >= 2.43.
- Source handles use no-follow/nonblocking open and must be regular files.
- Destinations are created exclusively.
- Strict reflink never silently copies.
- Never delete a workspace while live or exiting group members remain.
- Windows and BSD are future ports, not claimed supported platforms in this change.
- Preserve the original design's effective index, disposable writes, byte paths, exit codes and two-second process grace period.
- Keep Rust/tool version pins in mise.toml and Linux static ELF CI checks.

## Review Focus

1. Replacing the source pathname after opening must not redirect cloning (Task 1).
2. Existing destination content/symlinks must survive failed imports (Task 1).
3. Orphaned workers that do not generate SIGCHLD must stop before workspace cleanup (Task 1).
4. Hook paths with spaces/newlines and relative/alternate index routing must survive target selection (Task 2).
5. macOS /var aliases and BSD utilities must not hide failures or weaken source-state assertions (Tasks 1-3).

---

## Task 1: Platform filesystem and managed process operations

**Files:** Create src/platform/{mod,fs,process,terminal,linux,macos}.rs; modify src/{main,copy,process,runner,git,snapshot,ignored,workspace}.rs, Cargo.toml/Cargo.lock and tests/{common/mod,copy_modes,signals}.rs. Move src/terminal.rs under platform.

**Interfaces:** platform::fs supplies open_source(&Path) -> io::Result<File>, clone_file(&File, &mut File, &Path) -> io::Result<()>, os_string(Vec<u8>) -> OsString, symlink, and directory permission repair. platform::process::ProcessGroup supplies spawn(&mut Command) -> io::Result<(Child, Self)>, signal(i32), exists() -> io::Result<bool>, reap(), and foreground() -> Result<TerminalGuard>. Platform specialization initializes supervision and supplies an optional completion-check interval. Existing app/Git Supervisor API stays unchanged.

- [x] Add clone safety tests for replaced source pathname, existing destination and destination symlinks; use platform-neutral filesystem assertions in copy integration tests. Adapt signal harness PTY signatures and isolate its Linux subreaper/discovery code without skipping product signal cases.
- [x] Run focused tests on Linux and native Mac. Expected: current Mac compilation fails on FICLONE/prctl; new boundary tests fail until implemented.
- [x] Add direct rustix fs/process dependency; implement selected platform interfaces, preserve copy policy and Linux supervision; add macOS completion observation. Centralize existing Unix path/symlink/mode operations under platform.
- [x] Run all Linux tests and Mac tests. Expected: real-Git behavior and all six signal/PTY tests pass, with hook/tooling failures addressed in Task 2.
- [x] Commit the platform implementation after relevant checks pass.

## Task 2: Native targets, installation and portable hooks

**Files:** Modify .cargo/config.toml, mise.toml/mise.lock, justfile, scripts/check-staged-tree, tests/self_hosted_hook.rs; add a shared target-selection script only if configuration alone cannot keep callers consistent.

**Interfaces:** mise supplies the default CARGO_BUILD_TARGET (musl on x86-64 Linux, native macOS); recipes/install/bootstrap use that value consistently, while honoring explicit overrides. Hook argv/index semantics remain unchanged.

- [x] Add hook fixture coverage for native/default and overridden targets, relative executable PATH, trailing-newline paths and existing alternate-index cases. Expected: current hardcoded musl/GNU realpath behavior fails on Mac or target override.
- [x] Remove Cargo's unconditional Linux target, configure mise host-aware defaults/target installation, use the selected target in install/bootstrap, remove GNU-only path resolution, and lock Linux/macOS tool metadata.
- [x] Run focused hook suite on both hosts, just dev-run --help, release builds and temporary-root install on Mac. Expected: no Linux target/linker flags on macOS and no change to Linux static defaults.
- [x] Commit portable development tooling after validation.

## Task 3: CI, documentation and complete validation

**Files:** Modify .github/workflows/ci.yml, README.md, AGENTS.md, docs/validation.md and this plan's execution record.

**Interfaces:** CI checks Linux and macOS, retaining Linux-only static ELF and restricted jobs. Documentation matches tested behavior and remaining port limitations.

- [x] Configure native macOS CI for full check/formatter/release/package validation. Keep static-link validation Linux-only.
- [x] Update supported-platform, setup, target, hook and future-port documentation.
- [x] Run mise exec -- just fmt-dev and just check on Linux and the Mac, release/static checks, real staged-hook validation, and shell/workflow checks. Expected: all tests/checks pass, full format pipeline leaves tracked files unchanged.
- [x] Obtain one fresh whole-change review, resolve important findings with regressions, record actual validation and limitations, then commit the completed work.

## Execution record

Existing Linux baseline: 74 tests passed before this port. Native Mac baseline
failed compilation at Linux-only cloning/supervision and PTY signatures. The port
passes 84 Linux tests (also in the restricted container) and 87 native Mac tests.
Native signal testing exposed zombie and committed-exit states; both have
regressions. BSD grep and implicit-shell fallback have regressions as well.
Build/install/help and the real Mac staged-hook pipeline pass. Remote validation
used a temporary checkout and preserved the original Mac mise.lock modification.
Whole-change review found one important membership race; its deterministic
regression failed before the fix and passes afterward. No other findings remain.
See docs/validation.md.

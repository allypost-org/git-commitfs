# AGENTS.md

## Project and scope

git-commitfs is a Rust CLI for running commands against a disposable checkout of
the Git index. The approved design and implementation plan are under
`docs/superpowers/`. Read them before changing product behavior. The CLI is
implemented as one package and one binary, `git-commitfs`, under `src/`.
Capture (`snapshot`), materialization (`checkout`), ignored imports (`ignored` and
`copy`), lifetime (`workspace`), and command/process supervision (`runner`,
`process`, `terminal`) have separate responsibilities. Integration tests use real
Git fixtures; `tests/acceptance.rs` also exercises real hooks and partial commits.

## Toolchain and commands

- Rust edition 2024. `mise.toml` is the source of truth for tool versions;
  `mise.lock` records resolutions. Do not duplicate pins in another toolchain file.
- Stable Rust builds, tests, and runs Clippy. A separately pinned nightly runs
  rustfmt because the import/string formatting settings require nightly.
- Linux v1 defaults to `x86_64-unknown-linux-musl` in `.cargo/config.toml`.
  mise installs that target for stable Rust. Release output is
  `target/x86_64-unknown-linux-musl/release/git-commitfs`, statically linked;
  `just install-cli` explicitly selects musl because Cargo install ignores the
  normal build-target default. Keep CI's ELF static-link checks in place.
- mise manages Rust, nightly rustfmt, just, hk, Pkl, Tombi, fd, and watchexec.
  Git >= 2.43, Bash, and standard Linux utilities are system prerequisites.
- Bootstrap with `mise trust` and `mise exec -- just setup`. This installs
  task-scoped formatter tools and this clone's hk hooks, without global hook changes.
- Prefer `mise exec -- just <recipe>`; in an activated mise shell, use `just`.
  Do not prepend a distro Cargo or manually override the mise-selected toolchain.
- `just dev-run [args...]` / `just run [args...]`: debug / release execution.
- `just dev-build` / `just build`: debug / release builds.
- `just dev-watch [args...]` / `just dev-watch-build`: watch source/manifests.
- `just test [args...]`: all targets/features, with Cargo.lock locked.
- `just fmt-dev`: canonical modifying lint/format command: Clippy fixes, nightly
  Rust formatting, stable Rust formatting, and Tombi TOML formatting, in that
  order. The nightly pass applies the nightly-only settings in `rustfmt.toml`;
  stable rustfmt determines the final layout using its supported settings. The
  staged hook verifies the complete pipeline leaves files unchanged, including
  the nightly pre-pass; `just fmt-stable --check` checks the final stable layout.
- `just check`: read-only lockfile, Clippy, Rust/Cargo formatting, and test checks.
- `just check-staged`: run the pre-commit checks against the current index.
- `just install-cli`: install this package's release binary.

Use `just fmt-dev` for routine formatting/fixes. Keep the read-only recipes
available for verification; do not substitute a modifying formatter in CI/hooks.
Run relevant tests once they exist; do not describe an empty test run as coverage.

## Hooks and staged content

`hk.pkl` runs diff hygiene, the typographic-glyph check, and staged Cargo checks.
hk does not stash, fix, or stage files. `scripts/check-staged-tree` selects an
installed `git-commitfs` from PATH and delegates snapshot ownership to
`git-commitfs run`. When no working executable is available (including a dangling
mise shim), it builds the current working tree with mise-selected stable Cargo
into the ignored `target/hk-bootstrap` cache. That bootstrap build needs a
compilable working tree; installing a working CLI avoids that dependency.

`scripts/check-hook-workspace` is executed from the captured index inside the
app's disposable checkout. It runs `just check`, then `just fmt-dev`, rejecting
tracked/index changes and non-ignored untracked additions. Staged deletions and
tooling changes also trigger these checks. The app honors an alternate index
supplied by Git, keeps checkout/index writes private, and owns cleanup. The mise
environment disables Git's optional index refreshes, including during hk's file
discovery; mandatory locks for staging/committing still apply.

The hook explicitly uses `target/hk` as an ignored compiler cache for read-only
checks. The modifying fix pass gets fresh output under the disposable `.git`
directory so compiler-cache wrappers cannot leave it read-only artifacts.
This is a development-hook choice, not product-level source-cache sharing.
Keep the wrapper free of index-copy/checkout/object-store plumbing; the app is
the single implementation. Use `HK=0 git commit` or `--no-verify` only when an
intentional hook bypass is needed, and explain the reason.

## Style

- Clippy pedantic/nursery and `unwrap_used` are warnings, enforced by `just lint`.
  Prefer `?`; use `expect` only when its invariant is clear.
- Rustfmt uses four-space indentation, grouped crate-level imports, vertical
  import layout, and string/macro formatting. Tombi formats Cargo manifests.
- Use ASCII punctuation in committed text: `-`, `->`, `...`, and straight quotes.
  Proper-language Unicode is fine; decorative typographic glyphs are not.
  Represent intentionally tested glyphs with escapes where appropriate.
- Comments should explain non-obvious behavior, invariants, workarounds, or
  rationale. Avoid comments/docstrings that simply repeat the code.
- Keep clap help comments, `SAFETY:` comments, and justified lint suppressions
  accurate; they carry information users/reviewers need.
- Fix stale comments encountered while editing.

## Implementation invariants

- Preserve staged versus unstaged distinctions and alternate-index hook semantics.
- Keep normal workspace writes disposable; no automatic writeback or source-cache
  sharing in the product. Absolute paths/escaping symlinks are not sandboxed.
- Preserve OS-string paths/arguments and NUL-delimited Git output.
- Never hardlink writable workspace files to source files.
- Scope Git environment overrides to subprocesses, not process-wide mutation.
- Keep capture, materialization, copying, and execution responsibilities separate;
  add generic backends only when another real implementation needs them.

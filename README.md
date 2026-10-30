# git-commitfs

Run commands against a disposable checkout of the Git index, excluding unstaged
changes. The product is currently at the design/scaffold stage; the development
setup below is available now.

The [approved design](docs/superpowers/specs/2026-10-30-staged-command-design.md)
and [implementation plan](docs/superpowers/plans/2026-10-30-staged-command-execution.md)
describe the intended first release.

## Development setup

Install [mise](https://mise.jdx.dev/), Git >= 2.43, Bash, and the usual Linux build
tools. Then run:

```sh
mise trust
mise exec -- just setup
```

Tool versions are pinned in `mise.toml` and resolved in `mise.lock`. Stable Rust
is used for builds/Clippy; the nightly formatter is separately managed through
mise tasks. `just setup` installs both and enables hk for this clone, without
changing global Git hooks. Outside an activated mise shell, prefix recipes with
`mise exec --`.

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
compatibility than the planned CLI. Once git-commitfs is implemented, the hooks
can use it instead. Run a separate normal formatter/version-bump command when
you want its output to persist in the working tree.

set positional-arguments

default:
    @just --list

# Install pinned tools, including the formatter, and enable this clone's hooks.
setup:
    mise install --include-task-tools
    mise exec -- hk install --mise

run *args:
    cargo run \
        --locked \
        --release \
        --bin git-commitfs \
        -- \
        "$@"

dev-run *args:
    cargo run \
        --locked \
        --bin git-commitfs \
        -- \
        "$@"

build *args:
    cargo build \
        --locked \
        --release \
        --bin git-commitfs \
        "$@"

dev-build *args:
    cargo build \
        --locked \
        --bin git-commitfs \
        "$@"

_watch *args:
    watchexec \
        --clear=reset \
        --restart \
        --debounce 500ms \
        --watch src \
        --watch Cargo.toml \
        --watch Cargo.lock \
        --stop-signal 'kill' \
        -- \
        "$@"

dev-watch *args:
    just _watch \
        just dev-run "$@"

dev-watch-build *args:
    just _watch \
        just dev-build "$@"

test *args:
    cargo test \
        --locked \
        --workspace \
        --all-targets \
        --all-features \
        "$@"

lock-check:
    @cargo metadata --locked --format-version 1 >/dev/null

_clippy *args:
    cargo clippy \
        --locked \
        --workspace \
        --all-targets \
        --all-features \
        "$@"

lint *args:
    just _clippy -- -D warnings "$@"

lint-fix *args:
    just _clippy --fix --allow-dirty --allow-staged "$@"

fmt-stable *args:
    cargo fmt --all -- "$@" 2>/dev/null

fmt-nightly *args:
    mise run nightly-rustfmt "$@"

fmt-toml *args:
    tombi format "$@"

fmt: fmt-nightly fmt-stable fmt-toml

# Apply Clippy fixes, then format Rust and every Cargo manifest.
fmt-dev: lint-fix fmt

# Read-only checks against the current working tree.
check: lock-check lint (fmt-stable "--check") (fmt-toml "--check") test

# Run the staged checks exactly as the pre-commit hook does.
check-staged:
    hk run pre-commit

install-cli *args:
    cargo install --locked --path . --bin git-commitfs --target x86_64-unknown-linux-musl "$@"

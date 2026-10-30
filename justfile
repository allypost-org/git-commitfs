set positional-arguments

default:
    @just --list

# Install pinned tools, including the formatter, and enable this clone's hooks.
setup:
    mise install --include-task-tools
    mise exec -- hk install --mise

run *args:
    cargo run --locked --release --bin git-commitfs -- "$@"

dev-run *args:
    cargo run --locked --bin git-commitfs -- "$@"

build *args:
    cargo build --locked --release --bin git-commitfs "$@"

dev-build *args:
    cargo build --locked --bin git-commitfs "$@"

dev-watch *args:
    watchexec --clear=reset --restart --debounce 500ms --watch src --watch Cargo.toml --watch Cargo.lock -- just dev-run "$@"

dev-watch-build *args:
    watchexec --clear=reset --restart --debounce 500ms --watch src --watch Cargo.toml --watch Cargo.lock -- just dev-build "$@"

test *args:
    cargo test --locked --workspace --all-targets --all-features "$@"

lock-check:
    @cargo metadata --locked --format-version 1 >/dev/null

lint:
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

lint-fix:
    cargo clippy --locked --fix --allow-dirty --allow-staged --workspace --all-targets --all-features

fmt-stable:
    cargo fmt --all -- --config-path rustfmt-stable.toml

fmt-nightly:
    mise run rustfmt

fmt-rust: fmt-nightly && fmt-stable

fmt-cargo:
    fd --type f --glob Cargo.toml --exec-batch tombi format

fmt: fmt-rust && fmt-cargo

# Apply Clippy fixes, then format Rust and every Cargo manifest.
fmt-dev: lint-fix && fmt

fmt-rust-check:
    cargo fmt --all -- --config-path rustfmt-stable.toml --check

fmt-cargo-check:
    fd --type f --glob Cargo.toml --exec-batch tombi format --check

fmt-check: fmt-rust-check fmt-cargo-check

# Read-only checks against the current working tree.
check: lock-check lint fmt-check test

# Run the staged checks exactly as the pre-commit hook does.
check-staged:
    hk run pre-commit

install-cli:
    cargo install --locked --path . --bin git-commitfs --target x86_64-unknown-linux-musl

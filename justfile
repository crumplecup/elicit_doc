default:
    just --list

check:
    cargo check --features cli

build:
    cargo build --features cli

fmt:
    cargo fmt --all

clippy:
    cargo clippy --features cli -- -D warnings

test:
    cargo test

check-all: fmt clippy test
    @echo "all checks passed"

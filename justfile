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

# Regenerate all coverage CSVs to verif/coverage/
run *ARGS:
    cargo run --features cli -- run {{ARGS}}

# Regenerate only shadow reports
run-shadows *ARGS:
    cargo run --features cli -- run --only shadows {{ARGS}}

# Regenerate only impl coverage reports
run-impls *ARGS:
    cargo run --features cli -- run --only impls {{ARGS}}

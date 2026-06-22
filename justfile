default:
    just --list

check:
    cargo check

build:
    cargo build

fmt:
    cargo fmt --all

clippy:
    cargo clippy -- -D warnings

test:
    cargo test

check-all: fmt clippy test
    @echo "all checks passed"

# Regenerate all coverage CSVs to verif/coverage/
run *ARGS:
    cargo run -- run {{ARGS}}

# Regenerate only shadow reports
run-shadows *ARGS:
    cargo run -- run --only shadows {{ARGS}}

# Regenerate only impl coverage reports
run-impls *ARGS:
    cargo run -- run --only impls {{ARGS}}

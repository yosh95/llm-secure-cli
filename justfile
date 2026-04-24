# Default to showing help
default: help

# List all available commands
help:
    @just --list

lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --all-targets --all-features

audit:
    cargo audit

run:
    cargo run --release

install:
    cargo install --path .

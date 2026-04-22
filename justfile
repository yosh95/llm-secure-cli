default: check-all

check-all:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features
    cargo audit

install:
    cargo install --path .

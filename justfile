# ──── llm-secure-cli Justfile ─────────────────────────────────────────────
# Run `just` or `just --list` to see all available recipes.

default: list

# ──── Development ──────────────────────────────────────────────────────────

# Run all checks (format, lint, test) in sequence
@check: fmt-check clippy test

# Format all source code
@fmt:
    cargo fmt

# Check formatting (CI-friendly, exits non-zero if unformatted)
@fmt-check:
    cargo fmt --check

# Run clippy with strict lints
@clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run clippy with auto-fix (allow dirty working tree)
@clippy-fix:
    cargo clippy --all-targets --fix --allow-dirty

# Run all tests
@test:
    cargo test --all-features

# Run tests with output (no capture)
@test-verbose:
    cargo test --all-features -- --nocapture

# Run only unit tests (exclude integration/bench)
@test-unit:
    cargo test --lib --all-features

# Run only integration tests
@test-integration:
    cargo test --test '*' --all-features

# Run a specific test by name (e.g., `just test-filter security`)
@test-filter filter:
    cargo test --all-features -- {{filter}}

# ──── Build & Install ─────────────────────────────────────────────────────

# Debug build
@build:
    cargo build

# Release build (optimized)
@build-release:
    cargo build --release

# Install the binary locally (~/.cargo/bin/llsc)
@install: build-release
    cargo install --path .

# Install with force (overwrite existing)
@install-force: build-release
    cargo install --force --path .

# ──── Run ──────────────────────────────────────────────────────────────────

# Run the application (interactive mode)
@run *args:
    cargo run -- {{args}}

# Run with release optimizations
@run-release *args:
    cargo run --release -- {{args}}

# ──── Benchmarks ───────────────────────────────────────────────────────────

# Run local security benchmarks
@bench-local:
    cargo bench --bench benchmark_local

# Run Verifier Committee benchmarks (requires API keys)
# Usage: just bench-verifier openrouter amazon/nova-2-lite-v1
@bench-verifier provider model:
    cargo bench --bench benchmark_verifier -- {{provider}} {{model}}

# ──── Documentation ────────────────────────────────────────────────────────

# Build and open documentation
@docs:
    cargo doc --open --no-deps

# Build documentation without opening
@docs-build:
    cargo doc --no-deps

# ──── Security & Auditing ──────────────────────────────────────────────────

# Run cargo-audit (requires `cargo install cargo-audit`)
@audit:
    cargo audit

# Run cargo-deny (requires `cargo install cargo-deny`)
@deny:
    cargo deny check

# Update dependencies
@update:
    cargo update

# Check for outdated dependencies (requires `cargo install cargo-outdated`)
@outdated:
    cargo outdated

# ──── Docker ───────────────────────────────────────────────────────────────

# Build Docker image
@docker-build:
    docker build -t llm-secure-cli .

# Run Docker container interactively
@docker-run:
    docker run -it --rm \
        -v ~/.llsc:/home/agent/.llsc \
        -v $(pwd):/workspace \
        llm-secure-cli

# ──── Cleanup ──────────────────────────────────────────────────────────────

# Remove build artifacts
@clean:
    cargo clean

# Full clean including target directory
@clean-all: clean
    rm -rf target

# ──── CI Pipeline (run before pushing) ─────────────────────────────────────

# Full CI check (format, lint, test, build)
@ci: fmt-check clippy test build-release

# ──── Utility ──────────────────────────────────────────────────────────────

# List all available recipes
@list:
    @just --list

# --- Build Stage ---
FROM rust:1.95-slim-bookworm AS builder

WORKDIR /app
# Copy the entire project
COPY . .
# Build the 'llsc' binary in release mode
RUN cargo build --release

# --- Runtime Stage (Sandbox Environment) ---
FROM debian:bookworm-slim

# Install basic tools for the agent
RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    curl \
    python3 \
    python3-pip \
    vim-tiny \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage
# Cargo installs the binary to target/release/llsc
COPY --from=builder /app/target/release/llsc /usr/local/bin/llsc

# Create directory for the config file
RUN mkdir -p /root/.llm_secure_cli

# Set the working directory
WORKDIR /workspace

# Default security level
ENV LLM_CLI_SECURITY_LEVEL=high

# Set the binary as the entry point
ENTRYPOINT ["llsc"]

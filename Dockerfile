# --- Build Stage ---
FROM rust:slim-trixie AS builder

WORKDIR /app
# Copy the entire project
COPY . .
# Build the 'llsc' binary in release mode
RUN cargo build --release

# --- Runtime Stage (Sandbox Environment) ---
FROM debian:trixie-slim

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
COPY --from=builder /app/target/release/llsc /usr/local/bin/llsc

# Ensure the config directory exists
RUN mkdir -p /root/.llm_secure_cli

# Set the working directory
WORKDIR /workspace

# Default security level
# In 'high' mode, llsc will refuse to start if the binary or manifest is tampered with.
ENV LLM_CLI_SECURITY_LEVEL=high

# Set the binary as the entry point
ENTRYPOINT ["llsc"]

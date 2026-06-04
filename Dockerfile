# --- Build Stage ---
FROM rust:slim-trixie AS builder

WORKDIR /app

# 1. Copy only manifest files
COPY Cargo.toml Cargo.lock ./

# 2. Create a dummy project to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    touch src/lib.rs && \
    cargo build --release && \
    rm -rf src

# 3. Copy the actual source code
COPY src ./src

# 4. Build the final binary
# We touch the main file to ensure cargo rebuilds it
RUN touch src/lib.rs src/main.rs && cargo build --release

# --- Runtime Stage (Sandbox Environment) ---
FROM debian:trixie-slim

# Install basic tools for the agent
RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    curl \
    python3 \
    python3-pip \
    python3-requests \
    poppler-utils \
    vim-tiny \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Create a non-privileged user
RUN useradd -m -u 1000 agent
USER agent
WORKDIR /home/agent

# Copy the binary from the builder stage
COPY --from=builder --chown=agent:agent /app/target/release/llsc /usr/local/bin/llsc

# Ensure the config directory exists
RUN mkdir -p /home/agent/.llsc


# Set the binary as the entry point
ENTRYPOINT ["llsc"]

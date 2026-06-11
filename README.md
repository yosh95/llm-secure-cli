# llm-secure-cli: Unified OpenAI-Compatible CLI for AI Agents

[![CI](https://github.com/yosh95/llm-secure-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/yosh95/llm-secure-cli/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](https://www.rust-lang.org)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20428940.svg)](https://doi.org/10.5281/zenodo.20428939)

`llm-secure-cli` (binary name: `llsc`) is a high-assurance command-line tool designed for interacting with Large Language Models (LLMs). It provides a unified, stable interface for any OpenAI-compatible API, including **OpenRouter, OpenAI, Ollama, and LiteLLM**, prioritizing cognitive focus, secure execution, and extensible automation.

---

### Purpose & Positioning

Enterprise adoption of autonomous AI agents faces a fundamental challenge: how to grant AI meaningful agency while maintaining security and governance standards.

This project explores practical solutions to this challenge through working code.

The framework implements CISSP/CISA/CCSP-level security principles (non-repudiation, PQC resilience) applied to the threat surface introduced by autonomous LLM agents. The design focuses on:

- Providing a reference architecture for high-assurance agentic systems.
- Enabling evaluation of practical trade-offs between AI agent autonomy and security controls.
- Supporting discussion of agentic AI governance through concrete implementation.

---

### Why Audit Logs? — Why AI Agents Need Auditing

Enterprise adoption of autonomous AI agents is hindered by two fundamental risks: destructive actions (unintended file modifications, system changes) and information leakage (exfiltration of sensitive data). Large Language Models are inherently black-box, stochastic systems — their behavior cannot be fully controlled or predicted through rules alone.

This tool addresses these challenges through two key mechanisms:

- Verifier Committee (Multi-LLM Validation): Before any tool execution, N independent LLMs concurrently assess the proposed action for validity, safety, and information leakage risk. If any member flags a concern, the system escalates to Human-in-the-Loop (HITL) for approval.
- Tamper-Evident Audit Logs: Every action is recorded with its full reasoning chain — *why* the agent chose a particular tool with particular arguments — protected by chained hashing and PQC signatures (ML-DSA-87) to ensure non-repudiation and auditability.

The design philosophy draws inspiration from corporate governance: just as artificial persons (corporations) are controlled through mandatory auditing, autonomous AI agents — another form of non-human actor — require audit-based control to be safely entrusted with agency.

---

<p align="center">

```mermaid
flowchart TB
    subgraph UI["💻 User Interface"]
        A["Terminal / CLI"]
        B["User Prompt & Context"]
    end

    subgraph Core["⚙️ Core Orchestration"]
        C["Unified Client<br/>(Provider Registry)"]
        D["Chat Session<br/>(ReAct Loop)"]
    end

    subgraph Providers["LLM Providers"]
        E1["OpenAI"]
        E2["Google Gemini"]
        E3["Anthropic Claude"]
        E4["Ollama (Local)"]
    end

    subgraph Security["🔒 Triple-Lock Security Framework"]
        subgraph T1["Tier 1: Space"]
            F1["Static Analysis<br/>(Control char / Null-byte check)"]
        end
        subgraph T2["Tier 2: Behavior"]
            F2["Verifier Committee<br/>(N-member, any-flag policy)<br/>Semantic Firewall"]
        end
        subgraph T3["Tier 3: Time"]
            F3["PQC (ML-DSA-87 / ML-KEM-1024)<br/>Tamper-evident Audit / Merkle Anchor"]
        end
    end

    subgraph Execution["Secure Execution"]
        G["Secure Tool Executor<br/>(Human-in-the-Loop)"]
    end

    subgraph Tools["External Tools"]
        H1["execute_python<br/>(Universal Executor)"]
        H2["brave_search<br/>(Web Search)"]
    end

    A --> B
    B --> C
    C --> D
    C -.-> Providers
    
    D -->|"Tool Call"| Security
    F1 --> G
    F2 --> G
    F3 --> G
    
    G --> H1
    G --> H2
    
    H1 & H2 -->|"Observations / Results"| D
```

  <br>
  <em>Simplified Architecture Flow</em>
  <br>
  <em>Note: Provider-specific clients (OpenAI, Google Gemini, Anthropic Claude) use the unified OpenAI-compatible client with appropriate payload formatters. Dedicated client implementations exist for OpenRouter and Ollama.</em>
</p>

---

##  Quick Start

1.  Install:
    ```bash
    # Install from source
    git clone https://github.com/yosh95/llm-secure-cli.git
    cd llm-secure-cli
    cargo install --path .
    ```
2.  Set API Keys: `llsc` uses OpenAI-compatible APIs. Set keys for your chosen provider.
    ```bash
    # Example for OpenRouter
    export OPENROUTER_API_KEY="your-api-key"
    
    # Generic provider name support
    # ANYNAME_API_KEY can be used if you define [ANYNAME] in config.toml
    ```
> **Note:** This CLI intentionally omits a dedicated MCP (Model Context Protocol) client.
> The `execute_python` tool allows the LLM to communicate with any MCP server directly
> via Python's `requests` or similar libraries, making a native MCP client redundant.
> All external APIs are accessed through the universal executor, keeping the tool surface
> minimal and the security boundary simple.


### 3. Chat: Type `llsc` to start an interactive session.

1.   Automatic Initialization: On the first run, `~/.llsc/config.toml` is automatically created.
2.   Model Setup: By default, no model is selected. Use `/model <model_name>` (e.g., `/model llama3`) to set one before your first request.
3.   Brave Search: Built-in support for the Brave Search API is available for comprehensive searching across all providers (requires `BRAVE_API_KEY`).
4.   File/URL Attachment: Use `/attach <path|URL>` to quickly add local files or web content to your conversation.
5.  Configure (Optional): Ollama is the default provider. To use OpenRouter or others, edit the configuration file:

    ```bash
    # Edit ~/.llsc/config.toml
    ```

6.  Help: Type `/help` inside the chat to see all commands.

### Docker Isolation (Optional)
Run the agent in a completely isolated container to protect your host system.

1. **Build**: `docker build -t llm-secure-cli .`
2. **Setup API Keys**:
   - **Option A: `.env` file (Recommended)**: Place a `.env` file in your host's `~/.llsc/` directory.
     ```bash
     # ~/.llsc/.env
     OPENROUTER_API_KEY=sk-...
     OPENAI_API_KEY=sk-...
     ```
   - **Option B: Environment Variables**: Pass them via the `-e` flag during `docker run`.
3. **Run**:
   ```bash
   docker run -it --rm \
     -v ~/.llsc:/home/agent/.llsc \
     -v $(pwd):/workspace \
     llm-secure-cli -m llama3 "Summarize the files in this directory"
   ```

### One-Shot Examples
```bash
# Ask a question using the default provider (Ollama) and a specific model
llsc -m llama3 "What is the capital of France?"

# Use a specific provider and model (e.g., OpenRouter)
llsc -p openrouter -m google/gemini-2.0-flash-001 "Explain quantum computing"

# Output raw text to a file (disables Markdown rendering)
llsc -m llama3 --stdout --raw "Write a python script to sort files" > sort.py
```

## Core Features & Tools

- Unified Provider Access: Seamlessly switch between any OpenAI-compatible APIs (**OpenRouter, OpenAI, Ollama, LiteLLM**, and any custom OpenAI-compatible endpoint). Custom providers (e.g., Anthropic Claude, Google Gemini) can be added via config with the appropriate `formatter` setting.
- Autonomous Agent: A streamlined set of built-in tools for complex automation:
    - **Universal Executor**: `execute_python` — run arbitrary Python code for any file operation, data processing, or computation task.
    - **Web Search**: `brave_search` — Brave LLM Context API for grounded, pre-extracted web content (LLM-optimized).
- High-Assurance via Verifier Committee: Every tool call is verified by the **Verifier Committee** — N independent LLMs operating concurrently under an "any-flag" policy — as a Semantic Firewall against unsafe or misaligned tool calls.
- Operational Stability: A clean, flicker-free UI designed for long-term "Deep Work" sessions.
- Human-in-the-Loop: The Verifier Committee auto-approves safe tool calls; potentially unsafe calls always require human confirmation.

### Autonomous Agent Capabilities
The AI agent autonomously selects tools to perform tasks. For example, it can search for a bug with Python file operations, read the relevant code, and apply a fix — all through `execute_python`. All actions are logged with cryptographic signatures for auditability.

---

## Security & Governance (High-Assurance Framework)

As a tool designed with **CISSP/CISA/CCSP** principles in mind, `llm-secure-cli` implements a multi-layered security architecture to mitigate the risks associated with autonomous AI agents.

### 1. Access Control (AI-native ABAC & Verifier Committee)
`llm-secure-cli` implements a modern **Attribute-Based Access Control (ABAC)**, moving away from fragile, platform-dependent static rules.
- **AI-native Policy Engine (Verifier Committee)**: Replaces complex regex blocklists with a hardcoded **Security Constitution**. The system automatically gathers context (OS, User, Directory, Git status) and uses N independent LLM verifiers to judge risks semantically using structured verdicts (ALLOW/REVIEW). This avoids the quagmire of platform-dependent static rules.
- **Any-Flag Policy**: The Verifier Committee runs ALL members concurrently. If ANY member flags a call as requiring review, human approval is mandatory. Only if ALL members approve is the call auto-approved.
- **Path Guardrails (Verifier-based)**: Path validation is handled entirely by the Verifier Committee. The static path whitelist has been removed — the verifier LLM uses its inherent knowledge of sensitive paths (like `C:\Windows` or `/etc`) together with the user's intent context to determine whether a file access is safe.
- **PQC at Maximum Strength**: Security requirements (PQC signature level, audit encryption) are fixed at the highest available NIST Level 5 (ML-DSA-87 for signing, ML-KEM-1024 for encryption). Risk-level-based variant switching has been discontinued.
- **Security Verification (Verifier Committee)**: Every tool call is verified by the Verifier Committee. This acts as a **Semantic Firewall**, ensuring the proposed tool call aligns with the user's intent, respects the Security Constitution (no destructive ops, no sensitive path access, no secret exfiltration), and providing corrected arguments if small discrepancies are detected.
- **Minimalist Fast-fail**: A lightweight syntactic check blocks only control characters and NULL bytes in **nanoseconds**, while the heavy lifting of security judgment is shifted to the Verifier Committee. Shell invocation pattern detection has been removed as redundant — the Verifier Committee handles all semantic analysis.
- **Verifier Fallback**: When the verifier is unavailable (network error, API failure), the system always asks for human approval. The `block` option has been removed — the verifier fallback now always requires manual confirmation.
- **Physical Isolation (Docker)**: The agent can be run inside a Docker container to provide a hard boundary between the AI and the host system.

### 2. Identity & Non-Repudiation (Experimental Reference)
- **Distributed Trust Model**: Implements a decentralized identity model where clients and servers only exchange public keys. This is designed to explore how to prevent lateral movement if a single component is compromised; however, it requires thorough evaluation before use in production environments.
- **Hybrid Identity Tokens**: Uses **COSE (RFC 9052)** binary structures combining **Ed25519** with **Post-Quantum Cryptography (ML-DSA)**. This serves as a reference for how long-term non-repudiation might be handled in autonomous agent systems.
- **Bi-directional Verification**: Tool results can be signed by the responder, allowing the requester to verify that the observations are authentic and untampered within the protocol's scope.

### 3. Observability & Audit Compliance (Tier 3 Reference Implementation)
- Tamper-Evident Audit Logs: Audit trails are protected using **Chained Hashing** and optionally encrypted with **ML-KEM (Kyber)** for confidentiality.
- **Merkle Tree Anchoring**: The Tier 3 implementation uses Merkle Trees to anchor log batches, demonstrating an architecture to prevent historical revisionism and provide compact proofs of session integrity.

---

##  Advanced Commands & Power User Tips

### Command-Line Flags
```bash
llsc [SOURCES...]                    # Start interactive chat (optional initial text/files)
llsc -p <provider>                   # Start with specific provider
llsc -m <model>                      # Start with specific model alias
llsc --stdout                        # Non-interactive mode, output to stdout
llsc --raw                           # Disable Markdown rendering (use with --stdout)
llsc --session <path>                # Load a saved session on startup
llsc "query"                         # One-shot query
```

### Subcommands
```bash
llsc keygen                         # Generate Ed25519 and PQC (ML-DSA/ML-KEM) key pairs
llsc verify-session <id>             # Verify session integrity using Merkle Anchor
llsc list-sessions                   # List available anchored sessions
llsc decrypt-log <input> [-o <out>]  # Decrypt PQC-encrypted audit logs
```

### Interactive Session Commands
Inside the `llsc` interactive session:
- `/help`, `/h`: Show help message.
- `/quit`, `/q`: Exit the application.
- `/verify [on|off]`: Show or toggle verifier status.
- `/vcommittee [set|add|remove|list] [<provider:model>]`: Manage verifier committee members.
- `F2`: Open external editor to edit the current prompt (multi-line composition).
- `/clear`, `/c`: Clear conversation history.
- `/info`, `/i`: Show session info and security status.
- `/edit_history`, `/eh`: View/edit the conversation history in TOML format.
- `/raw`: Show conversation as raw text.
- `/session [load|delete <id>|clear]`: List, load, delete, or clear saved sessions.
- `/attach <path/URL>`: Add a file or website content to the context.
- `/tools [on|off]`: Show or toggle autonomous tool use status.
- `/tool_output`, `/to [on|off]`: Toggle display of tool execution results.
- `/model`, `/m [-u] [<name>]`: List models or switch to `provider:model`. Use `-u` to refresh the cache.
- `/alias` — list all; `/alias <name>` — show one; `/alias <name> <target>` — create/update; `/alias -d <name>` — delete.
- `/summarize`, `/s`: Summarize history and clear it (context preservation).
- `/template`, `/t [<name>]`: List templates or insert one into the prompt.
- `/view [<path>]`: Open saved image or file with system default app.
- `/credits`: Show detailed OpenRouter credit info.

### Keybindings
- **Newline**: `Ctrl+J` (Insert a newline without submitting).
- **Clear Screen**: `Ctrl+L`.
- **History**: `Up/Down` arrows to navigate.
- **Interrupt**: `Ctrl+C` to cancel the current thinking process or exit the session.
- **External Editor**: `F2` (Open external editor to edit the current prompt).

### Power User Tips
- **Backgrounding (`Ctrl+Z`)**: Suspend the session to perform shell operations, then use `fg` to return.
- **Prompt Continuation**: Use `\` at the end of a line or open a code block with ``` to enter multi-line mode automatically.
- **External Editor**: Press `F2` to open your default editor (`vim`, `nano`, etc.) for composing multi-line prompts.
- **Disabling Tools Manually**: Use `/tools off` to prevent errors when using a model that doesn't support function calling.



## Environment Variables

| Variable | Description | Default |
|---|---|---|
| `LLM_SECURE_AUTO_APPROVE` | Automatically answer "Yes" to all confirmation prompts (for testing). | unset (off) |
| `BRAVE_API_KEY` | API key for the built-in Brave Search tool. | — |
| `LLM_CLI_KEY_PASSPHRASE` | Passphrase for encrypted key storage. | — |
| `LLM_CLI_KEY_PASSPHRASE_FILE` | Path to a file containing the passphrase. | — |

Provider API keys are read from environment variables (e.g., `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `OLLAMA_API_KEY`) or from `~/.llsc/.env`.

## Security Configuration Reference

The primary security configuration is in `src/config/defaults.toml` (overridden by `~/.llsc/config.toml`). Security-related runtime state (verifier committee members, enabled/disabled flag) is persisted in `~/.llsc/state.toml`.

### Verifier Committee Configuration
The Verifier Committee uses N independent LLM verifiers with an "any-flag" policy. Configure verifier members via the `/vcommittee` interactive command or directly in `state.toml`:

```toml
# ~/.llsc/state.toml
verifier_enabled = true
verifier_committee_members = ["ollama:gemma4:e2b", "openai:gpt-4o-mini", "openrouter:anthropic/claude-3-haiku"]
```

**Note**: The `[security]` section in `config.toml` is currently unused — verifier settings are managed through `state.toml` (via the `/vcommittee` interactive command). This will be consolidated in a future release.

## Development & Benchmarks
To run the local security primitive benchmarks (Static Analysis, PQC Keygen/Sign/Verify):
```bash
cargo bench --bench benchmark_local
```

To run the internal Verifier benchmark scenarios (requires API keys):
```bash
# Basic usage
cargo bench --bench benchmark_verifier -- <provider> <model>

# Example: Run with OpenRouter
cargo bench --bench benchmark_verifier -- openrouter amazon/nova-2-lite-v1

# Example: Run with Ollama
cargo bench --bench benchmark_verifier -- ollama llama3

# Or with a custom scenarios JSON file:
cargo bench --bench benchmark_verifier -- <provider> <model> path/to/your_scenarios.json
```

##  Development

### Prerequisites

- **Rust** 1.95.0 or later (edition 2024)
- **[just](https://github.com/casey/just)** — a modern command runner (optional, but recommended)

### Quick Checks with `just`

The project includes a `justfile` with common recipes:

```bash
# Show all available commands
just

# Format code
just fmt

# Run clippy with strict lints
just clippy

# Run all tests
just test

# Full CI pipeline (format → clippy → test → build-release)
just ci

# Install the binary locally
just install

# Run the application
just run
```

### CI/CD

CI runs on every push and pull request via [GitHub Actions](.github/workflows/ci.yml):

| Job | Description |
|-----|-------------|
| **Format** | `cargo fmt --check` |
| **Build & Test** | `cargo clippy`, `cargo build --release`, `cargo test` on Ubuntu, macOS, and Windows |
| **Security Audit** | `cargo audit` for dependency vulnerabilities, `cargo deny` for supply-chain security (advisories, licenses, crate duplicates) |
| **Docs** | `cargo doc` with warnings as errors |

### Running Benchmarks

```bash
# Local security primitive benchmarks
just bench-local

# Verifier benchmarks (requires API keys)
just bench-verifier openrouter google/gemini-3.1-flash-lite
```


## Disclaimer

**This tool is not certified or validated for enterprise production use.** No formal third-party security audit has been conducted, and the PQC primitives rely on Rust implementations that have not undergone independent cryptographic review. Deploying this in a regulated or mission-critical environment without additional validation would be inappropriate.

Users should:
- Conduct independent security reviews before production deployment.
- Validate PQC implementations for their specific use cases.
- Understand the limitations of the current implementation.

---

##  License
Licensed under [Apache License 2.0](LICENSE). 


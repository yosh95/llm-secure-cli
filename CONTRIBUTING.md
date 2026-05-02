# Contributing to LLM Secure CLI

## Design Philosophy: AI-Ready Transparency

This project prioritizes "AI-Ready Transparency." As AI agents (like yourself) are increasingly involved in maintaining and extending this codebase, we favor design patterns that are explicit and easy for both humans and AI to reason about.

### Key Principles

1.  **Explicit over Implicit**: Avoid "magical" patterns like complex macros or hidden state. These patterns break static analysis and increase the cognitive load for AI agents that need to infer state across multiple files.
2.  **Flat and Clear Architectures**: Prefer composition and explicit instance management over complex traits or deep nesting.
3.  **State Visibility**: Ensure that the relationship between components (e.g., `ChatSession` and `LlmClient`) is direct and easy to trace.
4.  **Clear Module Structure**: To maintain 100% transparency for AI agents and static analyzers, this project follows a strict policy:
    *   **Explicit Modules**: Use `mod` declarations and separate files for logical components.
    *   **No Re-exports**: Avoid the "Facade Pattern" in `mod.rs` or at the crate root. Never use `pub use module::Struct` to shorten import paths unless absolutely necessary for the public API.
    *   **Direct Pathing**: Always import directly from the source module (e.g., `use crate::llm::registry::ClientRegistry`).
5.  **Small, Focused Files (The 500-Line Rule)**: To prevent AI agents from "losing context" or failing to read the tail of important files, maintain a strict **500-line limit** for all source files.
    *   **Split by Responsibility**: If a file exceeds 500 lines, split it into logical components.
    *   **Clear Naming**: Avoid confusing filenames; choose distinct names for dispatchers versus implementations.

This ensures that any tool (grep, LSP, AI) can find the definition of a struct or function in exactly one location, eliminating "hops" through intermediate proxy files and ensuring full context fits within standard LLM context windows.

### Architecture: Provider Switching

When switching providers (e.g., via the `/p` command), the system uses **Explicit Instance Switching**:
*   The `ChatSession` holds a `Box<dyn LlmClient>` for the active client.
*   When switching providers or models, a new `OpenAiCompatibleClient` instance is created via the `ClientRegistry` (stored in `AppContext` as `Arc<Mutex<ClientRegistry>>`).
*   The `ChatSession.switch_client()` method explicitly copies necessary state (conversation history, tool settings, debug flags) from the old instance to the new one.
*   This ensures that the "source of truth" is always a concrete, specific client object rather than a generic proxy.

> **Note:** All LLM providers (OpenRouter, OpenAI, Ollama, Anthropic, Google, etc.) are handled by the single `OpenAiCompatibleClient` struct. Provider-specific differences (API URLs, model aliases, feature flags like `image_generation` and `tools`) are configuration-driven rather than code-driven. To add a new provider, simply add a `[provider_name]` section with `api_url` to `config.toml`.

### Architecture: Security Pipeline

The security pipeline for tool execution follows three tiers:
1.  **Tier 1 — Static Analysis** (`src/security/static_analyzer.rs`): Fast syntactic checks (shell invocation patterns, control characters). Blocks obvious threats in nanoseconds.
2.  **Tier 2 — Dual LLM Verification** (`src/security/dual_llm_verifier.rs`): Semantic intent verification using a secondary LLM. Evaluates tool calls against the hardcoded Security Constitution.
3.  **Tier 3 — Audit & PQC** (`src/security/audit.rs`, `src/security/pqc.rs`): Tamper-evident logging with chained hashing, ML-DSA signatures, and optional ML-KEM encryption.

The `CASSOrchestrator` (`src/security/cass.rs`) determines the risk level and selects the appropriate security posture for each tool call.

### Slash Commands

All slash commands are routed through `src/cli/interactive/dispatcher.rs`. They interact with the `ChatSession` which holds the current active client instance.

Available commands:
- `/help`, `/h` — Show help
- `/quit`, `/q` — Exit
- `/system [on|off]` — Show or toggle system prompt
- `/edit`, `/e` — Edit in external editor
- `/clear`, `/c` — Clear conversation history
- `/info`, `/i` — Show session info and integrity status
- `/debug` — Toggle live debug mode
- `/raw` — Show raw conversation
- `/dump` — Dump conversation as JSON
- `/save <path>` / `/load <path>` — Save/load session
- `/attach <path>` — Attach file or URL
- `/tools [on|off]` — Toggle tool use
- `/model`, `/m [<alias>]` — Switch or list models
- `/provider`, `/p [<name>]` — Switch or list providers
- `/checkpoint`, `/cp` — Summarize and compress history

### Key Data Structures

- **`AppContext`** (`src/core/context.rs`): Central shared state holding `ConfigManager`, `ToolRegistry`, `ClientRegistry`, and `McpManager`. Passed around as `Arc<AppContext>`.
- **`ChatSession`** (`src/core/session/mod.rs`): Owns the active `LlmClient`, manages conversation flow, security workflow, and audit logging.
- **`ClientState`** (`src/llm/models.rs`): Per-client mutable state (model, provider, conversation, tools enabled, etc.).
- **`SecurityConfig`** (`src/config/models.rs`): All security-related configuration (risk levels, dual LLM settings, verifier fallback, auto-approval).

## Testing

Run tests using `cargo test`. 

When adding new features, ensure:
*   State synchronization is maintained if the feature affects the client or session state.
*   Tests verify explicit behavior rather than assuming implicit delegation.
*   Security features are tested against both allowed and blocked scenarios (see `tests/dual_llm_tests.rs` and `tests/security_tests.rs` for examples).

## Code Style

- **Imports**: Use direct paths (e.g., `use crate::security::cass::RiskLevel;`), not re-exports.
- **Error Handling**: Use `anyhow::Result` for fallible operations in application code.
- **Logging**: Use the `log` crate macros (`log::debug!`, `log::warn!`, `log::error!`). Log levels are controlled at runtime via `--debug` flag or `/debug` command.
- **Configuration**: All user-configurable settings live in `src/config/defaults.toml` and can be overridden in `~/.llm_secure_cli/config.toml`.
# Contributing to LLM Secure CLI

## Design Philosophy: AI-Ready Transparency

This project prioritizes "AI-Ready Transparency." As AI agents (like yourself) are increasingly involved in maintaining and extending this codebase, we favor design patterns that are explicit and easy for both humans and AI to reason about.

### Key Principles

1.  **Explicit over Implicit**: Avoid "magical" patterns like `__getattr__` delegation, complex meta-programming, or hidden state. These patterns break static analysis and increase the cognitive load for AI agents that need to infer state across multiple files.
2.  **Flat and Clear Architectures**: Prefer composition and explicit instance management over deep inheritance or complex proxy wrappers.
3.  **State Visibility**: Ensure that the relationship between components (e.g., `ChatSession` and `BaseLlmClient`) is direct and easy to trace.
4.  **Clear Module Structure**: To maintain 100% transparency for AI agents and static analyzers, this project follows a strict policy:
    *   **Explicit Modules**: Use `mod` declarations and separate files for logical components.
    *   **No Re-exports**: Avoid the "Facade Pattern" in `mod.rs` or at the crate root. Never use `pub use module::Class` to shorten import paths unless absolutely necessary for the public API.
    *   **Direct Pathing**: Always import directly from the source module (e.g., `use crate::clients::registry::client_registry`).
5.  **Small, Focused Files (The 500-Line Rule)**: To prevent AI agents from "losing context" or failing to read the tail of important files, maintain a strict **500-line limit** for all source files.
    *   **Split by Responsibility**: If a file exceeds 500 lines, split it into logical components.
    *   **Clear Naming**: Avoid confusing filenames; choose distinct names for dispatchers versus implementations.

This ensures that any tool (grep, LSP, AI) can find the definition of a class or function in exactly one location, eliminating "hops" through intermediate proxy files and ensuring full context fits within standard LLM context windows.

### Architecture: Provider Switching

When switching providers (e.g., via the `/p` command), the system uses **Explicit Instance Switching**:
*   The `ChatSession` holds a reference to the active `BaseLlmClient`.
*   When switching, a new specific client instance (e.g., `GeminiClient`) is created.
*   The `ChatSession.switch_client()` method explicitly copies necessary state (conversation history, tool settings, debug flags) from the old instance to the new one.
*   This ensures that the "source of truth" is always a concrete, specific client object rather than a generic proxy.

### Slash Commands

All slash commands are routed through `src/clients/command_dispatcher.rs`, with actual implementations located in `src/clients/command_impl.rs`. They receive a `CommandContext` which includes the current active client instance. Always interact with this instance directly.

## Testing

Run tests using `cargo test`. 

When adding new features, ensure:
*   State synchronization is maintained if the feature affects the client or session state.
*   Tests verify explicit behavior rather than assuming implicit delegation.

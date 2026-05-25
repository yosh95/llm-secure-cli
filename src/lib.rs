//! # llm-secure-cli
//!
//! A high-assurance CLI tool for interacting with Large Language Models (LLMs)
//! through OpenAI-compatible APIs.  Provides a unified interface for
//! **OpenRouter, OpenAI, Ollama, and LiteLLM** with a focus on cognitive focus,
//! secure execution, and extensible automation.
//!
//! ## Architecture overview
//!
//! The security framework is organized into three phases executed for every
//! autonomous tool call:
//!
//! | Phase | Module | Description |
//! |-------|--------|-------------|
//! | 1 — Static analysis | [`security::static_analyzer`] | Deterministic fast-fail for null bytes / control chars |
//! | 2 — Verification & approval | [`core::session::phase2_verification`] | Zero Trust, Verifier Committee, human-in-the-loop |
//! | 3 — Execution & audit | [`core::session::phase3_execution`] | Tool execution with cryptographic audit logging |
//!
//! ## Key modules
//!
//! - [`cli`] — Interactive UI, Markdown rendering, syntax highlighting
//! - [`config`] — TOML-based configuration with defaults and user overrides
//! - [`core`] — Session lifecycle, input handling, the four-phase security pipeline
//! - [`llm`] — LLM client abstraction (OpenAI-compatible, Anthropic/Gemini formatters)
//! - [`security`] — ABAC, PQC, identity, audit, Merkle anchoring, path validation
//! - [`tools`] — Built-in tool registry (file ops, search, Python execution, web, MCP)
//! - [`utils`] — Logging, HTTP, chat logging, media handling

#![deny(clippy::unwrap_used)]
#![warn(clippy::expect_used)]

pub mod cli;
pub mod config;
pub mod consts;
pub mod core;
pub mod llm;
pub mod security;
pub mod tools;
pub mod utils;

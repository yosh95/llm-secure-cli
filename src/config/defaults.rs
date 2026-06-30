//! Default configuration values.
//!
//! All defaults are defined as `const` items in a single location
//! and are the **sole** source of truth for default values.
//! No config.toml is involved — users override via CLI flags only.
//!
//! # Convention
//!
//! Every configurable setting has a corresponding `DEFAULT_*` constant here.
//! These are used in three places:
//!
//! 1. `AppConfig::default()` — to build the initial config struct
//! 2. `#[clap(default_value_t = ...)]` — to show defaults in `--help`
//! 3. `CliOverrides::apply_to()` — as the fallback when no CLI flag is given

// ── General ──────────────────────────────────────────────────────────────

/// Default request timeout in seconds for LLM API calls.
pub const DEFAULT_REQUEST_TIMEOUT: u64 = 300;

/// Default verifier timeout in seconds.
pub const DEFAULT_VERIFIER_TIMEOUT: u64 = 60;

/// Default Python execution timeout in seconds.
pub const DEFAULT_PYTHON_TIMEOUT: u64 = 300;

/// Default path for saving generated images.
pub const DEFAULT_IMAGE_SAVE_PATH: &str = "~/Pictures/llsc";

/// Default maximum number of audit log lines to keep.
pub const DEFAULT_MAX_AUDIT_LOG_LINES: usize = 10_000;

/// Default maximum number of chat log lines to keep.
pub const DEFAULT_MAX_CHAT_LOG_LINES: usize = 5_000;

/// Default maximum number of chat archive files to retain.
pub const DEFAULT_MAX_CHAT_ARCHIVES: usize = 5;

/// Default maximum number of output lines per response.
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 5_000;

/// Default maximum number of output characters per response.
pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 50_000;

// ── PQC (Post-Quantum Cryptography) ───────────────────────────────────────

/// Default ML-DSA signature variant.
pub const DEFAULT_SIGNATURE_VARIANT: &str = "ml-dsa-44";

/// Default ML-KEM key encapsulation variant.
pub const DEFAULT_KEM_VARIANT: &str = "ml-kem-512";

// ── Provider API URLs ─────────────────────────────────────────────────────

/// Default Ollama API base URL.
pub const DEFAULT_OLLAMA_API_URL: &str = "http://localhost:11434/v1";

/// Default OpenRouter API base URL.
pub const DEFAULT_OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1";

/// Default vLLM API base URL.
pub const DEFAULT_VLLM_API_URL: &str = "http://localhost:8000/v1";

/// Default OpenAI API base URL.
pub const DEFAULT_OPENAI_API_URL: &str = "https://api.openai.com/v1";

#![deny(clippy::unwrap_used)]
#![warn(clippy::expect_used)]

use clap::{Parser, Subcommand};
use llm_secure_cli::config::defaults;
use llm_secure_cli::config::models::CliOverrides;
use std::io::{IsTerminal, stdin};
use std::process;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    command: Option<Commands>,

    /// Sources (text, files, URLs)
    sources: Vec<String>,

    /// Model to use
    #[clap(short, long)]
    model: Option<String>,

    /// Provider to use (e.g. ollama, openrouter, openai)
    #[clap(short, long)]
    provider: Option<String>,

    /// Print to stdout and exit
    #[clap(short, long)]
    stdout: bool,

    /// Disable Markdown rendering (recommended with --stdout)
    #[clap(long)]
    raw: bool,

    /// Disable Human-in-the-Loop approval.
    /// When set, all tool calls are auto-approved WITHOUT human confirmation.
    /// WARNING: This bypasses the final guardrail — use with extreme caution.
    #[clap(long)]
    disable_human_in_the_loop: bool,

    /// Load a saved session JSON file on startup
    #[clap(long)]
    session: Option<String>,

    /// Override the base directory for config and logs
    #[clap(short = 'D', long, default_value_t = String::from("~/.llsc"))]
    base_dir: String,

    // ── General overrides ──
    /// Request timeout in seconds for LLM API calls
    #[clap(long, default_value_t = defaults::DEFAULT_REQUEST_TIMEOUT)]
    request_timeout: u64,

    /// Verifier timeout in seconds
    #[clap(long, default_value_t = defaults::DEFAULT_VERIFIER_TIMEOUT)]
    verifier_timeout: u64,

    /// Python execution timeout in seconds
    #[clap(long, default_value_t = defaults::DEFAULT_PYTHON_TIMEOUT)]
    python_timeout: u64,

    /// Path for saving generated images
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_IMAGE_SAVE_PATH))]
    image_save_path: String,

    /// Maximum number of audit log lines
    #[clap(long, default_value_t = defaults::DEFAULT_MAX_AUDIT_LOG_LINES)]
    max_audit_log_lines: usize,

    /// Maximum number of chat log lines
    #[clap(long, default_value_t = defaults::DEFAULT_MAX_CHAT_LOG_LINES)]
    max_chat_log_lines: usize,

    /// Maximum number of chat archive files
    #[clap(long, default_value_t = defaults::DEFAULT_MAX_CHAT_ARCHIVES)]
    max_chat_archives: usize,

    /// Maximum number of output lines per response
    #[clap(long, default_value_t = defaults::DEFAULT_MAX_OUTPUT_LINES)]
    max_output_lines: usize,

    /// Maximum number of output characters per response
    #[clap(long, default_value_t = defaults::DEFAULT_MAX_OUTPUT_CHARS)]
    max_output_chars: usize,

    // ── Security overrides ──
    // ── PQC overrides ──
    /// PQC signature variant (ml-dsa-44, ml-dsa-65, or ml-dsa-87)
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_SIGNATURE_VARIANT))]
    signature_variant: String,

    /// PQC KEM variant (ml-kem-512, ml-kem-768, or ml-kem-1024)
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_KEM_VARIANT))]
    kem_variant: String,

    /// Ollama API base URL
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_OLLAMA_API_URL))]
    ollama_url: String,

    /// OpenRouter API base URL
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_OPENROUTER_API_URL))]
    openrouter_url: String,

    /// vLLM API base URL
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_VLLM_API_URL))]
    vllm_url: String,

    /// OpenAI API base URL
    #[clap(long, default_value_t = String::from(defaults::DEFAULT_OPENAI_API_URL))]
    openai_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate Ed25519 and PQC key pairs
    Keygen,
    /// Verify session integrity using Merkle Anchor
    VerifySession {
        /// Session Trace ID to verify
        trace_id: String,
    },
    /// List available sessions (anchored)
    ListSessions,
    /// Decrypt PQC-encrypted audit logs
    DecryptLog {
        /// Path to the encrypted audit log
        input: String,
        /// Path to save the decrypted log
        #[clap(short, long)]
        output: Option<String>,
    },
    /// Check API credits balance (only for OpenRouter provider)
    Credits {
        /// Provider to check credits for
        #[clap(default_value = "openrouter")]
        provider: String,
    },
    /// Show OpenRouter model rankings (token usage leaderboard)
    Rankings {
        /// Provider to check rankings for
        #[clap(default_value = "openrouter")]
        provider: String,
    },
}

fn main() {
    let args = Args::parse();

    // Initialize the base directory for config and logs.
    llm_secure_cli::consts::init_base_dir(if args.base_dir == "~/.llsc" {
        None
    } else {
        Some(std::path::PathBuf::from(&args.base_dir))
    });

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let is_atty = stdin().is_terminal();
    let ui = std::sync::Arc::new(llm_secure_cli::cli::ui::CliUi);

    let ctx = match llm_secure_cli::core::initializer::initialize_app(ui.clone()) {
        Ok(c) => c,
        Err(e) => {
            llm_secure_cli::cli::ui::report_error(&format!("Critical Initialization Error: {e}"));
            process::exit(1);
        }
    };

    // Apply CLI overrides to the config manager.
    // Because all args now have default_value_t, we need to detect which
    // ones the user actually supplied vs. the defaults.
    // We use `Option` internally in CliOverrides; `None` means "use default".
    // Since clap always fills default_value_t, we only pass the override
    // if needed. But a simpler approach: just always pass them.
    // The CliOverrides::apply_to will override whatever is in AppConfig.
    // Since the defaults are the same constants, it's idempotent.

    // Build overrides — but we only set fields that actually differ from
    // the compiled-in defaults, to keep CliOverrides meaningful.
    // However, since we can't easily detect "user-supplied vs default",
    // we just pass everything. The apply_to logic handles it correctly.

    let overrides = CliOverrides {
        request_timeout: Some(args.request_timeout),
        verifier_timeout: Some(args.verifier_timeout),
        python_timeout: Some(args.python_timeout),
        image_save_path: Some(args.image_save_path.clone()),
        max_audit_log_lines: Some(args.max_audit_log_lines),
        max_chat_log_lines: Some(args.max_chat_log_lines),
        max_chat_archives: Some(args.max_chat_archives),
        max_output_lines: Some(args.max_output_lines),
        max_output_chars: Some(args.max_output_chars),
        signature_variant: Some(args.signature_variant.clone()),
        kem_variant: Some(args.kem_variant.clone()),
    };
    ctx.config_manager.set_cli_overrides(overrides);

    // Apply provider URL overrides from CLI args
    apply_provider_url_overrides(&ctx, &args);

    if let Some(command) = args.command {
        handle_subcommand(command, &ctx);
        return;
    }

    if let Err(_e) = llm_secure_cli::cli::commands::chat::start_chat_session(
        llm_secure_cli::cli::commands::chat::ChatArgs {
            provider_arg: args.provider,
            model_arg: args.model,
            session_arg: args.session,
            sources: args.sources,
            stdout: args.stdout,
            raw: args.raw,
            is_atty,
            disable_human_in_the_loop: args.disable_human_in_the_loop,
        },
        ctx,
    ) {
        process::exit(1);
    }
}

/// Apply provider URL overrides from CLI args into the config manager.
fn apply_provider_url_overrides(
    ctx: &std::sync::Arc<llm_secure_cli::core::context::AppContext>,
    args: &Args,
) {
    use llm_secure_cli::config::models::ProviderConfig;
    use std::collections::HashMap;

    let mut url_overrides: HashMap<&str, &str> = HashMap::new();
    url_overrides.insert("ollama", &args.ollama_url);
    url_overrides.insert("openrouter", &args.openrouter_url);
    url_overrides.insert("vllm", &args.vllm_url);
    url_overrides.insert("openai", &args.openai_url);

    if let Ok(mut config) = ctx.config_manager.get_config() {
        let config_mut = std::sync::Arc::make_mut(&mut config);
        for (provider, url) in url_overrides {
            config_mut
                .providers
                .entry(provider.to_string())
                .or_insert_with(|| ProviderConfig {
                    api_key: None,
                    api_url: None,
                })
                .api_url = Some(url.to_string());
        }
        // Write back
        let _ = ctx.config_manager.set_config(config_mut.clone());
    }
}

fn handle_subcommand(
    command: Commands,
    ctx: &std::sync::Arc<llm_secure_cli::core::context::AppContext>,
) {
    match command {
        Commands::Keygen => llm_secure_cli::cli::commands::identity::run_keygen(),
        Commands::VerifySession { trace_id } => {
            llm_secure_cli::cli::commands::identity::run_verify_session(&trace_id);
        }
        Commands::ListSessions => {
            llm_secure_cli::cli::commands::identity::list_anchors();
        }
        Commands::DecryptLog { input, output } => {
            llm_secure_cli::cli::commands::pqc_decrypt::decrypt_log_file(
                input.into(),
                output.map(std::convert::Into::into),
            );
        }
        Commands::Credits { provider } => {
            llm_secure_cli::cli::commands::credits::run_credits(&ctx.config_manager, &provider);
        }
        Commands::Rankings { provider } => {
            llm_secure_cli::cli::commands::rankings::run_rankings(&ctx.config_manager, &provider);
        }
    }
}

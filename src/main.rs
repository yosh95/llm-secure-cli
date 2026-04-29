use clap::{Parser, Subcommand};
use llm_secure_cli::cli::ui;
use llm_secure_cli::core::session::ChatSession;
use llm_secure_cli::llm::providers::anthropic::ClaudeClient;
use llm_secure_cli::llm::providers::google::GeminiClient;
use llm_secure_cli::llm::providers::ollama::OllamaClient;
use llm_secure_cli::llm::providers::openai::OpenAiClient;
use std::process;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    command: Option<Commands>,

    /// Sources (text, files, URLs)
    sources: Vec<String>,

    /// Model alias
    #[clap(short, long)]
    model: Option<String>,

    /// Provider to use
    #[clap(short, long)]
    provider: Option<String>,

    /// Print to stdout and exit
    #[clap(short, long)]
    stdout: bool,

    /// Disable Markdown rendering (recommended with --stdout)
    #[clap(long)]
    raw: bool,

    /// Run as an MCP server
    #[clap(long)]
    mcp_server: bool,

    /// Load a saved session JSON file on startup
    #[clap(long)]
    session: Option<String>,

    /// Enable debug logging
    #[clap(long)]
    debug: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List available models for a provider
    Models {
        /// Provider name (e.g., openai, anthropic, google, ollama)
        provider: Option<String>,
        /// Specific models to show detail for (JSON)
        #[clap(num_args = 0..)]
        models: Vec<String>,
        /// Verbose output (table format)
        #[clap(short, long)]
        verbose: bool,
    },
    /// Identity and Integrity management
    Identity {
        #[clap(subcommand)]
        subcommand: Option<IdentityCommands>,
    },
    /// Decrypt PQC-encrypted audit logs
    DecryptLog {
        /// Path to the encrypted audit log
        input: String,
        /// Path to save the decrypted log
        #[clap(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum IdentityCommands {
    /// Generate RSA and PQC key pairs
    Keygen,
    /// Generate/Update integrity manifest
    Manifest,
    /// Full integrity verification
    Verify {
        /// Verify only the last N lines
        #[clap(long)]
        tail: Option<usize>,
    },
    /// Verify session integrity using Merkle Anchor
    VerifySession {
        /// Session Trace ID to verify
        trace_id: String,
    },
    /// List available sessions (anchored)
    ListSessions,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    use std::io::{IsTerminal, stdin};
    let is_atty = stdin().is_terminal();

    // Initialize logger with 'debug' level by default so we can toggle it at runtime.
    // The actual output is controlled by log::set_max_level.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    if args.debug {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Warn);
    }

    // --- Initialization ---
    let ctx = std::sync::Arc::new(llm_secure_cli::core::context::AppContext::new());
    llm_secure_cli::security::permissions::setup_permissions();

    if !llm_secure_cli::consts::CONFIG_FILE_PATH.exists() {
        llm_secure_cli::config::init::init_config();
    }

    // --- Interactive Setup & Integrity Check ---
    let is_identity_cmd = matches!(args.command, Some(Commands::Identity { .. }));

    if !is_identity_cmd {
        use llm_secure_cli::security::identity::IdentityManager;
        use llm_secure_cli::security::integrity::IntegrityVerifier;

        // 1. Ensure Identity Keys
        if !IdentityManager::has_keys()
            && is_atty
            && ui::ask_confirm("Identity keys not found. Generate new PQC keypair for this agent?")
                .unwrap_or(false)
        {
            if let Err(e) = IdentityManager::ensure_keys(true) {
                ui::report_error(&format!("Failed to generate keys: {}", e));
            } else {
                ui::report_success("Identity keys generated.");
            }
        }

        // 2. System Integrity Check
        let verifier = IntegrityVerifier::new();
        let config = ctx.config_manager.get_config();
        let security_level = std::env::var("LLM_CLI_SECURITY_LEVEL")
            .unwrap_or_else(|_| config.security.security_level.clone());

        if !verifier.manifest_path.exists() {
            let msg = if security_level == "high" {
                "SECURITY FAILURE: Integrity manifest not found. In 'high' security mode, a signed manifest is required."
            } else {
                "Integrity manifest not found. This protects your binary and config from unauthorized changes."
            };

            ui::report_warning(msg);
            if is_atty
                && ui::ask_confirm("Generate and sign integrity manifest now?").unwrap_or(false)
            {
                if let Err(e) = verifier.rebuild_manifest() {
                    ui::report_error(&format!("Failed to build manifest: {}", e));
                    if security_level == "high" {
                        process::exit(1);
                    }
                } else {
                    ui::report_success("Integrity manifest generated.");
                }
            } else if security_level == "high" {
                process::exit(1);
            }
        } else {
            match verifier.verify() {
                Ok(true) => {
                    // Integrity OK
                }
                Ok(false) => {
                    ui::report_warning("CRITICAL: SYSTEM INTEGRITY MISMATCH");
                    eprintln!(
                        "The binary or configuration has changed since the last manifest update."
                    );
                    eprintln!("(This occurs after 'cargo install' or manual configuration edits)");

                    if is_atty
                        && ui::ask_confirm(
                            "Would you like to re-authorize (re-sign) the current system state?",
                        )
                        .unwrap_or(false)
                    {
                        if let Err(e) = verifier.rebuild_manifest() {
                            ui::report_error(&format!("Failed to rebuild manifest: {}", e));
                            if security_level == "high" {
                                process::exit(1);
                            }
                        } else {
                            ui::report_success("Integrity manifest updated.");
                        }
                    } else if security_level == "high" {
                        ui::report_error(
                            "Execution aborted due to integrity failure in 'high' security mode.",
                        );
                        process::exit(1);
                    }
                }
                Err(e) => {
                    log::warn!("Integrity verification error: {}", e);
                }
            }
        }
    }

    // Register built-in tools
    {
        let mut registry = ctx.tool_registry.lock().unwrap();
        llm_secure_cli::tools::registry::register_builtin_tools(&mut registry, &ctx.config_manager);
    }

    // Initialize remote MCP tools if configured
    let _ = llm_secure_cli::tools::registry::initialize_remote_tools(
        ctx.tool_registry.clone(),
        &ctx.config_manager,
        &ctx.mcp_manager,
    )
    .await;

    // Register clients
    {
        let mut registry = ctx.client_registry.lock().unwrap();
        registry.register(
            "openai",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(OpenAiClient::new(config_manager, model, stdout, raw))
            }),
        );
        registry.register(
            "anthropic",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(ClaudeClient::new(config_manager, model, stdout, raw))
            }),
        );
        registry.register(
            "ollama",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(OllamaClient::new(config_manager, model, stdout, raw))
            }),
        );
        registry.register(
            "google",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(GeminiClient::new(config_manager, model, stdout, raw))
            }),
        );
        // Aliases
        registry.register(
            "gpt",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(OpenAiClient::new(config_manager, model, stdout, raw))
            }),
        );
        registry.register(
            "claude",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(ClaudeClient::new(config_manager, model, stdout, raw))
            }),
        );
        registry.register(
            "gemini",
            std::sync::Arc::new(|model, stdout, raw, config_manager| {
                Box::new(GeminiClient::new(config_manager, model, stdout, raw))
            }),
        );
    }

    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            Commands::Models {
                provider,
                models,
                verbose,
            } => {
                if let Some(p) = provider {
                    llm_secure_cli::cli::commands::models::list_models(
                        &ctx.config_manager,
                        &p,
                        models,
                        verbose,
                    )
                    .await;
                } else {
                    let active_providers = ctx.config_manager.get_active_providers();
                    if active_providers.is_empty() {
                        println!("No active providers found. Please set API keys.");
                    } else {
                        for p in active_providers {
                            println!("\n--- Models for {} ---", p);
                            llm_secure_cli::cli::commands::models::list_models(
                                &ctx.config_manager,
                                &p,
                                models.clone(),
                                verbose,
                            )
                            .await;
                        }
                    }
                }
                return;
            }
            Commands::Identity { subcommand } => {
                match subcommand {
                    Some(IdentityCommands::Keygen) => {
                        llm_secure_cli::cli::commands::identity::run_keygen()
                    }
                    Some(IdentityCommands::Manifest) => {
                        llm_secure_cli::cli::commands::identity::run_manifest()
                    }
                    Some(IdentityCommands::Verify { tail }) => {
                        llm_secure_cli::cli::commands::identity::run_verify(tail)
                    }
                    Some(IdentityCommands::VerifySession { trace_id }) => {
                        llm_secure_cli::cli::commands::identity::run_verify_session(&trace_id);
                    }
                    Some(IdentityCommands::ListSessions) => {
                        llm_secure_cli::cli::commands::identity::list_anchors()
                    }
                    None => println!("Please specify an identity subcommand."),
                }
                return;
            }
            Commands::DecryptLog { input, output } => {
                llm_secure_cli::cli::commands::pqc_decrypt::decrypt_log_file(
                    input.into(),
                    output.map(|o| o.into()),
                );
                return;
            }
        }
    }

    if args.mcp_server {
        if let Err(e) = llm_secure_cli::cli::commands::mcp_server::run_mcp_server(ctx.clone()).await
        {
            ui::report_error(&format!("MCP Server Error: {}", e));
            std::process::exit(1);
        }
        return;
    }

    // Standard chat
    let config = ctx.config_manager.get_config();
    let active_providers = ctx.config_manager.get_active_providers();

    let mut provider = args
        .provider
        .unwrap_or_else(|| config.general.unified_default_provider.clone());

    if !active_providers.contains(&provider) {
        // Map common aliases
        let mapped = match provider.as_str() {
            "gpt" => "openai",
            "claude" => "anthropic",
            "gemini" => "google",
            _ => &provider,
        };

        if active_providers.contains(&mapped.to_string()) {
            provider = mapped.to_string();
        } else if !active_providers.is_empty() {
            provider = active_providers[0].clone();
        } else {
            ui::report_error("No active LLM providers found.");
            println!("\nPlease set an API key environment variable for at least one provider:");
            println!("  - GEMINI_API_KEY (or GOOGLE_API_KEY)");
            println!("  - OPENAI_API_KEY");
            println!("  - ANTHROPIC_API_KEY");
            println!("  - Ollama: No API key required for local use.");
            process::exit(1);
        }
    }

    let model = args.model.unwrap_or_else(|| "default".to_string());

    let stdout = args.stdout || !is_atty;

    if args.raw && !stdout {
        ui::report_warning(
            "--raw is primarily intended for use with --stdout. In interactive mode, rich rendering is enabled by default.",
        );
    }

    let client = {
        let registry = ctx.client_registry.lock().unwrap();
        registry.create_client(&provider, &model, stdout, args.raw, &ctx.config_manager)
    };

    if let Some(mut client) = client {
        client.get_state_mut().live_debug = args.debug;

        if let Some(session_path) = args.session {
            match client.load_session(&session_path) {
                Ok(_) => ui::report_success(&format!("Session loaded from {}", session_path)),
                Err(e) => ui::report_error(&format!("Failed to load session: {}", e)),
            }
        }

        let pdf_as_base64 = client.should_send_pdf_as_base64();
        let mut session = ChatSession::new(client, ctx.clone());

        let mut all_sources = args.sources;
        if !is_atty {
            use std::io::Read;
            let mut buffer = String::new();
            if std::io::stdin().read_to_string(&mut buffer).is_ok() {
                let trimmed = buffer.trim();
                if !trimmed.is_empty() {
                    all_sources.insert(0, trimmed.to_string());
                }
            }
        }

        let sources = if all_sources.is_empty() {
            None
        } else {
            Some(llm_secure_cli::utils::media::process_sources(all_sources, pdf_as_base64).await)
        };
        session.run(sources, None).await;
    } else {
        ui::report_error(&format!(
            "Provider '{}' not found or not configured.",
            provider
        ));
        process::exit(1);
    }
}

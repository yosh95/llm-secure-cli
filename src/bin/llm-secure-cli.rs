use clap::{Parser, Subcommand};
use llm_secure_cli::clients::claude::ClaudeClient;
use llm_secure_cli::clients::gemini::GeminiClient;
use llm_secure_cli::clients::ollama::OllamaClient;
use llm_secure_cli::clients::openai::OpenAiClient;
use llm_secure_cli::clients::session::ChatSession;
use llm_secure_cli::ui;
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

    /// Disable Markdown rendering
    #[clap(long)]
    raw: bool,

    /// Enable MCP integration
    #[clap(long)]
    mcp: bool,

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
        #[clap(min_values = 0)]
        models: Vec<String>,
        /// Verbose output (table format)
        #[clap(short, long)]
        verbose: bool,
    },
    /// Benchmark Dual LLM latency
    Benchmark {
        /// The LLM provider alias
        provider: String,
        /// The model name or alias
        model: String,
        /// Number of iterations
        #[clap(short, long, default_value_t = 5)]
        iterations: u32,
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
    /// List available session anchors
    ListAnchors,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if args.debug {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    }

    // --- Initialization ---
    llm_secure_cli::security::permissions::setup_permissions();

    if !llm_secure_cli::consts::CONFIG_FILE_PATH.exists() {
        llm_secure_cli::apps::config_init::init_config();
    }

    // Register clients
    {
        let mut registry = llm_secure_cli::clients::registry::CLIENT_REGISTRY.lock().unwrap();
        registry.register("openai", |model, stdout, raw| {
            Box::new(OpenAiClient::new(model, stdout, raw))
        });
        registry.register("anthropic", |model, stdout, raw| {
            Box::new(ClaudeClient::new(model, stdout, raw))
        });
        registry.register("ollama", |model, stdout, raw| {
            Box::new(OllamaClient::new(model, stdout, raw))
        });
        registry.register("google", |model, stdout, raw| {
            Box::new(GeminiClient::new(model, stdout, raw))
        });
        // Aliases
        registry.register("gpt", |model, stdout, raw| {
            Box::new(OpenAiClient::new(model, stdout, raw))
        });
        registry.register("claude", |model, stdout, raw| {
            Box::new(ClaudeClient::new(model, stdout, raw))
        });
        registry.register("gemini", |model, stdout, raw| {
            Box::new(GeminiClient::new(model, stdout, raw))
        });
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
                    llm_secure_cli::apps::model_listing::list_models(&p, models, verbose).await;
                } else {
                    println!("Please specify a provider.");
                }
                return;
            }
            Commands::Benchmark {
                provider,
                model,
                iterations,
            } => {
                llm_secure_cli::apps::benchmark::run_benchmark(&provider, &model, iterations).await;
                return;
            }
            Commands::Identity { subcommand } => {
                match subcommand {
                    Some(IdentityCommands::Keygen) => llm_secure_cli::apps::identity_tool::run_keygen(),
                    Some(IdentityCommands::Manifest) => {
                        llm_secure_cli::apps::identity_tool::run_manifest()
                    }
                    Some(IdentityCommands::Verify { tail }) => {
                        llm_secure_cli::apps::identity_tool::run_verify(tail)
                    }
                    Some(IdentityCommands::VerifySession { trace_id }) => {
                        llm_secure_cli::apps::identity_tool::run_verify_session(&trace_id);
                    }
                    Some(IdentityCommands::ListAnchors) => {
                        llm_secure_cli::apps::identity_tool::list_anchors()
                    }
                    None => println!("Please specify an identity subcommand."),
                }
                return;
            }
            Commands::DecryptLog { input, output } => {
                llm_secure_cli::apps::pqc_decrypt::decrypt_log_file(
                    input.into(),
                    output.map(|o| o.into()),
                );
                return;
            }
        }
    }

    if args.mcp_server {
        llm_secure_cli::apps::mcp_server::run_mcp_server().await;
        return;
    }

    // Standard chat
    let config_manager = &llm_secure_cli::clients::config::CONFIG_MANAGER;
    let config = config_manager.get_config();
    let active_providers = config_manager.get_active_providers();

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
            println!("  - OLLAMA_API_KEY (required for remote Ollama)");
            process::exit(1);
        }
    }

    let model = args.model.unwrap_or_else(|| "default".to_string());

    let is_atty = unsafe { libc::isatty(0) != 0 };
    let stdout = args.stdout || !is_atty;

    let client = {
        let registry = llm_secure_cli::clients::registry::CLIENT_REGISTRY.lock().unwrap();
        registry.create_client(&provider, &model, stdout, args.raw)
    };

    if let Some(client) = client {
        let mut session = ChatSession::new(client);

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
            Some(llm_secure_cli::modules::media_utils::process_sources(all_sources).await)
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

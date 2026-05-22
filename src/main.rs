#![deny(clippy::unwrap_used)]
#![warn(clippy::expect_used)]

use clap::{Parser, Subcommand};
use std::io::{IsTerminal, stdin};
use std::process;

#[derive(Parser)]
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

    /// Run as a Zero Trust MCP server (requires PQC signature and key registration)
    #[clap(long)]
    mcp_server_zt: bool,

    /// Load a saved session JSON file on startup
    #[clap(long)]
    session: Option<String>,

    /// Override the base directory for config and logs (default: ~/.llm_secure_cli)
    #[clap(short = 'D', long)]
    base_dir: Option<String>,
}

#[derive(Subcommand)]
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
        /// Update models cache
        #[clap(short, long)]
        update: bool,
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
    /// Verify Agent Skills for safety (structural, signature, and semantic checks)
    VerifySkill {
        /// Path to the skill directory or a directory containing multiple skills
        path: String,
        /// Recursively scan for skill directories
        #[clap(short, long)]
        recursive: bool,
        /// Run Semantic Firewall analysis (requires a configured Dual LLM verifier)
        #[clap(short, long)]
        semantic: bool,
        /// Output results as JSON
        #[clap(long)]
        json: bool,
        /// Override the Dual LLM provider for semantic analysis
        #[clap(long)]
        provider: Option<String>,
        /// Override the Dual LLM model for semantic analysis
        #[clap(long)]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum IdentityCommands {
    /// Generate Ed25519 and PQC key pairs
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

    // Initialize the base directory for config and logs.
    llm_secure_cli::consts::init_base_dir(args.base_dir.as_ref().map(std::path::PathBuf::from));

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let is_atty = stdin().is_terminal();
    let ui = std::sync::Arc::new(llm_secure_cli::cli::ui::CliUi);

    let ctx = match llm_secure_cli::core::initializer::initialize_app(ui.clone()).await {
        Ok(c) => c,
        Err(e) => {
            llm_secure_cli::cli::ui::report_error(&format!("Critical Initialization Error: {}", e));
            process::exit(1);
        }
    };

    if let Some(command) = args.command {
        handle_subcommand(command, &ctx).await;
        return;
    }

    if args.mcp_server || args.mcp_server_zt {
        if let Err(e) = llm_secure_cli::cli::commands::mcp_server::run_mcp_server(
            ctx.clone(),
            args.mcp_server_zt,
        )
        .await
        {
            ctx.ui.report_error(&format!("MCP Server Error: {}", e));
            process::exit(1);
        }
        return;
    }

    // Delegates chat session startup to the extracted module.
    llm_secure_cli::cli::commands::chat::start_chat_session(
        llm_secure_cli::cli::commands::chat::ChatArgs {
            provider_arg: args.provider,
            model_arg: args.model,
            session_arg: args.session,
            sources: args.sources,
            stdout: args.stdout,
            raw: args.raw,
            is_atty,
        },
        ctx,
    )
    .await;
}

async fn handle_subcommand(
    command: Commands,
    ctx: &std::sync::Arc<llm_secure_cli::core::context::AppContext>,
) {
    match command {
        Commands::Models {
            provider,
            models,
            verbose,
            update,
        } => {
            if update {
                println!("Updating models cache...");
                ctx.config_manager.update_models_cache().await;
                println!("Cache updated successfully.");
                return;
            }
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
        }
        Commands::Identity { subcommand } => match subcommand {
            Some(IdentityCommands::Keygen) => llm_secure_cli::cli::commands::identity::run_keygen(),
            Some(IdentityCommands::Manifest) => {
                llm_secure_cli::cli::commands::identity::run_manifest()
            }
            Some(IdentityCommands::Verify { tail }) => {
                llm_secure_cli::cli::commands::identity::run_verify(tail)
            }
            Some(IdentityCommands::VerifySession { trace_id }) => {
                llm_secure_cli::cli::commands::identity::run_verify_session(&trace_id)
            }
            Some(IdentityCommands::ListSessions) => {
                llm_secure_cli::cli::commands::identity::list_anchors()
            }
            None => println!("Please specify an identity subcommand."),
        },
        Commands::DecryptLog { input, output } => {
            llm_secure_cli::cli::commands::pqc_decrypt::decrypt_log_file(
                input.into(),
                output.map(|o| o.into()),
            );
        }
        Commands::VerifySkill {
            path,
            recursive,
            semantic,
            json,
            provider,
            model,
        } => {
            llm_secure_cli::cli::commands::skill_verify::run_skill_verify(
                ctx,
                &path,
                recursive,
                semantic,
                json,
                provider.as_deref(),
                model.as_deref(),
            )
            .await;
        }
    }
}

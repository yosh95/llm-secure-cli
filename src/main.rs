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

    /// Load a saved session JSON file on startup
    #[clap(long)]
    session: Option<String>,

    /// Override the base directory for config and logs (default: ~/.llsc)
    #[clap(short = 'D', long)]
    base_dir: Option<String>,
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
    /// Check API credits balance (only for `OpenRouter` provider)
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
            llm_secure_cli::cli::ui::report_error(&format!("Critical Initialization Error: {e}"));
            // SAFETY: No ActiveSession has been created yet, so no Drop
            // destructors (finalize_audit) will be skipped by process::exit.
            process::exit(1);
        }
    };

    if let Some(command) = args.command {
        handle_subcommand(command, &ctx).await;
        return;
    }

    // Delegates chat session startup to the extracted module.
    // start_chat_session returns Result instead of calling process::exit
    // so that the session's Drop (finalize_audit) runs even on failure.
    if let Err(_e) = llm_secure_cli::cli::commands::chat::start_chat_session(
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
    .await
    {
        // No ActiveSession is in scope here — it was either never created
        // or has already been dropped (running finalize_audit via Drop).
        // process::exit is safe here because there are no destructors to skip.
        process::exit(1);
    }
}

async fn handle_subcommand(
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
            llm_secure_cli::cli::commands::credits::run_credits(&ctx.config_manager, &provider)
                .await;
        }
        Commands::Rankings { provider } => {
            llm_secure_cli::cli::commands::rankings::run_rankings(&ctx.config_manager, &provider)
                .await;
        }
    }
}

use clap::{Parser, Subcommand};
use llm_secure_cli::cli::commands::identity;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    subcommand: Option<IdentityCommands>,
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

fn main() {
    let args = Args::parse();

    match args.subcommand {
        Some(IdentityCommands::Keygen) => identity::run_keygen(),
        Some(IdentityCommands::Manifest) => identity::run_manifest(),
        Some(IdentityCommands::Verify { tail }) => identity::run_verify(tail),
        Some(IdentityCommands::VerifySession { trace_id }) => {
            identity::run_verify_session(&trace_id);
        }
        Some(IdentityCommands::ListAnchors) => identity::list_anchors(),
        None => println!("Please specify a subcommand. Use --help for details."),
    }
}

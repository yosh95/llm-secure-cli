use clap::{Parser, Subcommand};
use llm_secure_cli::apps::identity_tool;

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
        Some(IdentityCommands::Keygen) => identity_tool::run_keygen(),
        Some(IdentityCommands::Manifest) => identity_tool::run_manifest(),
        Some(IdentityCommands::Verify { tail }) => identity_tool::run_verify(tail),
        Some(IdentityCommands::VerifySession { trace_id }) => {
            identity_tool::run_verify_session(&trace_id);
        }
        Some(IdentityCommands::ListAnchors) => identity_tool::list_anchors(),
        None => println!("Please specify a subcommand. Use --help for details."),
    }
}

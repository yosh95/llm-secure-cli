use clap::Parser;
use llm_secure_cli::apps::model_listing;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Provider name (e.g., openai, anthropic, google, ollama)
    provider: Option<String>,
    /// Specific models to show detail for (JSON)
    #[clap(min_values = 0)]
    models: Vec<String>,
    /// Verbose output (table format)
    #[clap(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Some(p) = args.provider {
        model_listing::list_models(&p, args.models, args.verbose).await;
    } else {
        println!("Please specify a provider.");
    }
}

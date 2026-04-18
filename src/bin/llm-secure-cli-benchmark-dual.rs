use clap::Parser;
use llm_secure_cli::apps::benchmark;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The LLM provider alias
    provider: String,
    /// The model name or alias
    model: String,
    /// Number of iterations
    #[clap(short, long, default_value_t = 5)]
    iterations: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    benchmark::run_benchmark(&args.provider, &args.model, args.iterations).await;
}

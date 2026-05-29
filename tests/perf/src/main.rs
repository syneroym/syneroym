pub mod orchestrator;
pub mod reporter;
pub mod scenarios;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run latency tests
    Latency,
    /// Run concurrency tests (Category 3) - Placeholder
    Concurrency,
    /// Run soak tests (Category 5) - Placeholder
    Soak,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Latency => {
            println!("Running Phase 2: Latency Overhead Tests");
            scenarios::tcp_proxy_latency::run_scenario().await?;
            scenarios::wasm_latency::run_scenario().await?;
        }
        Commands::Concurrency => {
            println!("Concurrency tests not yet implemented.");
        }
        Commands::Soak => {
            println!("Soak tests not yet implemented.");
        }
    }

    Ok(())
}

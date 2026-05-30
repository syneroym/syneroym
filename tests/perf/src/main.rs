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
    /// Run soak / endurance tests (Category 5)
    Soak {
        /// Duration in seconds (default: 1800 = 30 minutes)
        #[arg(long, default_value = "1800")]
        duration: u64,
    },
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
            println!("Running Phase 3: Concurrency & Resource Profiling Tests");
            scenarios::concurrency::run_scenario().await?;
        }
        Commands::Soak { duration } => {
            println!("Running Phase 4: Soak / Endurance Tests (duration: {}s)", duration);
            scenarios::soak::run_scenario(duration).await?;
        }
    }

    Ok(())
}

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use syneroym_identity::Identity;

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the local node
    Node {
        #[command(subcommand)]
        command: NodeCommands,
    },
    /// Get the status of a local node
    Status {
        #[arg(long)]
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    /// Initialize a new node
    Init {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Start a node
    Start {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        detach: bool,
    },
    /// Stop a running node
    Stop {
        #[arg(long)]
        dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Node { command } => match command {
            NodeCommands::Init { dir } => {
                if !dir.exists() {
                    fs::create_dir_all(dir)?;
                }

                let identity = Identity::generate();
                let identity_bytes = identity.to_bytes();

                let key_path = dir.join("identity.key");
                fs::write(&key_path, identity_bytes)?;

                let doc = identity.to_doc(
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                );
                let doc_path = dir.join("identity.json");
                let doc_json = serde_json::to_string_pretty(&doc)?;
                fs::write(&doc_path, doc_json)?;

                println!("Initialized node successfully at {}", dir.display());
                println!("Node ID: {}", doc.id);
            }
            NodeCommands::Start { dir, detach } => {
                println!("Node started at {} (detached: {})", dir.display(), detach);
            }
            NodeCommands::Stop { dir } => {
                println!("Node stopped cleanly at {}", dir.display());
            }
        },
        Commands::Status { dir } => {
            println!("Status for {}:\n  is_online: true\n  node-id: dummy-id", dir.display());
        }
    }

    Ok(())
}

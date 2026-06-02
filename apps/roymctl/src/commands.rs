//! Roymctl CLI subcommands orchestrator
//!
//! Registers CLI parsing hooks and routes input options to command modules.

use clap::Subcommand;
use std::path::PathBuf;
use syneroym_identity::Identity;

pub mod app;
pub mod identity;
pub mod registry;
pub mod substrate;

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Manage the local substrate daemon
    #[command(alias = "node")]
    Substrate {
        #[command(subcommand)]
        command: substrate::SubstrateCommands,
    },
    /// Manage `SynApps` on the local node
    App {
        #[command(subcommand)]
        command: app::AppCommands,
    },
    /// Manage local cryptographic identities
    Identity {
        #[command(subcommand)]
        command: identity::IdentityCommands,
    },
    /// Compute the 8-character short hash of an input string
    Shorthash {
        /// The input string to hash (e.g. DID or interface name)
        input: String,
    },
    /// Generate a consistent alias for a service ID and optional nickname
    Alias {
        /// The service ID (DID)
        service_id: String,
        /// Optional nickname
        #[arg(long)]
        nickname: Option<String>,
        /// Optional interface name to include in the alias (outputs full hostname)
        #[arg(long)]
        interface: Option<String>,
    },
    /// Manage entries in the community registry
    Registry {
        #[command(subcommand)]
        command: registry::RegistryCommands,
    },
}

/// Execute the subcommands
pub async fn run(
    command: Commands,
    api_url: String,
    substrate_opt: Option<String>,
    dir: PathBuf,
) -> anyhow::Result<()> {
    match command {
        Commands::Substrate { command } => {
            substrate::handle(&command, &dir).await?;
        }
        Commands::App { command } => {
            let substrate_did = substrate_opt.or_else(|| {
                // Try to load local substrate DID from key file if it exists
                let key_path = dir.join("substrate.key");
                Identity::load_from_path(&key_path)
                    .map(|identity| syneroym_identity::substrate::derive_did_key(&identity.public_key()))
                    .ok()
            }).ok_or_else(|| {
                anyhow::anyhow!("Substrate DID not provided and substrate.key not found. Use --substrate <did>")
            })?;

            app::handle(&command, &api_url, substrate_did, &dir).await?;
        }
        Commands::Identity { command } => {
            identity::handle(&command, &dir).await?;
        }
        Commands::Shorthash { input } => {
            let hash = syneroym_core::util::short_hash(&input);
            println!("{hash}");
        }
        Commands::Alias { service_id, nickname, interface } => {
            let alias = syneroym_core::util::generate_alias(nickname.as_deref(), &service_id);
            if let Some(iface) = interface {
                let iface_hash = syneroym_core::util::short_hash(&iface);
                println!("{alias}-i{iface_hash}.localhost");
            } else {
                println!("{alias}");
            }
        }
        Commands::Registry { command } => {
            registry::handle(&command, &api_url, &dir).await?;
        }
    }
    Ok(())
}

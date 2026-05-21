//! Substrate control and launch subcommands
//!
//! Commands to boot up or manage the main substrate execution node.

use clap::Subcommand;
use std::fs;
use std::path::Path;
use syneroym_identity::Identity;

#[derive(Subcommand, Debug, Clone)]
pub enum SubstrateCommands {
    /// Initialize local configuration and default identity for a substrate
    Init,
    /// Get the status/health of the running daemon via API (Placeholder)
    Status,
    /// View configuration of the local substrate via API (Placeholder)
    Config,
}

/// Handle local substrate subcommands
pub async fn handle(command: &SubstrateCommands, dir: &Path) -> anyhow::Result<()> {
    match command {
        SubstrateCommands::Init => {
            if !dir.exists() {
                fs::create_dir_all(dir)?;
            }
            let identity = Identity::generate()?;
            let identity_bytes = identity.to_bytes();
            let key_path = dir.join("identity.key");
            fs::write(&key_path, identity_bytes)?;
            println!("Initialized node local configuration at {}", dir.display());
        }
        SubstrateCommands::Status => {
            // Placeholder: Not yet implemented in SDK
            println!("Node status command is not yet fully implemented with SDK.");
        }
        SubstrateCommands::Config => {
            // Placeholder: Not yet implemented in SDK
            println!("Node config command is not yet fully implemented with SDK.");
        }
    }
    Ok(())
}

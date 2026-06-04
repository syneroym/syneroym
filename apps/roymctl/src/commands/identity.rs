//! Identity management subcommands
//!
//! Commands to generate node keypairs, create agreements, and inspect node
//! DIDs.

use std::{fs, path::Path};

use clap::Subcommand;
use syneroym_identity::{Identity, substrate};

#[derive(Subcommand, Debug, Clone)]
pub enum IdentityCommands {
    /// Create a new identity locally
    Create {
        #[arg(long)]
        name: String,
    },
    /// List locally stored identities
    List,
    /// Show details of a specific identity locally
    Show {
        #[arg(long)]
        name: String,
    },
}

/// Handle local identity subcommands
pub async fn handle(command: &IdentityCommands, dir: &Path) -> anyhow::Result<()> {
    match command {
        IdentityCommands::Create { name } => {
            let identities_dir = dir.join("identities");
            if !identities_dir.exists() {
                fs::create_dir_all(&identities_dir)?;
            }
            let key_path = identities_dir.join(format!("{name}.key"));
            if key_path.exists() {
                anyhow::bail!("Identity '{}' already exists at {}", name, key_path.display());
            }

            let identity = Identity::generate()?;
            identity.save_to_path(&key_path)?;

            let did = substrate::derive_did_key(&identity.public_key());

            println!("Created new local identity: {name}");
            println!("DID: {did}");
            println!("Key stored at: {}", key_path.display());
        }
        IdentityCommands::List => {
            let identities_dir = dir.join("identities");
            if !identities_dir.exists() {
                println!(
                    "No identities found (directory {} does not exist)",
                    identities_dir.display()
                );
                return Ok(());
            }

            println!("{:<20} {:<60}", "NAME", "DID");
            println!("{:-<80}", "");

            for entry in fs::read_dir(identities_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "key")
                    && let Some(name) = path.file_stem().and_then(|s| s.to_str())
                {
                    if let Ok(identity) = Identity::load_from_path(&path) {
                        let did = substrate::derive_did_key(&identity.public_key());
                        println!("{name:<20} {did:<60}");
                    } else {
                        println!("{:<20} {:<60}", name, "[Invalid Key File]");
                    }
                }
            }
        }
        IdentityCommands::Show { name } => {
            let key_path = dir.join("identities").join(format!("{name}.key"));
            if !key_path.exists() {
                anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
            }

            let identity = Identity::load_from_path(&key_path)?;
            let did = substrate::derive_did_key(&identity.public_key());

            println!("Identity: {name}");
            println!("DID:      {did}");
            println!("Path:     {}", key_path.display());
        }
    }
    Ok(())
}

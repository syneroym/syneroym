use clap::Subcommand;
use std::fs;
use std::path::Path;
use syneroym_identity::Identity;

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
            let key_path = identities_dir.join(format!("{}.key", name));
            if key_path.exists() {
                anyhow::bail!("Identity '{}' already exists at {}", name, key_path.display());
            }

            let identity = Identity::generate()?;
            let identity_bytes = identity.to_bytes();
            fs::write(&key_path, identity_bytes)?;

            let did = syneroym_identity::substrate::derive_did_key(&identity.public_key());

            println!("Created new local identity: {}", name);
            println!("DID: {}", did);
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
                    let bytes = fs::read(&path)?;
                    if let Ok(bytes_array) = bytes.try_into() {
                        let identity = Identity::from_bytes(&bytes_array);
                        let did =
                            syneroym_identity::substrate::derive_did_key(&identity.public_key());
                        println!("{:<20} {:<60}", name, did);
                    } else {
                        println!("{:<20} {:<60}", name, "[Invalid Key File]");
                    }
                }
            }
        }
        IdentityCommands::Show { name } => {
            let key_path = dir.join("identities").join(format!("{}.key", name));
            if !key_path.exists() {
                anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
            }

            let bytes = fs::read(&key_path)?;
            let bytes_array: [u8; 32] =
                bytes.try_into().map_err(|_| anyhow::anyhow!("Invalid key file size"))?;
            let identity = Identity::from_bytes(&bytes_array);
            let did = syneroym_identity::substrate::derive_did_key(&identity.public_key());

            println!("Identity: {}", name);
            println!("DID:      {}", did);
            println!("Path:     {}", key_path.display());
        }
    }
    Ok(())
}

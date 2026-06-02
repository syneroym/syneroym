//! Registry management subcommands
//!
//! Commands for querying, registering, and listing endpoints in the community registry.

use clap::Subcommand;
use std::path::Path;
use syneroym_core::community_registry::{EndpointInfo, EndpointType, SignedEndpointInfo};
use syneroym_identity::Identity;

#[derive(Subcommand, Debug, Clone)]
pub enum RegistryCommands {
    /// Register a service DID against a substrate DID
    Register {
        /// The name of the local identity to register (from identities/ directory)
        #[arg(long)]
        identity: String,
        /// The DID of the substrate that hosts this service
        #[arg(long)]
        substrate: String,
        /// Optional nickname for the service
        #[arg(long)]
        nickname: Option<String>,
        /// Make this registration private (will not propagate to parent registries)
        #[arg(long)]
        private: bool,
    },
    /// Look up an entry in the community registry
    Lookup {
        /// The service ID or alias to look up
        service_id: String,
        /// Resolve mechanisms from the substrate (default: true)
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        resolve: bool,
    },
}

/// Handle community registry subcommands
pub async fn handle(command: &RegistryCommands, api_url: &str, dir: &Path) -> anyhow::Result<()> {
    match command {
        RegistryCommands::Register { identity: name, substrate, nickname, private } => {
            let key_path = dir.join("identities").join(format!("{name}.key"));
            if !key_path.exists() {
                anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
            }

            let identity = Identity::load_from_path(&key_path)?;
            let service_id = syneroym_identity::substrate::derive_did_key(&identity.public_key());

            let info = EndpointInfo {
                service_id: service_id.clone(),
                substrate_id: substrate.clone(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![], // Services resolved via substrate don't need mechanisms here
                nickname: nickname.clone(),
                is_private: *private,
                ttl: None,
            };

            let signature = identity.sign_json(&serde_json::to_value(&info)?)?;
            let signed_info = SignedEndpointInfo { info, signature };

            let client = reqwest::Client::new();
            let url = format!("{api_url}/register");
            let response = client.post(&url).json(&signed_info).send().await?;

            if response.status().is_success() {
                println!(
                    "Successfully registered service {service_id} against substrate {substrate}"
                );
                if let Some(n) = nickname {
                    let alias = syneroym_core::util::generate_alias(Some(n), &service_id);
                    println!("Alias: {alias}");
                }
            } else {
                let error_text = response.text().await?;
                anyhow::bail!("Registry registration failed ({url}): {error_text}");
            }
        }
        RegistryCommands::Lookup { service_id, resolve } => {
            let client = reqwest::Client::new();
            let url = format!("{api_url}/lookup/{service_id}?resolve={resolve}");
            let response = client.get(&url).send().await?;

            if response.status().is_success() {
                let signed_info: SignedEndpointInfo = response.json().await?;
                println!("{signed_info:#?}");
            } else {
                anyhow::bail!("Registry lookup failed ({}): {}", url, response.status());
            }
        }
    }
    Ok(())
}

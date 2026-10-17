//! Registry management subcommands
//!
//! Commands for querying, registering, and listing endpoints in the community
//! registry.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{ArgAction, Subcommand};
use reqwest::{Client, StatusCode};
use syneroym_core::{
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient,
        SignedEndpointInfo,
    },
    util,
};
use syneroym_identity::{Identity, substrate};

#[derive(Subcommand, Debug, Clone)]
pub enum RegistryCommands {
    /// Register a service DID against a substrate DID. The record this signs
    /// has no mailbox. It refuses a service whose current record names one:
    /// the record of a service with a mailbox is published by its deploy.
    Register {
        /// The name of the local identity to register (from identities/
        /// directory)
        #[arg(long)]
        identity: String,
        /// The DID of the substrate that hosts this service
        #[arg(long)]
        substrate: String,
        /// Optional nickname for the service
        #[arg(long)]
        nickname: Option<String>,
        /// Make this registration private (will not propagate to parent
        /// registries)
        #[arg(long)]
        private: bool,
    },
    /// Look up an entry in the community registry
    Lookup {
        /// The service ID or alias to look up
        service_id: String,
        /// Resolve mechanisms from the substrate (default: true)
        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        resolve: bool,
        /// Also query the global P2P DHT (BEP 0044)
        #[arg(long)]
        dht: bool,
    },
}

/// The record the registry holds for `service_id` now, if any.
async fn current_record(
    client: &Client,
    api_url: &str,
    service_id: &str,
) -> anyhow::Result<Option<SignedEndpointInfo>> {
    let response = client.get(format!("{api_url}/lookup/{service_id}")).send().await?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        anyhow::bail!("registry returned {status} for the current record of {service_id}");
    }
    Ok(Some(response.json::<SignedEndpointInfo>().await?))
}

/// A fresh record from this command has no mailbox, so on a service that
/// has one it would remove the mailbox for senders while the service still
/// pulls from it. A record that is for another service or does not verify is
/// ignored: a wrong or hostile registry must not be able to block or allow
/// this by lying.
fn refuse_if_mailbox(service_id: &str, current: Option<&SignedEndpointInfo>) -> anyhow::Result<()> {
    let names_a_mailbox = current.is_some_and(|record| {
        record.verify().is_ok()
            && record.info.service_id == service_id
            && record.info.mailbox.is_some()
    });
    if names_a_mailbox {
        anyhow::bail!(
            "service {service_id} has a mailbox; its record is published by its deploy (svc \
             deploy, app deploy or the supervisor). Registering here would remove the mailbox for \
             senders while the service still pulls from it. Redeploy the service to change its \
             record."
        );
    }
    Ok(())
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
            let service_id = substrate::derive_did_key(&identity.public_key());

            let not_after = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

            let info = EndpointInfo {
                service_id: service_id.clone(),
                substrate_id: substrate.clone(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![], // Services resolved via substrate don't need mechanisms here
                nickname: nickname.clone(),
                is_private: *private,
                ttl: None,
                not_after,
                generation: 0,
                mailbox: None,
            };

            let signed_info = info.sign(&identity)?;

            let client = Client::new();
            refuse_if_mailbox(
                &service_id,
                current_record(&client, api_url, &service_id).await?.as_ref(),
            )?;
            let url = format!("{api_url}/register");
            let response = client.post(&url).json(&signed_info).send().await?;

            if response.status().is_success() {
                println!(
                    "Successfully registered service {service_id} against substrate {substrate}"
                );
                if let Some(n) = nickname {
                    let alias = util::generate_alias(Some(n), &service_id);
                    println!("Alias: {alias}");
                }
            } else {
                let error_text = response.text().await?;
                anyhow::bail!("Registry registration failed ({url}): {error_text}");
            }
        }
        RegistryCommands::Lookup { service_id, resolve, dht } => {
            let registry_client = RegistryClient::new(*dht, Some(api_url.to_string()));
            match registry_client.lookup(service_id, *resolve).await {
                Ok(signed_info) => {
                    println!("{signed_info:#?}");
                }
                Err(e) => {
                    anyhow::bail!("Registry lookup failed for {service_id}: {e}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

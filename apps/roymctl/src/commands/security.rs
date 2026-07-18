use std::{
    io::{self, Read},
    path::Path,
    time::Duration,
};

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum KekCommands {
    /// Inject a Key Encryption Key (KEK) into the running substrate daemon
    Inject {
        /// 32-byte hex-encoded KEK
        kek_hex: String,
    },
    /// Rotate the active Key Encryption Key (KEK)
    Rotate {
        /// 32-byte hex-encoded new KEK
        new_kek_hex: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum SecretCommands {
    /// Set a secret value in a service's private vault
    Set {
        /// The service ID (DID) of the target service
        service_id: String,
        /// The secret key (name)
        key: String,
    },
}

pub async fn handle_kek(
    command: &KekCommands,
    api_url: &str,
    substrate_did: String,
    dir: &Path,
    run_as: Option<&str>,
) -> anyhow::Result<()> {
    let mut client = super::client_for(substrate_did, api_url, dir, run_as)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        KekCommands::Inject { kek_hex } => {
            client.inject_kek(kek_hex.clone()).await?;
            println!("KEK successfully injected");
        }
        KekCommands::Rotate { new_kek_hex } => {
            client.rotate_kek(new_kek_hex.clone()).await?;
            println!("KEK successfully rotated");
        }
    }
    Ok(())
}

pub async fn handle_secret(
    command: &SecretCommands,
    api_url: &str,
    substrate_did: String,
    dir: &Path,
    run_as: Option<&str>,
) -> anyhow::Result<()> {
    let mut client = super::client_for(substrate_did, api_url, dir, run_as)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        SecretCommands::Set { service_id, key } => {
            let mut value = Vec::new();
            io::stdin().read_to_end(&mut value)?;
            client.set_secret(service_id.clone(), key.clone(), value).await?;
            println!("Secret '{}' set successfully for service {}", key, service_id);
        }
    }
    Ok(())
}

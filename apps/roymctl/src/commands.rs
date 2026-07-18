//! Roymctl CLI subcommands orchestrator
//!
//! Registers CLI parsing hooks and routes input options to command modules.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use app::AppCommands;
use clap::Subcommand;
use identity::IdentityCommands;
use registry::RegistryCommands;
use substrate::SubstrateCommands;
use svc::SvcCommands;
use syneroym_core::util;
use syneroym_identity::{Identity, substrate as identity_substrate};
use syneroym_sdk::SyneroymClient;
use syneroym_ucan::CapabilityToken;

pub mod app;
pub mod identity;
pub mod registry;
pub mod security;
pub mod substrate;
pub mod svc;

use security::{KekCommands, SecretCommands};

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Manage the local substrate daemon
    #[command(alias = "node")]
    Substrate {
        #[command(subcommand)]
        command: SubstrateCommands,
    },
    /// Manage `SynSvcs` on the local node
    Svc {
        #[command(subcommand)]
        command: SvcCommands,
    },
    /// Manage `SynApps` on the local node
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    /// Manage local cryptographic identities
    Identity {
        #[command(subcommand)]
        command: IdentityCommands,
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
        /// Optional interface name to include in the alias (outputs full
        /// hostname)
        #[arg(long)]
        interface: Option<String>,
    },
    /// Manage entries in the community registry
    Registry {
        #[command(subcommand)]
        command: RegistryCommands,
    },
    /// Manage the Key Encryption Key (KEK)
    Kek {
        #[command(subcommand)]
        command: KekCommands,
    },
    /// Manage vault secrets
    Secret {
        #[command(subcommand)]
        command: SecretCommands,
    },
}

fn get_substrate_did(substrate_opt: Option<String>, dir: &Path) -> anyhow::Result<String> {
    substrate_opt
        .or_else(|| {
            // Try to load local substrate DID from key file if it exists
            let key_path = dir.join("substrate.key");
            Identity::load_from_path(&key_path)
                .map(|identity| identity_substrate::derive_did_key(&identity.public_key()))
                .ok()
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Substrate DID not provided and substrate.key not found. Use --substrate <did>"
            )
        })
}

/// Build a client acting as `--as <name>` if given, else with the ephemeral
/// key `SyneroymClient::new` generates (today's behavior) -- M04A Slice B7a,
/// F5. Distinct from `svc deploy --identity`, which names the app's own
/// signing key for its registry certificate, not the operator. If `--ucan
/// <path>` names a signed `CapabilityToken` JSON file (M04A Slice B7b,
/// `roymctl identity issue-grant`'s output), it is read, parsed, and
/// presented via `with_ucan` -- on top of whichever transport identity `--as`
/// selected.
///
/// `--ucan` requires `--as` (post-commit review, F2): the token's
/// `audience_did` must equal the connection's verified master DID
/// (`from_verified_chain`'s audience check,
/// `crates/router/src/route_handler/io.rs`), and without `--as` that DID is a
/// fresh ephemeral key `SyneroymClient::new` generates per invocation --
/// never the grant's `--to`. The mismatch fails only on the server side (a
/// `warn!`-logged chain drop, not a client-visible error), so the caller
/// silently falls back to `AuthLevel::Delegated` and sees a confusing
/// "holds no grant" error downstream instead of the real cause. Rejected
/// here instead, before any connection is attempted.
pub(crate) fn client_for(
    substrate_did: String,
    api_url: &str,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<SyneroymClient> {
    if ucan_path.is_some() && run_as.is_none() {
        anyhow::bail!(
            "--ucan requires --as <name>: the presented token's audience must match the \
             connecting identity, and without --as that identity is a fresh ephemeral key that \
             can never match. Pass --as <name>, where <name> is the identity the grant's --to \
             names."
        );
    }
    let client = match run_as {
        None => SyneroymClient::new(substrate_did, api_url.to_string()),
        Some(name) => {
            let path = dir.join("identities").join(format!("{name}.key"));
            let id = Identity::load_from_path(&path)
                .with_context(|| format!("no local identity '{name}' at {}", path.display()))?;
            SyneroymClient::new_with_identity(substrate_did, api_url.to_string(), id)
        }
    };
    match ucan_path {
        None => Ok(client),
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
            let token: CapabilityToken = serde_json::from_str(&raw)
                .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))?;
            Ok(client.with_ucan(token))
        }
    }
}

/// Execute the subcommands
pub async fn run(
    command: Commands,
    api_url: String,
    substrate_opt: Option<String>,
    dir: PathBuf,
    run_as: Option<String>,
    ucan_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    match command {
        Commands::Substrate { command } => {
            substrate::handle(&command, &dir).await?;
        }
        Commands::Svc { command } => {
            let substrate_did = get_substrate_did(substrate_opt, &dir)?;
            svc::handle(
                &command,
                &api_url,
                substrate_did,
                &dir,
                run_as.as_deref(),
                ucan_path.as_deref(),
            )
            .await?;
        }
        Commands::App { command } => {
            let substrate_did = get_substrate_did(substrate_opt, &dir)?;
            app::handle(
                &command,
                &api_url,
                substrate_did,
                &dir,
                run_as.as_deref(),
                ucan_path.as_deref(),
            )
            .await?;
        }
        Commands::Identity { command } => {
            identity::handle(&command, &dir).await?;
        }
        Commands::Shorthash { input } => {
            let hash = util::short_hash(&input);
            println!("{hash}");
        }
        Commands::Alias { service_id, nickname, interface } => {
            let alias = util::generate_alias(nickname.as_deref(), &service_id);
            if let Some(iface) = interface {
                let iface_hash = util::short_hash(&iface);
                println!("{alias}-i{iface_hash}.localhost");
            } else {
                println!("{alias}");
            }
        }
        Commands::Registry { command } => {
            registry::handle(&command, &api_url, &dir).await?;
        }
        Commands::Kek { command } => {
            let substrate_did = get_substrate_did(substrate_opt, &dir)?;
            security::handle_kek(
                &command,
                &api_url,
                substrate_did,
                &dir,
                run_as.as_deref(),
                ucan_path.as_deref(),
            )
            .await?;
        }
        Commands::Secret { command } => {
            let substrate_did = get_substrate_did(substrate_opt, &dir)?;
            security::handle_secret(
                &command,
                &api_url,
                substrate_did,
                &dir,
                run_as.as_deref(),
                ucan_path.as_deref(),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Post-commit review (F2): `client_for` rejects `--ucan` without `--as`
    /// directly, not just via clap's `requires` on the CLI's global flags --
    /// a direct caller within the crate would otherwise hit the confusing
    /// downstream "holds no grant" failure instead of a clear cause.
    #[test]
    fn client_for_rejects_ucan_without_as() {
        let dir = tempfile::tempdir().unwrap();
        let result = client_for(
            "did:key:zSomeSubstrate".to_string(),
            "http://localhost:7961",
            dir.path(),
            None,
            Some(Path::new("/does/not/matter.json")),
        );
        let err = result.expect_err("--ucan without --as must be rejected");
        assert!(err.to_string().contains("--as"), "error must point at --as: {err}");
    }
}

//! Identity management subcommands
//!
//! Commands to generate node keypairs, create agreements, and inspect node
//! DIDs.

use std::{fs, path::Path, time::Duration};

use anyhow::Context;
use clap::Subcommand;
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::deploy;
use syneroym_ucan::{Ability, Capability, CapabilityToken, ResourceUri};

use super::member_identity;

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
    /// Issue a new DelegationCertificate
    Delegate {
        #[arg(long)]
        master: String,
        #[arg(long)]
        temp_did: String,
        #[arg(long)]
        expires_days: u64,
        #[arg(long)]
        scope: String,
    },
    /// Publish MasterAnchorPayload to the community registry
    PublishAnchor {
        #[arg(long)]
        master: String,
        #[arg(long)]
        registry_url: String,
    },
    /// Issue a UCAN `CapabilityToken` granting an ability to another DID --
    /// e.g. the substrate owner granting `orchestrator/deploy` on
    /// `substrate:<node>/app/*` to an operator. Prints the signed token as
    /// JSON; present it with the global `--ucan <path>` flag.
    IssueGrant {
        /// Name of the locally-stored identity issuing the grant (the root
        /// of trust for `--with`'s resource -- e.g. the substrate owner, or
        /// a service's own recorded owner).
        #[arg(long)]
        from: String,
        /// DID of the grantee (the token's audience).
        #[arg(long)]
        to: String,
        /// The ability to grant, e.g. `orchestrator/deploy`.
        #[arg(long)]
        can: String,
        /// The resource to grant it on, e.g. `substrate:<node_did>/app/*`.
        #[arg(long)]
        with: String,
        #[arg(long)]
        expires_days: u64,
        /// Forbid the grantee from further delegating this capability
        /// (ADR-0015 A3 `can_delegate: false`). Absent defaults to
        /// delegable.
        #[arg(long)]
        no_delegate: bool,
    },
    /// Certify the instance key a substrate would derive for a member
    /// master, so that master can be presented on the deployed service's
    /// outbound calls (ADR-0020 §1). The primitive `--master`/renewal
    /// command: queries `--substrate` for the instance key it would derive
    /// under this operator's identity for the master's own DID, then signs a
    /// `service-instance`-scoped certificate over it and prints the
    /// certificate JSON -- pass it to `svc deploy --instance-certificate` or
    /// re-deploy with `svc deploy --master` to install it.
    ///
    /// `identity delegate` cannot do this job: it requires `--temp-did`,
    /// which is exactly what the operator does not have until the substrate
    /// reports it.
    ///
    /// Queries `--substrate` over `orchestrator/resolve-instance-identity`
    /// (gated on `orchestrator/status`), so it needs an
    /// operator identity the substrate authorizes -- pass `--as <name>` (or
    /// `--ucan <token>` covering this app) once the substrate is claimed.
    /// On an unowned substrate this command is denied outright.
    CertifyInstance {
        /// Name of the local member master identity issuing the
        /// certificate. The certificate always names this identity's own
        /// resolved DID as the service_id being certified -- there is no
        /// other value it could validly hold, so it is derived rather than
        /// taken as a separate flag.
        #[arg(long)]
        master: String,
        /// DID of the substrate to query for its derived instance key.
        #[arg(long)]
        substrate: String,
        #[arg(long, default_value_t = 24)]
        expires_hours: u64,
        /// Community registry URL to publish/refresh this master's anchor at
        /// (D-A1-7). Without it, the certificate this command mints is
        /// unusable on the wire until an anchor exists some other way
        /// (`roymctl identity publish-anchor`).
        #[arg(long)]
        registry_url: Option<String>,
    },
    /// Certify the node's record-signing key for a service owner (M06C Slice
    /// C3).
    CertifySigning {
        #[arg(long)]
        master: String,
        #[arg(long)]
        substrate: String,
        #[arg(long, default_value_t = 24)]
        expires_hours: u64,
    },
}

/// Handle local identity subcommands
pub async fn handle(
    command: &IdentityCommands,
    api_url: &str,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
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
        IdentityCommands::Delegate { master, temp_did, expires_days, scope } => {
            let key_path = dir.join("identities").join(format!("{master}.key"));
            if !key_path.exists() {
                anyhow::bail!("Master identity '{}' not found at {}", master, key_path.display());
            }
            let identity = Identity::load_from_path(&key_path)?;

            let temp_pubkey =
                substrate::resolve_did_key(temp_did).context("Failed to resolve temporary DID")?;

            let cert = DelegationCertificate::issue(
                &identity,
                temp_pubkey,
                expires_days * 24 * 3600,
                scope.clone(),
            )?;
            println!("{}", cert.to_json()?);
        }
        IdentityCommands::PublishAnchor { master, registry_url } => {
            let key_path = dir.join("identities").join(format!("{master}.key"));
            if !key_path.exists() {
                anyhow::bail!("Master identity '{}' not found at {}", master, key_path.display());
            }
            let identity = Identity::load_from_path(&key_path)?;

            let client = RegistryClient::new(true, Some(registry_url.clone()));
            let master_id = substrate::derive_did_key(&identity.public_key());

            client.publish_master_anchor(&master_id, vec![], None, &identity, true).await?;
            println!("Successfully published MasterAnchorPayload to {}", registry_url);
        }
        IdentityCommands::IssueGrant { from, to, can, with, expires_days, no_delegate } => {
            let key_path = dir.join("identities").join(format!("{from}.key"));
            if !key_path.exists() {
                anyhow::bail!("Identity '{}' not found at {}", from, key_path.display());
            }
            let issuer = Identity::load_from_path(&key_path)?;

            let caveats = no_delegate.then(|| serde_json::json!({"can_delegate": false}));
            let capability =
                Capability { with: ResourceUri(with.clone()), can: Ability(can.clone()), caveats };

            let token = CapabilityToken::issue(
                &issuer,
                to,
                vec![capability],
                serde_json::Map::new(),
                expires_days * 24 * 3600,
                vec![],
            )?;
            println!("{}", serde_json::to_string_pretty(&token)?);
        }
        IdentityCommands::CertifyInstance {
            master,
            substrate: substrate_did,
            expires_hours,
            registry_url,
        } => {
            let master_identity = member_identity::resolve_member_master(dir, master)?;
            let service_id = substrate::derive_did_key(&master_identity.public_key());

            let mut client =
                super::client_for(substrate_did.clone(), api_url, dir, run_as, ucan_path)?;
            client.wait_for_ready(Duration::from_secs(5)).await?;

            let cert =
                deploy::certify_instance(&client, &master_identity, &service_id, *expires_hours)
                    .await?;
            member_identity::refresh_anchor_or_warn(registry_url.as_deref(), &master_identity)
                .await?;
            println!("{}", cert.to_json()?);
        }
        IdentityCommands::CertifySigning { master, substrate: substrate_did, expires_hours } => {
            let master_identity = member_identity::resolve_member_master(dir, master)?;
            let service_id = substrate::derive_did_key(&master_identity.public_key());

            let mut client =
                super::client_for(substrate_did.clone(), api_url, dir, run_as, ucan_path)?;
            client.wait_for_ready(Duration::from_secs(5)).await?;

            let cert = deploy::certify_record_signing(
                &client,
                &master_identity,
                &service_id,
                *expires_hours,
            )
            .await?;
            println!("{}", cert.to_json()?);
        }
    }
    Ok(())
}

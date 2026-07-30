//! Substrate control and launch subcommands
//!
//! Commands to boot up or manage the main substrate execution node.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::Subcommand;
use syneroym_core::config::{DEFAULT_CONTROLLER_AGREEMENT_FILE, DEFAULT_SUBSTRATE_KEY_FILE};
use syneroym_identity::{
    Identity,
    substrate::{ControllerAgreement, SubstrateIdentityState, SubstrateIdentityStatus},
};

#[derive(Subcommand, Debug, Clone)]
pub enum SubstrateCommands {
    /// Initialize local configuration and default identity for a substrate
    Init {
        /// Overwrite an existing substrate key. This is the node's identity
        /// -- replacing it orphans every service-owner row, installed
        /// instance certificate, and endpoint record keyed to the old DID,
        /// and invalidates any `agreement.json` already claiming this node.
        #[arg(long)]
        force: bool,
    },
    /// Establish ownership of this substrate: mint a mutually-signed
    /// `ControllerAgreement` binding the node's DID to a controller identity
    /// you hold. Must run on the substrate host -- it reads the node's own
    /// private key, which never leaves that filesystem.
    ///
    /// The substrate picks the agreement up on its next start (from
    /// `<dir>/agreement.json`, or from `[identity].agreement` if set), and
    /// from then on only the controller holds `substrate/admin`: deploy,
    /// undeploy, status, and the `security` interface (KEK/secrets).
    Claim {
        /// Local identity that becomes the controller (see `roymctl identity
        /// create --name`). Must not be the node's own key.
        #[arg(long)]
        controller: String,
        /// The node's private key. Defaults to `<dir>/substrate.key`.
        #[arg(long)]
        substrate_key: Option<PathBuf>,
        /// Where to write the agreement. Defaults to `<dir>/agreement.json`,
        /// which the substrate discovers with no config change.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Expiry in days. Omitted means no expiry. Checked only at boot --
        /// an agreement that expires while the substrate is running keeps
        /// granting `substrate/admin` until the next restart.
        #[arg(long)]
        expires_days: Option<u64>,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Get the status/health of the running daemon via API (Placeholder)
    Status,
    /// View configuration of the local substrate via API (Placeholder)
    Config,
}

/// Mint and write a `ControllerAgreement`. Factored out of the clap arm so
/// tests can drive it directly without shelling out.
fn claim(
    dir: &Path,
    controller: &str,
    substrate_key: Option<PathBuf>,
    out: Option<PathBuf>,
    expires_days: Option<u64>,
    force: bool,
) -> anyhow::Result<(ControllerAgreement, PathBuf)> {
    let node_key_path = substrate_key.unwrap_or_else(|| dir.join(DEFAULT_SUBSTRATE_KEY_FILE));
    if !node_key_path.exists() {
        bail!(
            "no substrate key at {}. Run `roymctl substrate init --dir {}` first, or pass \
             --substrate-key <path>. `claim` must run on the substrate host: it signs with the \
             node's own key.",
            node_key_path.display(),
            dir.display()
        );
    }

    let controller_path = dir.join("identities").join(format!("{controller}.key"));
    if !controller_path.exists() {
        bail!(
            "no local identity '{controller}' at {}. Create one with `roymctl identity create \
             --name {controller}`.",
            controller_path.display()
        );
    }

    let node = Identity::load_from_path(&node_key_path)
        .with_context(|| format!("failed to load substrate key at {}", node_key_path.display()))?;
    let controller_identity = Identity::load_from_path(&controller_path).with_context(|| {
        format!("failed to load controller identity at {}", controller_path.display())
    })?;

    let out_path = out.unwrap_or_else(|| dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE));
    if out_path.exists() && !force {
        bail!(
            "{} already exists; pass --force to replace it. Replacing an agreement transfers \
             ownership on the next substrate start.",
            out_path.display()
        );
    }

    let expires_in_secs = expires_days
        .map(|d| {
            d.checked_mul(24 * 3600).ok_or_else(|| {
                anyhow::anyhow!("--expires-days {d} is too large to convert to seconds")
            })
        })
        .transpose()?;
    let agreement = ControllerAgreement::issue(&node, &controller_identity, expires_in_secs)?;

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out_path, serde_json::to_string_pretty(&agreement)?)
        .with_context(|| format!("failed to write agreement to {}", out_path.display()))?;

    // Read back what was actually written and verify it, rather than trust
    // the in-memory value: a serde round-trip bug or a future field rename
    // would otherwise print "claimed" for a node that still boots unowned.
    let written = fs::read_to_string(&out_path)
        .with_context(|| format!("failed to read back agreement at {}", out_path.display()))?;
    let reloaded = ControllerAgreement::from_json(&written).with_context(|| {
        format!("agreement written to {} does not round-trip", out_path.display())
    })?;
    let state = SubstrateIdentityState::init(&node, Some(&reloaded), None, true)
        .context("the agreement just written does not verify")?;
    if state.status != SubstrateIdentityStatus::Verified {
        bail!(
            "the agreement written to {} does not verify (status: {:?})",
            out_path.display(),
            state.status
        );
    }

    Ok((agreement, out_path))
}

/// Handle local substrate subcommands
pub async fn handle(command: &SubstrateCommands, dir: &Path) -> anyhow::Result<()> {
    match command {
        SubstrateCommands::Init { force } => {
            if !dir.exists() {
                fs::create_dir_all(dir)?;
            }
            let key_path = dir.join(DEFAULT_SUBSTRATE_KEY_FILE);
            if key_path.exists() && !force {
                bail!(
                    "{} already exists; pass --force to replace it. This is the node's identity \
                     -- replacing it orphans everything attributed to the old DID and invalidates \
                     any agreement.json already claiming this node.",
                    key_path.display()
                );
            }
            let identity = Identity::generate()?;
            identity.save_to_path(&key_path)?;
            println!("Initialized node local configuration at {}", dir.display());
            println!("  substrate key: {}", key_path.display());
        }
        SubstrateCommands::Claim { controller, substrate_key, out, expires_days, force } => {
            let (agreement, out_path) =
                claim(dir, controller, substrate_key.clone(), out.clone(), *expires_days, *force)?;

            println!("Substrate claimed.");
            println!("  node DID:       {}", agreement.controlled);
            println!("  controller DID: {}", agreement.controller);
            println!("  agreement:      {}", out_path.display());
            println!();
            println!(
                "Restart the substrate to apply -- it reads the agreement once, at boot. It is \
                 picked up from {} automatically; a substrate started with a different data \
                 directory can be pointed at it with `syneroym-substrate run --agreement {}`. \
                 From then on, control it with `roymctl --as {controller} ...`.",
                out_path.display(),
                out_path.display()
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn init_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let node = Identity::generate().unwrap();
        fs::write(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE), node.to_bytes()).unwrap();
        fs::create_dir_all(dir.path().join("identities")).unwrap();
        let controller = Identity::generate().unwrap();
        fs::write(dir.path().join("identities").join("owner.key"), controller.to_bytes()).unwrap();
        dir
    }

    #[test]
    fn claim_writes_a_verifiable_agreement() {
        let dir = init_dir();
        let (_agreement, out_path) = claim(dir.path(), "owner", None, None, None, false).unwrap();

        // Read the bytes actually on disk, not the in-memory value `claim`
        // returned -- this is what exercises the serde round trip (field
        // renames, `to_string_pretty`) rather than just the in-process path.
        let written = fs::read_to_string(&out_path).unwrap();
        let reloaded = ControllerAgreement::from_json(&written).unwrap();

        let node = Identity::load_from_path(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE)).unwrap();
        let state = SubstrateIdentityState::init(&node, Some(&reloaded), None, true).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::Verified);
    }

    #[test]
    fn claim_refuses_to_overwrite_without_force() {
        let dir = init_dir();
        claim(dir.path(), "owner", None, None, None, false).unwrap();

        let err = claim(dir.path(), "owner", None, None, None, false).unwrap_err();
        assert!(err.to_string().contains("--force"));

        // --force does overwrite.
        claim(dir.path(), "owner", None, None, None, true).unwrap();
    }

    #[test]
    fn claim_reports_a_missing_substrate_key_with_the_init_hint() {
        let dir = tempfile::tempdir().unwrap();
        let err = claim(dir.path(), "owner", None, None, None, false).unwrap_err();
        assert!(err.to_string().contains("substrate init"));
    }

    #[test]
    fn claim_rejects_an_expiry_too_large_to_convert_to_seconds_instead_of_panicking() {
        let dir = init_dir();
        let err = claim(dir.path(), "owner", None, None, Some(u64::MAX), false).unwrap_err();
        assert!(err.to_string().contains("too large"));
    }
}

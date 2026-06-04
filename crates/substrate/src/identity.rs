//! Substrate boot-time identity setup and verification
//!
//! Loads cryptographic keyfiles, resolves agreements, and initializes verified
//! controller states during runtime boot.

use std::{fs, path::Path};

use syneroym_core::config::{DEFAULT_SUBSTRATE_KEY_FILE, IdentityConfig};
use syneroym_identity::{
    Identity,
    substrate::{ControllerAgreement, SubstrateIdentityState},
};
use tracing::info;

/// Setup and initialize the substrate's identity and controller state.
pub fn setup_substrate_identity(
    config: &IdentityConfig,
    app_data_dir: &Path,
) -> anyhow::Result<SubstrateIdentityState> {
    let key_path =
        config.key.clone().unwrap_or_else(|| app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    // Ensure the directory for the key exists
    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Load or generate substrate identity
    let substrate_identity = if key_path.exists() {
        Identity::load_from_path(&key_path)?
    } else {
        let id = Identity::generate()?;
        id.save_to_path(&key_path)?;
        id
    };

    // Load agreement if path is provided and exists
    let agreement = if let Some(ref path) = config.agreement {
        if path.exists() {
            let json = fs::read_to_string(path)?;
            Some(ControllerAgreement::from_json(&json)?)
        } else {
            None
        }
    } else {
        None
    };

    // Initialize substrate state
    let substrate_identity_state = SubstrateIdentityState::init(
        &substrate_identity,
        agreement.as_ref(),
        config.controller_did.as_deref(),
        config.require_agreement,
    )?;

    info!(
        did = %substrate_identity_state.did,
        controller = ?substrate_identity_state.controller,
        status = ?substrate_identity_state.status,
        "substrate identity initialized"
    );

    Ok(substrate_identity_state)
}

pub fn get_secret(config: &IdentityConfig, app_data_dir: &Path) -> anyhow::Result<[u8; 32]> {
    let key_path =
        config.key.clone().unwrap_or_else(|| app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    let identity = Identity::load_from_path(&key_path)?;
    Ok(identity.to_bytes())
}

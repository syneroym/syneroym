use std::fs;
use syneroym_core::config::{DEFAULT_SUBSTRATE_KEY_FILE, IdentityConfig};
use syneroym_identity::Identity;
use syneroym_identity::substrate::{ControllerAgreement, SubstrateIdentityState};
use tracing::info;

/// Setup and initialize the substrate's identity and controller state.
pub fn setup_substrate_identity(
    config: &IdentityConfig,
    app_data_dir: &std::path::Path,
) -> anyhow::Result<SubstrateIdentityState> {
    let key_path =
        config.key.clone().unwrap_or_else(|| app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    // Ensure the directory for the key exists
    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Load or generate substrate identity
    let substrate_identity = if key_path.exists() {
        let key_bytes = fs::read(&key_path)?;
        let mut key_arr = [0u8; 32];
        let copy_len = std::cmp::min(key_bytes.len(), 32);
        key_arr[..copy_len].copy_from_slice(&key_bytes[..copy_len]);
        Identity::from_bytes(&key_arr)
    } else {
        let id = Identity::generate()?;
        fs::write(&key_path, id.to_bytes())?;
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

pub fn get_secret(
    config: &IdentityConfig,
    app_data_dir: &std::path::Path,
) -> anyhow::Result<[u8; 32]> {
    let key_path =
        config.key.clone().unwrap_or_else(|| app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    let key_bytes = fs::read(&key_path)?;
    let mut key_arr = [0u8; 32];
    let copy_len = std::cmp::min(key_bytes.len(), 32);
    key_arr[..copy_len].copy_from_slice(&key_bytes[..copy_len]);

    Ok(key_arr)
}

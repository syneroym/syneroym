//! Substrate boot-time identity setup and verification
//!
//! Loads cryptographic keyfiles, resolves agreements, and initializes verified
//! controller states during runtime boot.

use std::{fs, path::Path};

use anyhow::Context;
use syneroym_core::config::{
    DEFAULT_CONTROLLER_AGREEMENT_FILE, DEFAULT_SUBSTRATE_KEY_FILE, IdentityConfig,
};
use syneroym_identity::{
    Identity,
    substrate::{ControllerAgreement, SubstrateIdentityState},
};
use tracing::{info, warn};

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

    // An explicit `[identity].agreement` wins; otherwise the substrate picks
    // up `<app_data_dir>/agreement.json` if it exists -- `roymctl substrate
    // claim`'s default output, so claim-then-restart establishes ownership
    // with no config edit.
    let agreement_path = config
        .agreement
        .clone()
        .unwrap_or_else(|| app_data_dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE));

    let agreement = if agreement_path.exists() {
        // The `[identity].agreement`/`controller_did` exclusivity check in
        // `main.rs` only sees the *configured* agreement path, which stays
        // `None` on the discovery route below -- so a discovered
        // `agreement.json` can silently outrank a configured
        // `controller_did` with no warning anywhere. Harmless in practice
        // (`controller_did` alone never becomes an admin root), but worth
        // one line so the boot log does not name a controller nobody
        // configured with no explanation.
        if config.agreement.is_none() && config.controller_did.is_some() {
            warn!(
                agreement = %agreement_path.display(),
                controller_did = ?config.controller_did,
                "a discovered controller agreement supersedes the configured controller_did; the \
                 latter is ignored"
            );
        }

        let json = fs::read_to_string(&agreement_path).with_context(|| {
            format!("failed to read controller agreement at {}", agreement_path.display())
        })?;
        // A present-but-unparseable agreement is a hard failure on both the
        // explicit and the discovered path. Booting unowned because the
        // ownership artifact was malformed is the exact silent failure this
        // slice removes.
        Some(ControllerAgreement::from_json(&json).with_context(|| {
            format!("invalid controller agreement at {}", agreement_path.display())
        })?)
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

#[cfg(test)]
mod tests {
    use syneroym_identity::substrate::{SubstrateIdentityStatus, derive_did_key};
    use tempfile::TempDir;

    use super::*;

    fn no_agreement_config() -> IdentityConfig {
        IdentityConfig {
            key: None,
            controller_did: None,
            agreement: None,
            require_agreement: false,
            nickname: None,
        }
    }

    #[test]
    fn a_discovered_agreement_is_loaded_without_config() {
        let dir = TempDir::new().unwrap();
        let node = Identity::generate().unwrap();
        node.save_to_path(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE)).unwrap();
        let controller = Identity::generate().unwrap();
        let agreement = ControllerAgreement::issue(&node, &controller, None).unwrap();
        fs::write(
            dir.path().join(DEFAULT_CONTROLLER_AGREEMENT_FILE),
            serde_json::to_string(&agreement).unwrap(),
        )
        .unwrap();

        let state = setup_substrate_identity(&no_agreement_config(), dir.path()).unwrap();

        assert_eq!(state.status, SubstrateIdentityStatus::Verified);
        assert_eq!(state.controller, Some(derive_did_key(&controller.public_key())));
    }

    #[test]
    fn a_malformed_discovered_agreement_fails_the_boot() {
        let dir = TempDir::new().unwrap();
        let node = Identity::generate().unwrap();
        node.save_to_path(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE)).unwrap();
        fs::write(dir.path().join(DEFAULT_CONTROLLER_AGREEMENT_FILE), "not json").unwrap();

        let err = setup_substrate_identity(&no_agreement_config(), dir.path()).unwrap_err();
        assert!(err.to_string().contains("invalid controller agreement"));
    }
}

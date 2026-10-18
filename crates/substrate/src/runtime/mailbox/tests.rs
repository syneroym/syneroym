use syneroym_core::config::SubstrateConfig;

use super::require_registry;

#[test]
fn the_mailbox_role_refuses_to_start_without_a_registry() {
    let mut config = SubstrateConfig::default();
    config.substrate.registry_url = None;
    config.substrate.enable_bep0044_dht = false;

    let err = require_registry(&config).unwrap_err().to_string();
    assert!(err.contains("substrate.registry_url"));
    assert!(err.contains("substrate.enable_bep0044_dht"));
}

#[test]
fn a_registry_url_or_the_dht_is_enough() {
    let mut config = SubstrateConfig::default();
    config.substrate.registry_url = Some("http://localhost:8080".to_string());
    config.substrate.enable_bep0044_dht = false;
    assert!(require_registry(&config).is_ok());

    config.substrate.registry_url = None;
    config.substrate.enable_bep0044_dht = true;
    assert!(require_registry(&config).is_ok());

    config.substrate.registry_url = Some("http://localhost:8080".to_string());
    config.substrate.enable_bep0044_dht = true;
    assert!(require_registry(&config).is_ok());
}

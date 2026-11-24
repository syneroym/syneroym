use super::*;
use crate::wake::DEFAULT_WAKE_TIMEOUT_MS;

#[test]
fn a_manifest_with_a_wake_round_trips() {
    let toml_str = r#"
            id = "syneroym:roym-app"
            version = "0.1.0"

            [services.transaction]
            service_type = "wasm"
            source = "crates/roym_transaction/target/wasm32-wasip2/release/roym_transaction.wasm"
            interfaces = ["syneroym-roym:transaction/api@0.1.0"]

            [services.transaction.wake]
            interface = "syneroym-roym:transaction/api@0.1.0"
            method = "invoke"
            params = '[ "{\"method\":\"transaction.drive\",\"params\":{}}" ]'
            on_events = ["conversation.message"]
            every_secs = 30
            timeout_ms = 10000
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let tx = manifest.services.get(&LogicalServiceName::new("transaction")).unwrap();
    let wake = tx.config.wake.as_ref().expect("wake should be present");
    assert_eq!(wake.interface.as_str(), "syneroym-roym:transaction/api@0.1.0");
    assert_eq!(wake.method, "invoke");
    assert_eq!(
        wake.params.as_deref(),
        Some(r#"[ "{\"method\":\"transaction.drive\",\"params\":{}}" ]"#)
    );
    assert_eq!(wake.on_events, vec!["conversation.message"]);
    assert_eq!(wake.every_secs, 30);
    assert_eq!(wake.timeout_ms, DEFAULT_WAKE_TIMEOUT_MS);

    // TOML round-trip
    let toml_round = manifest.to_toml().unwrap();
    let deserialized_toml = SynAppManifest::from_toml(&toml_round).unwrap();
    assert_eq!(manifest, deserialized_toml);

    // JSON round-trip
    let json_round = manifest.to_json().unwrap();
    let deserialized_json = SynAppManifest::from_json(&json_round).unwrap();
    assert_eq!(manifest, deserialized_json);
}

#[test]
fn a_service_without_a_wake_keeps_its_deploy_hash() {
    let toml_str = r#"
            id = "syneroym:roym-app"
            version = "0.1.0"

            [services.transaction]
            service_type = "wasm"
            source = "crates/roym_transaction/target/wasm32-wasip2/release/roym_transaction.wasm"
            interfaces = ["syneroym-roym:transaction/api@0.1.0"]
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let tx = manifest.services.get(&LogicalServiceName::new("transaction")).unwrap();
    assert!(tx.config.wake.is_none());

    let serialized_toml = toml::to_string(&tx.config).unwrap();
    assert!(!serialized_toml.contains("wake"), "serialized TOML should not contain 'wake'");

    let serialized_json = serde_json::to_string(&tx.config).unwrap();
    assert!(!serialized_json.contains("wake"), "serialized JSON should not contain 'wake'");
}

#[test]
fn a_manifest_with_an_invalid_wake_is_refused_at_validation() {
    let toml_str = r#"
            id = "syneroym:roym-app"
            version = "0.1.0"

            [services.transaction]
            service_type = "wasm"
            source = "crates/roym_transaction/target/wasm32-wasip2/release/roym_transaction.wasm"
            interfaces = ["syneroym-roym:transaction/api@0.1.0"]

            [services.transaction.wake]
            interface = "syneroym-roym:undeclared/api@0.1.0"
            method = "invoke"
            every_secs = 30
        "#;

    let err = SynAppManifest::from_toml(toml_str).unwrap_err();
    assert!(err.to_string().contains("declares an invalid wake"), "{err}");
}

use std::fs;

use syneroym_core::test_constants::{GREETER_INTERFACE_NAME, greeter_wasm_path};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType as WitServiceType,
    WakeSpec as WitWakeSpec, WasmManifest,
};

use super::{super::*, helpers::*};

#[tokio::test]
async fn deploy_manifest_with_valid_wake_passes_and_persists_deploy_facts() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    let wake_spec = WitWakeSpec {
        interface_name: GREETER_INTERFACE_NAME.to_string(),
        method: "greet".to_string(),
        params: Some(r#"{"msg":"hello"}"#.to_string()),
        on_events: vec!["roym.sync.requested".to_string()],
        every_secs: 30,
        timeout_ms: 5000,
    };

    let manifest = DeployManifest::unpublished(
        ServiceConfig { wake: Some(wake_spec), ..Default::default() },
        WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
        }),
    );

    let service_id = "test-woken-service".to_string();
    service
        .deploy(service_id.clone(), manifest, &owner)
        .await
        .expect("deploy with valid wake should succeed");

    let wake_targets = service.registry.wake_targets();
    assert_eq!(wake_targets.len(), 1);
    let (target_id, wake_json) = &wake_targets[0];
    assert_eq!(target_id, &service_id);
    let spec: syneroym_app_orchestration::WakeSpec =
        serde_json::from_str(wake_json).expect("wake_json must parse as WakeSpec");
    assert_eq!(spec.interface.as_str(), GREETER_INTERFACE_NAME);
    assert_eq!(spec.method, "greet");
    assert_eq!(spec.params.as_deref(), Some(r#"{"msg":"hello"}"#));
    assert_eq!(spec.on_events, vec!["roym.sync.requested"]);
    assert_eq!(spec.every_secs, 30);
    assert_eq!(spec.timeout_ms, 5000);
}

#[tokio::test]
async fn deploy_manifest_with_wake_targeting_undeclared_interface_is_rejected() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    let wake_spec = WitWakeSpec {
        interface_name: "undeclared:interface/name".to_string(),
        method: "drive".to_string(),
        params: None,
        on_events: vec![],
        every_secs: 60,
        timeout_ms: 1000,
    };

    let manifest = DeployManifest::unpublished(
        ServiceConfig { wake: Some(wake_spec), ..Default::default() },
        WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
        }),
    );

    let err = service
        .deploy("test-rejected-wake-service".to_string(), manifest, &owner)
        .await
        .expect_err("deploy with undeclared wake interface must fail");

    assert!(
        err.contains("declared interfaces"),
        "expected error about declared interfaces, got: {err}"
    );
}

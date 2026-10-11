use syneroym_core::config::SubstrateConfig;

use super::CoordinatorWebRtc;

#[tokio::test]
async fn rejects_invalid_parent_relay_url() {
    let mut config = SubstrateConfig::default();
    config.parent_coordinator.iroh.url = Some("not-a-valid-url".to_string());

    let result = CoordinatorWebRtc::init(&config).await;
    assert!(result.is_err(), "expected error for invalid parent relay url");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("parent_coordinator.iroh.url"), "unexpected error message: {err}");
}

use syneroym_core::config::IrohParentConfig;

use super::HomeRelaySource;

#[test]
fn no_url_is_degraded_mode() {
    let config = IrohParentConfig { url: None };
    let source = HomeRelaySource::from_config(&config).unwrap();
    assert_eq!(source, None);
}

#[test]
fn bad_url_stops_startup() {
    let config = IrohParentConfig { url: Some("localhost:7964".into()) };
    let err = HomeRelaySource::from_config(&config).unwrap_err().to_string();
    assert!(err.contains("parent_coordinator.iroh.url"), "expected setting in error: {err}");
    assert!(err.contains("localhost:7964"), "expected bad value in error: {err}");
}

#[test]
fn url_is_a_fixed_source() {
    let config = IrohParentConfig { url: Some("http://127.0.0.1:7964".into()) };
    let source = HomeRelaySource::from_config(&config).unwrap();
    let want_url = "http://127.0.0.1:7964/".parse().unwrap();
    assert_eq!(source, Some(HomeRelaySource::Fixed { url: want_url }));
}

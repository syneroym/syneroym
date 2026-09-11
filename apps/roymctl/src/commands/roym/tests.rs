use serde_json::json;

use super::*;

fn svc(service_id: &str, interfaces: &[&str]) -> DeployedService {
    serde_json::from_value(json!({
        "service_id": service_id,
        "interfaces": interfaces,
        "endpoint_type": "wasm",
    }))
    .unwrap()
}

fn roym_svcs() -> Vec<DeployedService> {
    vec![
        svc("did:key:zWeb", &["syneroym-roym:web/api@0.1.0"]),
        svc("did:key:zProfile", &["syneroym-roym:profile/api@0.1.0"]),
        svc("did:key:zConv", &["syneroym-roym:conversation/api@0.1.0"]),
        svc("did:key:zOther", &["syneroym:http/incoming-handler@0.2.0"]),
    ]
}

#[test]
fn find_roym_service_matches_by_app_interface() {
    let svcs = roym_svcs();
    assert_eq!(find_roym_service(&svcs, "conversation").unwrap(), "did:key:zConv");
    assert_eq!(find_roym_service(&svcs, "web").unwrap(), "did:key:zWeb");
}

#[test]
fn find_roym_service_errors_when_absent() {
    let err = find_roym_service(&roym_svcs(), "directory").unwrap_err().to_string();
    assert!(err.contains("no Roym 'directory' service is deployed"), "{err}");
}

#[test]
fn find_roym_service_errors_when_ambiguous() {
    let svcs = vec![
        svc("did:key:zConvA", &["syneroym-roym:conversation/api@0.1.0"]),
        svc("did:key:zConvB", &["syneroym-roym:conversation/api@0.1.0"]),
    ];
    let err = find_roym_service(&svcs, "conversation").unwrap_err().to_string();
    assert!(err.contains("2 Roym 'conversation' services"), "{err}");
}

#[test]
fn parse_window_valid_and_invalid() {
    let v = parse_window("100,200").unwrap();
    assert_eq!(v["earliest_secs"], 100);
    assert_eq!(v["latest_secs"], 200);

    assert!(parse_window("200,100").is_err());
    assert!(parse_window("100").is_err());
    assert!(parse_window("100,200,300").is_err());
    assert!(parse_window("abc,200").is_err());
}

#[test]
fn parse_minor_units_handles_various_exponents() {
    assert_eq!(parse_minor_units("12.34", "USD", 2).unwrap(), 1234);
    assert_eq!(parse_minor_units("12.3", "USD", 2).unwrap(), 1230);
    assert_eq!(parse_minor_units("12", "USD", 2).unwrap(), 1200);
    assert_eq!(parse_minor_units("0.05", "USD", 2).unwrap(), 5);
    assert_eq!(parse_minor_units("0", "USD", 2).unwrap(), 0);
    assert!(parse_minor_units("12.345", "USD", 2).is_err());

    assert_eq!(parse_minor_units("1200", "JPY", 0).unwrap(), 1200);
    assert!(parse_minor_units("12.5", "JPY", 0).is_err());

    assert_eq!(parse_minor_units("12.5", "KWD", 3).unwrap(), 12500);
    assert_eq!(parse_minor_units("12.500", "KWD", 3).unwrap(), 12500);
    assert_eq!(parse_minor_units("12.123", "KWD", 3).unwrap(), 12123);
    assert!(parse_minor_units("12.1234", "KWD", 3).is_err());
}

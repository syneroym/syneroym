use super::*;

#[test]
fn status_values_keep_their_wire_strings() {
    assert_eq!(serde_json::to_string(&CoordinatorStatus::Healthy).unwrap(), "\"healthy\"");
    assert_eq!(serde_json::to_string(&CoordinatorStatus::AtCapacity).unwrap(), "\"at_capacity\"");
}

#[test]
fn unknown_status_string_parses_as_unknown() {
    let parsed: CoordinatorStatus = serde_json::from_str("\"degraded\"").unwrap();
    assert_eq!(parsed, CoordinatorStatus::Unknown);
}

#[test]
fn older_answer_without_new_fields_parses() {
    let raw = r#"{
        "endpoint_addr_bytes": [1, 2, 3],
        "substrate_id": "test-id",
        "relay_url": "https://relay.example.com",
        "parent_coordinator_url": null,
        "identity": {
            "did": "x",
            "controller_status": "unverified"
        },
        "relay": {
            "online": true
        }
    }"#;
    let info: CoordinatorInfo = serde_json::from_str(raw).unwrap();
    assert_eq!(info.status, CoordinatorStatus::Unknown);
    let relay = info.relay.expect("relay should be present");
    assert!(relay.online);
    assert_eq!(relay.clients, None);
    assert_eq!(relay.max_clients, None);
    assert!(info.registry.is_none());
}

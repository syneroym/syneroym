use syneroym_core::streaming::StreamDirection;
use syneroym_identity::Identity;
use syneroym_rpc::OnBehalfProof;
use syneroym_ucan::CapabilityToken;

use super::{RoutePreamble, RouteProtocol, RouteTransport, proof::*, redacted::*};

#[test]
fn parses_route_preamble() {
    let parsed = RoutePreamble::parse("json-rpc://health|substrate-123\n").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Binary);
    assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
    assert_eq!(parsed.interface, "health");
    assert_eq!(parsed.service_id, "substrate-123");
}

#[test]
fn parses_http_scheme() {
    let parsed = RoutePreamble::parse("http://health|substrate-123\n").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Http);
    assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
}

#[test]
fn parses_combined_scheme() {
    let parsed = RoutePreamble::parse("http-wrpc://health|substrate-123\n").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Http);
    assert_eq!(parsed.protocol, RouteProtocol::Wrpc);
}

#[test]
fn parses_route_preamble_no_interface() {
    let parsed = RoutePreamble::parse("json-rpc://substrate-123\n").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Binary);
    assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
    assert_eq!(parsed.interface, "");
    assert_eq!(parsed.service_id, "substrate-123");
}

#[test]
fn parses_http_path() {
    let parsed = RoutePreamble::from_http_path("/v1/my-service/health").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Http);
    assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
    assert_eq!(parsed.service_id, "my-service");
    assert_eq!(parsed.interface, "health");
}

#[test]
fn parses_http_path_rejects_bad_prefix() {
    assert!(RoutePreamble::from_http_path("/api/my-service/health").is_err());
}

#[test]
fn parses_http_path_rejects_missing_interface() {
    assert!(RoutePreamble::from_http_path("/v1/my-service").is_err());
}

#[test]
fn parses_http_path_rejects_empty_segments() {
    assert!(RoutePreamble::from_http_path("/v1//health").is_err());
    assert!(RoutePreamble::from_http_path("/v1/my-service/").is_err());
}

#[test]
fn parses_with_query_params() {
    let parsed =
        RoutePreamble::parse("raw://health|substrate-123?enc=ecdh-p256&pubkey=abc123\n").unwrap();
    assert_eq!(parsed.transport, RouteTransport::Raw);
    assert_eq!(parsed.protocol, RouteProtocol::Raw);
    assert_eq!(parsed.interface, "health");
    assert_eq!(parsed.service_id, "substrate-123");
    assert_eq!(parsed.enc, Some("ecdh-p256".to_string()));
    assert_eq!(parsed.pubkey, Some("abc123".to_string()));
}

#[test]
fn parses_and_round_trips_dir_query_param() {
    let parsed = RoutePreamble::parse("raw://file-transfer|svc-1?dir=download\n").unwrap();
    assert_eq!(parsed.dir, Some(StreamDirection::Download));
    assert_eq!(parsed.to_string(), "raw://file-transfer|svc-1?dir=download");

    let parsed = RoutePreamble::parse("raw://file-transfer|svc-1?dir=upload\n").unwrap();
    assert_eq!(parsed.dir, Some(StreamDirection::Upload));
}

#[test]
fn invalid_dir_value_parses_as_none_not_an_error() {
    let parsed = RoutePreamble::parse("raw://file-transfer|svc-1?dir=sideways\n").unwrap();
    assert_eq!(parsed.dir, None);
}

#[test]
fn missing_dir_is_none() {
    let parsed = RoutePreamble::parse("raw://file-transfer|svc-1\n").unwrap();
    assert_eq!(parsed.dir, None);
}

#[test]
fn ucan_round_trips_through_display_and_parse() {
    let issuer = Identity::generate().unwrap();
    let token = CapabilityToken::issue(
        &issuer,
        "did:key:hybndrfg8ejkmcpqx",
        vec![],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let mut preamble = RoutePreamble::parse("json-rpc://health|substrate-123").unwrap();
    preamble.ucan = Some(token.clone());

    let round_tripped = RoutePreamble::parse(&preamble.to_string()).unwrap();
    assert_eq!(round_tripped.ucan, Some(token));
}

#[test]
fn test_display() {
    let p1 = RoutePreamble::parse("http://health|substrate-123").unwrap();
    assert_eq!(p1.to_string(), "http://health|substrate-123");

    let p2 = RoutePreamble::parse("json-rpc://substrate-123").unwrap();
    assert_eq!(p2.to_string(), "json-rpc://substrate-123");

    let p3 = RoutePreamble::parse("http-wrpc://my-service").unwrap();
    assert_eq!(p3.to_string(), "http-wrpc://my-service");

    let p4 = RoutePreamble::parse("wrpc://my-interface|my-service").unwrap();
    assert_eq!(p4.to_string(), "wrpc://my-interface|my-service");

    let p5 =
        RoutePreamble::parse("raw://health|substrate-123?enc=ecdh-p256&pubkey=abc123\n").unwrap();
    assert_eq!(p5.to_string(), "raw://health|substrate-123?enc=ecdh-p256&pubkey=abc123");
}

#[test]
fn test_proof_bytes_fixed_vector() {
    let bytes = proof_bytes("svc", "api", 1_700_000_000);
    let bytes_hex = hex::encode(&bytes);
    assert_eq!(
        bytes_hex,
        "0000001473796e65726f796d2d707265616d626c652d763100000003737663000000036170690000000a31373030303030303030"
    );

    let id = Identity::from_bytes(&[7u8; 32]);
    let sig = id.sign(&bytes);
    let sig_hex = hex::encode(sig.to_bytes());
    assert_eq!(
        sig_hex,
        "31d6b275b59b3d4d96594cb2b5a7c2b3a4c4d3b6ba7c485e4db29b66c063a056990d8e3c6f9e1deefe9123da2bf7ce4e9b7306018fed22ccda76a33cc7244a0b"
    );
}

#[test]
fn test_new_fields_round_trip() {
    let on_behalf = OnBehalfProof {
        pubkey: "aabb".to_string(),
        delegation: Some("{\"some\":\"cert\"}".to_string()),
        service_id: "orig-svc".to_string(),
        interface: "orig-iface".to_string(),
        ts: 1_700_000_123,
        sig: "11".repeat(64),
    };
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some("04aabbcc".to_string());
    preamble.pubkey = Some("123456".to_string());
    preamble.ts = Some(1_700_000_999);
    preamble.sig = Some("22".repeat(64));
    preamble.on_behalf = Some(on_behalf.clone());

    let line = preamble.to_string();
    let parsed = RoutePreamble::parse(&line).unwrap();
    assert_eq!(parsed.enc.as_deref(), Some("ecdh-p256"));
    assert_eq!(parsed.epk.as_deref(), Some("04aabbcc"));
    assert_eq!(parsed.pubkey.as_deref(), Some("123456"));
    assert_eq!(parsed.ts, Some(1_700_000_999));
    assert_eq!(parsed.sig.as_deref(), Some("22".repeat(64).as_str()));
    assert_eq!(parsed.on_behalf, Some(on_behalf));
}

#[test]
fn test_strict_parse_errors() {
    // Bad ts (not u64)
    let err = RoutePreamble::parse("json-rpc://svc?ts=bad").unwrap_err();
    assert!(err.to_string().contains("invalid ts in preamble"));

    // Short sig (not 128 hex chars)
    let err = RoutePreamble::parse("json-rpc://svc?sig=abcd").unwrap_err();
    assert!(err.to_string().contains("invalid sig in preamble"));

    // Uppercase sig
    let upper_sig = "AA".repeat(64);
    let err = RoutePreamble::parse(&format!("json-rpc://svc?sig={upper_sig}")).unwrap_err();
    assert!(err.to_string().contains("invalid sig in preamble"));

    // on_behalf with unknown field
    let bad_json = serde_json::json!({
        "pubkey": "key",
        "delegation": null,
        "service_id": "s",
        "interface": "i",
        "ts": 12345,
        "sig": "sig",
        "unknown_extra": true
    });
    let hex_bad = hex::encode(bad_json.to_string());
    let err = RoutePreamble::parse(&format!("json-rpc://svc?on_behalf={hex_bad}")).unwrap_err();
    assert!(err.to_string().contains("on_behalf"));
}

#[test]
fn test_redacted_views_never_leak_proof_material() {
    let sig_hex = "22".repeat(64);
    let ucan_json = "{\"dummy\":\"ucan\"}";
    let on_behalf_json = "{\"pubkey\":\"k\",\"delegation\":null,\"service_id\":\"s\",\"interface\"\
                          :\"i\",\"ts\":1,\"sig\":\"s\"}";

    let mut preamble = RoutePreamble::binary_json_rpc("target-service", "target-interface");
    preamble.sig = Some(sig_hex.clone());
    preamble.ts = Some(1_700_000_000);
    preamble.pubkey = Some("11223344".to_string());
    preamble.epk = Some("04aabb".to_string());
    preamble.on_behalf = serde_json::from_str(on_behalf_json).ok();

    let redacted_str = preamble.redacted().to_string();
    let debug_str = format!("{preamble:?}");

    for output in [&redacted_str, &debug_str] {
        assert!(!output.contains(&sig_hex), "must not contain sig hex");
        assert!(!output.contains(ucan_json), "must not contain ucan");
        assert!(!output.contains(on_behalf_json), "must not contain on_behalf json");
        assert!(output.contains("sig=set"));
        assert!(output.contains("epk=set"));
        assert!(output.contains("on_behalf=set"));
    }
}

#[test]
fn test_route_text_cuts_at_question_and_256_chars() {
    assert_eq!(route_text("json-rpc://svc?param=1"), "json-rpc://svc");
    assert_eq!(route_text("  http://api|svc?a=b  "), "http://api|svc");
    let long_raw = format!("json-rpc://{}", "a".repeat(300));
    let cut = route_text(&long_raw);
    assert_eq!(cut.len(), 256);
}

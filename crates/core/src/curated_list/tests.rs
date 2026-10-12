use serde_json::json;
use syneroym_identity::substrate;

use super::{
    CoordinatorEntry, CoordinatorFilter, CoordinatorTransport, CuratedList, check_entries,
    normalize_url, rank, rank_key,
};

#[test]
fn normalize_url_accepts_valid_urls() {
    let cases = [
        ("HTTPS://Relay.Example.ORG:443/", "https://relay.example.org"),
        ("https://relay.example.org/", "https://relay.example.org"),
        ("https://relay.example.org", "https://relay.example.org"),
        ("http://relay.example.org:80", "http://relay.example.org"),
        ("https://relay.example.org:8443/", "https://relay.example.org:8443"),
        ("https://relay.example.org/base/", "https://relay.example.org/base"),
        ("http://127.0.0.1:7964", "http://127.0.0.1:7964"),
    ];

    for (input, expected) in cases {
        let result = normalize_url(input).expect("valid url");
        assert_eq!(result, expected, "failed for input {input}");
    }
}

#[test]
fn normalize_url_rejects_invalid_urls() {
    let cases = [
        "relay.example.org",
        "localhost:7964",
        "ftp://relay.example.org",
        "https://user@relay.example.org",
        "https://relay.example.org/?a=1",
        "https://relay.example.org/#x",
        "",
    ];

    for input in cases {
        assert!(normalize_url(input).is_err(), "expected rejection for {input}");
    }
}

#[test]
fn rank_key_fixed_test_vector_matches() {
    let real_did = "did:key:h7wy7qswaygbmnniz4if97w6jcoduwdzbqm37ijtdrszorgue6hdingo";
    let expected_rfc8032_pubkey =
        hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .expect("valid hex");

    let resolved = substrate::resolve_did_key(real_did).expect("resolves did key");
    assert_eq!(resolved.as_bytes(), expected_rfc8032_pubkey.as_slice());
    let derived = substrate::derive_did_key(&resolved);
    assert_eq!(derived, real_did);

    let k_a = hex::encode(rank_key(real_did, "https://relay-a.example.org"));
    let k_b = hex::encode(rank_key(real_did, "https://relay-b.example.org"));
    let k_c = hex::encode(rank_key(real_did, "https://relay-c.example.org"));

    assert_eq!(k_a, "7fc0d81c87a800d1cc5c0445dd112037181c842b7b0796d571bd4f4ea8e8e734");
    assert_eq!(k_b, "9fcc61b2c05933a41b26f2915b946025b09ee4363fe2546d01f66ea38a3fa2c7");
    assert_eq!(k_c, "d830425b53c269245d7c7638956be20a7036bb42fee4076723aa261d1cd84391");

    let other = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    let o_a = hex::encode(rank_key(other, "https://relay-a.example.org"));
    let o_b = hex::encode(rank_key(other, "https://relay-b.example.org"));
    let o_c = hex::encode(rank_key(other, "https://relay-c.example.org"));

    assert_eq!(o_a, "e237c8f6954d7984e41b684dbdcfc7db057a283d10c920abfb4310523518ad6c");
    assert_eq!(o_b, "d5223d837045ea1f3d422235d43cd6e91491b1cd94eb83381bee8a821b311ced");
    assert_eq!(o_c, "74abc4c2c13ab5078cfef0953022888b1ef9c32c018d940fa1d0aaaeb92867bf");
}

#[test]
fn rank_sorts_entries_deterministically_for_chooser() {
    let real_did = "did:key:h7wy7qswaygbmnniz4if97w6jcoduwdzbqm37ijtdrszorgue6hdingo";
    let other = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    let make_entries = || {
        vec![
            CoordinatorEntry {
                transport: CoordinatorTransport::IrohRelay,
                url: "https://relay-b.example.org".to_string(),
                info_url: "https://relay-b.example.org:7974".to_string(),
                operator: "B".to_string(),
            },
            CoordinatorEntry {
                transport: CoordinatorTransport::IrohRelay,
                url: "https://relay-c.example.org".to_string(),
                info_url: "https://relay-c.example.org:7974".to_string(),
                operator: "C".to_string(),
            },
            CoordinatorEntry {
                transport: CoordinatorTransport::IrohRelay,
                url: "https://relay-a.example.org".to_string(),
                info_url: "https://relay-a.example.org:7974".to_string(),
                operator: "A".to_string(),
            },
        ]
    };

    let ranked_real = rank(real_did, make_entries());
    let urls_real: Vec<_> = ranked_real.iter().map(|e| e.url.as_str()).collect();
    assert_eq!(
        urls_real,
        [
            "https://relay-a.example.org",
            "https://relay-b.example.org",
            "https://relay-c.example.org"
        ]
    );

    let ranked_other = rank(other, make_entries());
    let urls_other: Vec<_> = ranked_other.iter().map(|e| e.url.as_str()).collect();
    assert_eq!(
        urls_other,
        [
            "https://relay-c.example.org",
            "https://relay-b.example.org",
            "https://relay-a.example.org"
        ]
    );
}

#[test]
fn rank_normalizes_and_deduplicates_entries() {
    let real_did = "did:key:h7wy7qswaygbmnniz4if97w6jcoduwdzbqm37ijtdrszorgue6hdingo";
    let entries = vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://Relay-A.example.org:443/".to_string(),
            info_url: "https://relay-a.example.org:7974".to_string(),
            operator: "A1".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay-a.example.org".to_string(),
            info_url: "https://relay-a.example.org:7974".to_string(),
            operator: "A2".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "not a valid url".to_string(),
            info_url: "https://bad.example.org".to_string(),
            operator: "Bad".to_string(),
        },
    ];

    let ranked = rank(real_did, entries);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].url, "https://relay-a.example.org");
}

#[test]
fn rank_preserves_first_choice_when_non_first_entry_removed() {
    let other = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    let entries = vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay-b.example.org".to_string(),
            info_url: "https://relay-b.example.org:7974".to_string(),
            operator: "B".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay-c.example.org".to_string(),
            info_url: "https://relay-c.example.org:7974".to_string(),
            operator: "C".to_string(),
        },
    ];

    let ranked = rank(other, entries);
    assert_eq!(ranked[0].url, "https://relay-c.example.org");
}

#[test]
fn check_entries_normalizes_and_validates() {
    let mut entries = vec![CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "HTTPS://relay.example.org:443/".to_string(),
        info_url: "HTTPS://relay.example.org:7974/".to_string(),
        operator: "Example".to_string(),
    }];

    check_entries(&mut entries).expect("valid entry");
    assert_eq!(entries[0].url, "https://relay.example.org");
    assert_eq!(entries[0].info_url, "https://relay.example.org:7974");
}

#[test]
fn check_entries_fails_on_bad_url_naming_index_and_field() {
    let mut entries = vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay-0.example.org".to_string(),
            info_url: "https://relay-0.example.org:7974".to_string(),
            operator: "0".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "bad-url".to_string(),
            info_url: "https://relay-1.example.org:7974".to_string(),
            operator: "1".to_string(),
        },
    ];

    let err = check_entries(&mut entries).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("roles.community_registry.coordinators[1]"),
        "error {msg:?} must contain entry index"
    );
    assert!(msg.contains("url"), "error {msg:?} must contain field name");
}

#[test]
fn check_entries_fails_on_bad_info_url() {
    let mut entries = vec![CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://relay.example.org".to_string(),
        info_url: "bad-info-url".to_string(),
        operator: "0".to_string(),
    }];

    let err = check_entries(&mut entries).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("info_url"), "error {msg:?} must contain info_url");
}

#[test]
fn check_entries_fails_on_duplicate_urls() {
    let mut entries = vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay.example.org".to_string(),
            info_url: "https://relay.example.org:7974".to_string(),
            operator: "0".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "HTTPS://relay.example.org:443/".to_string(),
            info_url: "https://relay.example.org:7974".to_string(),
            operator: "1".to_string(),
        },
    ];

    let err = check_entries(&mut entries).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("roles.community_registry.coordinators[1] is a duplicate of entry [0]"),
        "error {msg:?} must name indices [1] and [0]"
    );
}

#[test]
fn curated_list_serde_matches_expected_json() {
    let list = CuratedList {
        max_age_secs: 86_400,
        entries: vec![CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay.example.org".to_string(),
            info_url: "https://relay.example.org:7974".to_string(),
            operator: "Example Org".to_string(),
        }],
    };

    let serialized = serde_json::to_value(&list).expect("serializes");
    let expected = json!({
        "max_age_secs": 86400,
        "coordinators": [
            {
                "transport": "iroh-relay",
                "url": "https://relay.example.org",
                "info_url": "https://relay.example.org:7974",
                "operator": "Example Org"
            }
        ]
    });
    assert_eq!(serialized, expected);

    let deserialized: CuratedList<CoordinatorEntry> =
        serde_json::from_value(expected).expect("deserializes");
    assert_eq!(deserialized, list);
}

#[test]
fn curated_list_serde_ignores_unknown_fields() {
    let json_with_extra = json!({
        "max_age_secs": 3600,
        "future_field": "ignore_me",
        "coordinators": [
            {
                "transport": "iroh-relay",
                "url": "https://relay.example.org",
                "info_url": "https://relay.example.org:7974",
                "operator": "Example Org",
                "extra_field": 123
            }
        ]
    });

    let list: CuratedList<CoordinatorEntry> =
        serde_json::from_value(json_with_extra).expect("deserializes with unknown fields");
    assert_eq!(list.max_age_secs, 3600);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].operator, "Example Org");
}

#[test]
fn curated_list_serde_fails_on_missing_required_keys() {
    let missing_max_age = json!({
        "coordinators": []
    });
    assert!(serde_json::from_value::<CuratedList<CoordinatorEntry>>(missing_max_age).is_err());

    let missing_coordinators = json!({
        "max_age_secs": 86400
    });
    assert!(serde_json::from_value::<CuratedList<CoordinatorEntry>>(missing_coordinators).is_err());
}

#[test]
fn coordinator_filter_matches_transport() {
    let entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://relay.example.org".to_string(),
        info_url: "https://relay.example.org:7974".to_string(),
        operator: "Example".to_string(),
    };
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    assert!(super::ListEntry::matches(&entry, &filter));
}

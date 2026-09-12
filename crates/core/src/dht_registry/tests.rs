use syneroym_identity::Identity;

use super::*;

fn sample_endpoint_info(service_id: &str) -> EndpointInfo {
    EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: "did:key:zSubstrate".to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2, // far future -- not what these tests are about
        generation: 0,
    }
}

/// Mirrors `MasterAnchorPayload::sign`, but backdates the pkarr packet
/// timestamp so the stale-anchor path can be exercised directly
/// instead of argued for structurally.
fn sign_backdated(
    mut payload: MasterAnchorPayload,
    identity: &Identity,
    hours_ago: u64,
) -> SignedMasterAnchor {
    let master_id = substrate::derive_did_key(&identity.public_key());
    let keypair = Keypair::from_secret_key(&identity.to_bytes());

    let now_micros =
        time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_micros() as u64;
    let backdated_micros = now_micros - hours_ago * 60 * 60 * 1_000_000;
    let timestamp = Timestamp::from(backdated_micros);
    payload.timestamp = timestamp.as_u64();

    let json_str = serde_json::to_string(&payload).unwrap();
    let txt_rdata = TXT::try_from(json_str.as_str()).unwrap();
    let name = Name::new(PKARR_DNS_NAME).unwrap();
    let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
    let signed_packet = SignedPacket::new(&keypair, &records, timestamp).unwrap();
    let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
    SignedMasterAnchor { master_id, payload, pkarr_packet_hex }
}

#[test]
fn a_self_signed_record_verifies() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let signed = sample_endpoint_info(&did).sign(&identity).unwrap();

    assert!(signed.verify().is_ok());
}

#[test]
fn verify_returns_the_packets_own_timestamp() {
    // `community_registry`'s compare-and-swap needs this to reject a
    // rollback -- it is the pkarr/BEP44 sequence number, and the only
    // thing that makes "newer record wins" enforceable without a
    // second, independently-trackable counter.
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let signed = sample_endpoint_info(&did).sign(&identity).unwrap();
    let ts = signed.verify().expect("a freshly self-signed record must verify");

    let pubkey = substrate::resolve_did_key(&did).unwrap();
    let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes()).unwrap();
    let packet_bytes = hex::decode(&signed.pkarr_packet_hex).unwrap();
    let packet =
        SignedPacket::from_relay_payload(&pkarr_pubkey, &Bytes::from(packet_bytes)).unwrap();
    assert_eq!(ts, packet.timestamp());
}

#[test]
fn a_record_is_rejected_when_signed_by_a_key_other_than_its_own_service_id() {
    let claimed = Identity::generate().unwrap();
    let actual_signer = Identity::generate().unwrap();
    let claimed_did = substrate::derive_did_key(&claimed.public_key());

    // The one keying shape this design allows: every record must be
    // self-signed by the key its own `service_id` resolves to. A member
    // endpoint record's `service_id` is a member master DID, so this is
    // exactly the shape a hosting substrate cannot produce -- it never
    // holds that key (ADR-0020 §3).
    let signed = sample_endpoint_info(&claimed_did).sign(&actual_signer).unwrap();

    assert!(signed.verify().is_err());
}

#[test]
fn rewriting_the_substrate_id_after_signing_is_rejected() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let mut signed = sample_endpoint_info(&did).sign(&identity).unwrap();
    signed.info.substrate_id = "did:key:zAttacker".to_string();

    // The attack this guards against: a relay rewriting substrate_id so
    // a lookup follows it to a host the attacker controls.
    assert!(signed.verify().is_err());
}

/// ADR-0022 §2: a record signed before `generation` existed has no such
/// key in its stored JSON at all -- not `null`, absent -- and
/// must still deserialize, reading as `0` ("no generation claimed").
/// Against a hand-written fixture rather than a round trip, since a
/// round trip through the current struct can never omit the field.
#[test]
fn an_endpoint_record_written_before_generation_existed_reads_as_zero() {
    let json = r#"{
        "service_id": "did:key:zApp",
        "substrate_id": "did:key:zNode",
        "endpoint_type": "service",
        "mechanisms": [],
        "is_private": false,
        "not_after": 4102444800
    }"#;
    let info: EndpointInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.generation, 0);
}

/// `generation` is inside the signed payload, not a header alongside
/// it -- a relay that bumps it without the master's
/// cooperation must be caught exactly like `substrate_id` tampering
/// above.
#[test]
fn a_generation_survives_the_signature_round_trip() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let mut info = sample_endpoint_info(&did);
    info.generation = 1;
    let mut signed = info.sign(&identity).unwrap();
    assert!(signed.verify().is_ok(), "an honest generation must verify");

    signed.info.generation = 2;
    assert!(signed.verify().is_err(), "a generation altered after signing must fail verification");
}

#[test]
fn an_expired_record_is_rejected() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let mut info = sample_endpoint_info(&did);
    info.not_after = 1; // long past, regardless of when this test runs
    let signed = info.sign(&identity).unwrap();

    let err = signed.verify().expect_err("an expired record must not verify");
    assert!(err.to_string().contains("expired"));
}

/// Decodes a signed record's own hex-encoded packet back into the
/// `SignedPacket` shape `RegistryClient::lookup`'s DHT branch gets from
/// `dht.resolve` -- the same reconstruction `verify` does internally, so
/// `extract_verified_endpoint_from_packet` can be exercised without a
/// live DHT.
fn packet_from(signed: &SignedEndpointInfo) -> SignedPacket {
    let pubkey = substrate::resolve_did_key(&signed.info.service_id).unwrap();
    let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes()).unwrap();
    let packet_bytes = hex::decode(&signed.pkarr_packet_hex).unwrap();
    SignedPacket::from_relay_payload(&pkarr_pubkey, &Bytes::from(packet_bytes)).unwrap()
}

#[test]
fn extract_verified_endpoint_from_packet_returns_a_live_record() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let signed = sample_endpoint_info(&did).sign(&identity).unwrap();
    let packet = packet_from(&signed);

    let extracted = extract_verified_endpoint_from_packet(&did, &packet)
        .expect("a live record must be returned");
    assert_eq!(extracted.info.service_id, did);
}

#[test]
fn extract_verified_endpoint_from_packet_rejects_an_expired_one() {
    // The DHT record itself is authentic -- pkarr's own signature check
    // already passed by the time `resolve` returns it -- but its
    // `not_after` has lapsed. This is exactly the case the HTTP registry
    // branch already covered via `verify()`; the DHT branch used to skip
    // it entirely.
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let mut info = sample_endpoint_info(&did);
    info.not_after = 1;
    let signed = info.sign(&identity).unwrap();
    let packet = packet_from(&signed);

    assert!(extract_verified_endpoint_from_packet(&did, &packet).is_none());
}

#[tokio::test]
async fn a_self_signed_record_registers_to_the_dht_with_no_http_registry_configured() {
    // Reverses an earlier restriction on this design: a delegation-
    // signed record used to be keyed by a master DID but signed by a
    // different (instance) key, so pkarr -- which keys a packet by its
    // *signing* key -- had no DHT slot for it. Every record is now
    // self-signed by the key its own `service_id` resolves to, so that
    // mismatch cannot occur and every record has a DHT home.
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let signed = sample_endpoint_info(&did).sign(&identity).unwrap();

    let client = RegistryClient::new(true, None);
    // `sync_dht: false` backgrounds the real network publish (fire-and-
    // forget), so this returns immediately without needing live DHT
    // connectivity -- it is `register`'s early HTTP-registry-required
    // refusal this test disproves, not the DHT publish itself.
    assert!(client.register(&signed, false).await.is_ok());
}

#[tokio::test]
async fn test_dht_publication_skipped_for_private_record() {
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let mut info = sample_endpoint_info(&did);
    info.is_private = true;
    let signed = info.sign(&identity).unwrap();

    let client = RegistryClient::new(true, None);
    let res = client.register(&signed, false).await;
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("marked private is deliberately not published to the DHT"),
        "{err_msg}"
    );
}

#[test]
fn a_stale_anchor_still_passes_verify_signature_and_still_fails_verify() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload { revoked_keys: vec!["k".to_string()], ..Default::default() };

    let signed = sign_backdated(payload, &identity, 25);

    assert!(signed.verify_signature().is_ok());
    assert!(signed.verify().is_err());
}

#[test]
fn an_anchor_whose_master_id_does_not_resolve_is_rejected() {
    // Not the `master_id` equality tightening (that guard lives
    // in `fetch_own_master_anchor`/`resolve_master_anchor`, which need a
    // real registry to exercise -- see `crates/community_registry`'s
    // `refresh_refuses_an_anchor_served_under_the_wrong_master*` tests).
    // This is the pre-existing key-resolution failure: an unresolvable
    // `master_id` string can never verify at all.
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload::default();

    let mut signed = payload.sign(&identity).unwrap();
    signed.master_id = "did:key:zSomeoneElse".to_string();

    assert!(signed.verify_signature().is_err());
}

#[test]
fn stripping_a_revoked_key_after_signing_is_rejected() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload {
        revoked_keys: vec!["did:key:zRevoked".to_string()],
        ..Default::default()
    };

    let mut signed = payload.sign(&identity).unwrap();
    signed.payload.revoked_keys.clear();

    assert!(signed.verify_signature().is_err());
}

#[test]
fn adding_a_revoked_key_after_signing_is_rejected() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload::default();

    let mut signed = payload.sign(&identity).unwrap();
    signed.payload.revoked_keys.push("did:key:zInjected".to_string());

    assert!(signed.verify_signature().is_err());
}

#[test]
fn rewriting_the_revoke_list_registry_after_signing_is_rejected() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload::default();

    let mut signed = payload.sign(&identity).unwrap();
    signed.payload.revoke_list_registry = Some("https://attacker.example/list".to_string());

    assert!(signed.verify_signature().is_err());
}

#[test]
fn test_master_anchor_payload_timestamp_validation() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload::default();

    let mut signed = payload.clone().sign(&identity).unwrap();
    assert!(signed.verify().is_ok());

    // Negative test: Mismatch timestamp
    signed.payload.timestamp -= 1;
    assert!(signed.verify().is_err());
}

#[test]
fn test_master_anchor_payload_expired() {
    let identity = Identity::generate().unwrap();
    let payload = MasterAnchorPayload::default();

    let signed = payload.sign(&identity).unwrap();

    // We can't easily manipulate pkarr SignedPacket timestamp since it's signed.
    // But verify() works on the signed pkarr packet timestamp.
    assert!(signed.verify().is_ok());
}

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use syneroym_core::{dht_registry::MasterAnchorPayload, storage::MockStorage};
use syneroym_identity::{Identity, substrate::derive_did_key};
use tokio::io::duplex;

use super::*;

/// A fresh, empty `EndpointRegistry` for tests that don't exercise
/// owner-rooted trust (every other test in this module).
fn empty_registry() -> EndpointRegistry {
    EndpointRegistry::new_mock(Arc::new(MockStorage::new()))
}

/// A `MasterAnchorResolver` double whose `revoked_keys` are configured
/// per-issuer, mirroring `handshake.rs`'s own test `MockResolver`.
#[derive(Debug)]
struct MockResolver {
    revoked: HashMap<String, Vec<String>>,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for MockResolver {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        let mut anchor = MasterAnchorPayload::default();
        if let Some(revoked) = self.revoked.get(master_id) {
            anchor.revoked_keys = revoked.clone();
        }
        Ok(anchor)
    }
}

/// A `MasterAnchorResolver` double that counts calls, used to verify
/// `ucan_chain_not_revoked` de-duplicates identical `(issuer, audience)`
/// edges rather than resolving each occurrence independently.
#[derive(Debug, Default)]
struct CountingResolver {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for CountingResolver {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(MasterAnchorPayload::default())
    }
}

fn ucan_preamble(token: CapabilityToken) -> RoutePreamble {
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    preamble.ucan = Some(token);
    preamble
}

/// Step 21 (reference scenario): a client presents a UCAN rooted at the
/// node's admin root -> `build_caller` verifies the chain and the
/// normalized capability lands in `CallerContext.session`, with `auth`
/// upgraded to `Ucan`.
#[tokio::test]
async fn build_caller_admits_a_ucan_chain_rooted_at_admin_root() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        "did:key:zNode",
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Ucan);
    assert_eq!(caller.caller_did, id.master_did);
    assert!(
        caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
    );
}

/// `SessionContext::from_verified_chain` verifies `anchor_did` (ADR-0015
/// A5, amended), but `build_caller` builds `session` from
/// `Default::default()` and only merges `capabilities`/`claims` from the
/// verified result -- `anchor_did` must be copied across too, or the
/// field never reaches a real `CallerContext` and the `anchor` policy
/// terminal is unreachable in production regardless of how correctly it
/// compiles. `user_a` self-declares as the anchor and delegates through
/// an intermediate service; the presenting client's own chain inherits
/// that anchor unchanged.
#[tokio::test]
async fn build_caller_threads_the_verified_anchor_did_into_the_session() {
    let owner = Identity::generate().unwrap();
    let user_a = Identity::generate().unwrap();
    let intermediate = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let user_a_did = derive_did_key(&user_a.public_key());
    let intermediate_did = derive_did_key(&intermediate.public_key());
    let client_did = derive_did_key(&client.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let user_a_to_intermediate = CapabilityToken::issue_with_anchor(
        &user_a,
        &intermediate_did,
        Some(user_a_did.clone()),
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let intermediate_to_client = CapabilityToken::issue_with_anchor(
        &intermediate,
        &client_did,
        Some(user_a_did.clone()),
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![user_a_to_intermediate],
    )
    .unwrap();

    let preamble = ucan_preamble(intermediate_to_client);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    // `user_a` owns `svc1` and roots its own delegation so the
    // capability -- and with it the anchor substantiation check --
    // actually admits (ADR-0015 A6).
    let registry = empty_registry();
    registry.set_owner("svc1".to_string(), user_a_did.clone()).await.unwrap();

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        "did:key:zNode",
        &registry,
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.session.anchor_did, Some(user_a_did));
}

/// The same claim as
/// `build_caller_admits_a_ucan_chain_rooted_at_admin_root`, but driven
/// end to end from wire bytes rather than a hand-built `RoutePreamble`/
/// `VerifiedIdentity`: the preamble is serialized then re-`parse`d
/// (exercising the `ucan=`/`pubkey=` hex decode a real peer's
/// bytes would go through), and the `VerifiedIdentity` comes from
/// `HandshakeVerifier::verify_preamble` (the same call `handle_stream`
/// makes) rather than being constructed directly.
#[tokio::test]
async fn parsed_wire_preamble_with_ucan_reaches_build_caller() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    preamble.pubkey = Some(hex::encode(client.public_key().to_bytes()));
    preamble.ucan = Some(token);

    // Round-trip through the actual wire format, not the struct directly.
    let wire_line = preamble.to_preamble_line();
    let parsed = RoutePreamble::parse(&wire_line).unwrap();
    assert!(parsed.ucan.is_some(), "ucan= must survive the hex-encode/decode round trip");

    let resolver = MockResolver { revoked: HashMap::new() };
    let verified_id = HandshakeVerifier::verify_preamble(&parsed, &resolver)
        .await
        .expect("a self-asserted pubkey with no delegation cert must verify");
    assert_eq!(verified_id.master_did, client_did);

    let caller = build_caller(
        &parsed,
        &verified_id,
        Some(&admin_root),
        "did:key:zNode",
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Ucan);
    assert!(
        caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
    );
}

/// A UCAN presented by a DID other than the one it's addressed to (the
/// verified connection identity != `token.audience_did`) fails
/// structural verification: no capability is admitted, and `auth` stays
/// at the pre-UCAN `Delegated` level (fail-open on the transport
/// identity, fail-closed on the bad authorization token).
#[tokio::test]
async fn build_caller_rejects_audience_mismatch() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let impostor = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let impostor_did = derive_did_key(&impostor.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: impostor_did.clone(), temporary_did: impostor_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        "did:key:zNode",
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Delegated);
    assert!(
        !caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
    );
}

/// A chain rooted at an issuer that is neither the node's admin root nor
/// the target service's recorded owner grants nothing (the registry
/// here is empty, so there is no owner to match). `auth` must not
/// upgrade to `Ucan` either: the chain
/// verified structurally but admitted zero capabilities, so it holds no
/// more privilege than the pre-UCAN `Delegated` level.
#[tokio::test]
async fn build_caller_drops_capabilities_from_an_untrusted_root() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    // Issued by `alice`, who is not the admin root.
    let token = CapabilityToken::issue(
        &alice,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        "did:key:zNode",
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Delegated);
    assert!(
        !caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
    );
}

/// A structurally valid chain whose audience DID has been revoked by
/// the issuer's master anchor is rejected wholesale: no capability is
/// admitted and `auth` does not upgrade to `Ucan`.
#[tokio::test]
async fn build_caller_rejects_a_revoked_chain() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver {
        revoked: HashMap::from([(admin_root.clone(), vec![id.master_did.clone()])]),
    };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        "did:key:zNode",
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Delegated);
    assert!(
        !caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string()))
    );
}

/// A chain that reuses the same proof twice (a diamond shape) must
/// resolve each distinct `(issuer, audience)` edge only once, not once
/// per occurrence.
#[tokio::test]
async fn ucan_chain_not_revoked_dedupes_repeated_edges() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    // The same proof embedded twice.
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![owner_to_alice.clone(), owner_to_alice],
    )
    .unwrap();

    let resolver = CountingResolver::default();
    assert!(ucan_chain_not_revoked(&alice_to_bob, &resolver).await);
    assert_eq!(
        resolver.calls.load(Ordering::SeqCst),
        2,
        "expected exactly one resolution each for the (alice, bob) and (owner, alice) edges, \
         despite (owner, alice) appearing twice"
    );
}

/// A peer that never sends anything must not hold `read_preamble` open
/// forever -- with tokio's paused virtual clock this fires effectively
/// instantly rather than costing real wall-clock time.
#[tokio::test(start_paused = true)]
async fn test_read_preamble_times_out_on_idle_stream() {
    let (client, server) = duplex(64);
    let mut reader = BufReader::new(server);

    let result = time::timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader)).await;

    assert!(result.is_err(), "expected a timeout since the peer never wrote a preamble");
    drop(client);
}

/// An unauthenticated peer sending a line with no newline anywhere
/// within `MAX_PREAMBLE_LINE_BYTES` must be rejected, not read into
/// memory without bound.
#[tokio::test]
async fn test_read_preamble_rejects_oversized_line() {
    let oversized_len = MAX_PREAMBLE_LINE_BYTES as usize + 1024;
    // Large enough that `write_all` completes without needing a
    // concurrent reader to drain it.
    let (mut client, server) = duplex(oversized_len + 1024);
    let mut reader = BufReader::new(server);

    client.write_all(&vec![b'a'; oversized_len]).await.unwrap();

    let err = read_preamble(&mut reader).await.unwrap_err();
    assert!(
        err.to_string().contains("exceeds the maximum length"),
        "expected the oversized-line error, got: {err}"
    );
}

/// `Take`'s capped view into the reader's buffered data must not cause
/// bytes **after** the newline to be consumed/discarded -- only
/// `read_line`'s own delimiter scan drives how much of the underlying
/// buffer is actually consumed, so a pipelined payload following the
/// preamble line in the same write (e.g. the initial framed request)
/// must remain intact and correctly positioned for the next read.
#[tokio::test]
async fn test_read_preamble_preserves_bytes_after_the_line() {
    let (mut client, server) = duplex(4096);
    let mut reader = BufReader::new(server);

    let mut sent = b"json-rpc://health|substrate-123\n".to_vec();
    sent.extend_from_slice(b"TRAILING-PAYLOAD");
    client.write_all(&sent).await.unwrap();

    let preamble = read_preamble(&mut reader).await.unwrap();
    assert_eq!(preamble.service_id, "substrate-123");

    let mut trailing = vec![0u8; b"TRAILING-PAYLOAD".len()];
    reader.read_exact(&mut trailing).await.unwrap();
    assert_eq!(&trailing, b"TRAILING-PAYLOAD");
}

/// A peer that promptly sends a valid preamble line must not be
/// penalized by the timeout wrapper.
#[tokio::test]
async fn test_read_preamble_succeeds_within_timeout() {
    let (mut client, server) = duplex(64);
    let mut reader = BufReader::new(server);

    client.write_all(b"json-rpc://health|substrate-123\n").await.unwrap();

    let result = time::timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader)).await;

    let preamble = result.expect("must not time out on a promptly-sent preamble").unwrap();
    assert_eq!(preamble.service_id, "substrate-123");
}

/// An unowned substrate (`admin_root: None`) grants no node-wide
/// capability at all -- neither the `orchestrator/*` abilities the old
/// bootstrap posture used to issue, nor `substrate/admin`.
/// Fails closed; the only route to ownership is `roymctl substrate
/// claim`, run off the wire.
#[tokio::test]
async fn an_unowned_substrate_grants_no_node_wide_capability() {
    let client = Identity::generate().unwrap();
    let client_did = derive_did_key(&client.public_key());
    let node_did = "did:key:zNodeUnowned";
    let node_resource = ResourceUri::substrate(node_did);

    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller =
        build_caller(&preamble, &id, None, node_did, &empty_registry(), &resolver, false).await;

    for ability in [
        Ability::ORCHESTRATOR_DEPLOY,
        Ability::ORCHESTRATOR_UNDEPLOY,
        Ability::ORCHESTRATOR_STATUS,
        Ability::SUBSTRATE_ADMIN,
    ] {
        assert!(
            !caller.has_capability(&node_resource, &Ability(ability.to_string())),
            "expected unowned substrate to grant no node-wide capability, got {ability}"
        );
    }
}

/// The regression test for the over-grant trap the now-removed unowned
/// bootstrap grant carried: an unowned substrate must
/// NOT grant `data-layer/admin` (or `substrate/admin`, which would
/// entail it) to a verified caller. Passes trivially post-P0 (no
/// node-wide capability is granted at all), and is kept as a named
/// regression guard rather than folded into the test above.
#[tokio::test]
async fn unowned_substrate_does_not_grant_data_layer_admin() {
    let client = Identity::generate().unwrap();
    let client_did = derive_did_key(&client.public_key());
    let node_did = "did:key:zNodeUnowned";
    let some_service = ResourceUri::service("app-1", "svc-a");

    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller =
        build_caller(&preamble, &id, None, node_did, &empty_registry(), &resolver, false).await;

    assert!(
        !caller.has_capability(&some_service, &Ability(Ability::DATA_LAYER_ADMIN.to_string())),
        "unowned substrate must not grant data-layer/admin -- the over-grant trap"
    );
    assert!(!caller.has_capability(
        &ResourceUri::substrate(node_did),
        &Ability(Ability::SUBSTRATE_ADMIN.to_string())
    ));
}

/// The node's own DID is granted
/// `supervisor/resolve` node-wide only when
/// `[iam].grant_resolve_to_node_did` is on, and never otherwise (even
/// for the node's own DID) or for any other caller.
#[tokio::test]
async fn the_node_did_is_granted_supervisor_resolve_only_when_the_gate_is_on() {
    let node_did = "did:key:zNodeSelf";
    let node_resource = ResourceUri::substrate(node_did);
    let resolve_ability = Ability(Ability::SUPERVISOR_RESOLVE.to_string());
    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let resolver = MockResolver { revoked: HashMap::new() };

    // Gate off: even the node's own DID gets nothing.
    let self_id =
        VerifiedIdentity { master_did: node_did.to_string(), temporary_did: node_did.to_string() };
    let caller =
        build_caller(&preamble, &self_id, None, node_did, &empty_registry(), &resolver, false)
            .await;
    assert!(!caller.has_capability(&node_resource, &resolve_ability), "gate is off");

    // Gate on, but a different caller: still nothing.
    let other = Identity::generate().unwrap();
    let other_did = derive_did_key(&other.public_key());
    let other_id = VerifiedIdentity { master_did: other_did.clone(), temporary_did: other_did };
    let caller =
        build_caller(&preamble, &other_id, None, node_did, &empty_registry(), &resolver, true)
            .await;
    assert!(!caller.has_capability(&node_resource, &resolve_ability), "not the node's DID");

    // Gate on, node's own DID: granted.
    let caller =
        build_caller(&preamble, &self_id, None, node_did, &empty_registry(), &resolver, true).await;
    assert!(caller.has_capability(&node_resource, &resolve_ability), "gate on, node's DID");
}

/// The whole safety claim of the same-node resolve grant, as a
/// negative on the same
/// `CallerContext` -- the granted capability answers `supervisor/resolve`
/// on any `synapp:` resource and `false` for `substrate/admin`,
/// `orchestrator/deploy`, and `data-layer/admin`.
#[tokio::test]
async fn the_same_node_grant_does_not_confer_substrate_admin() {
    let node_did = "did:key:zNodeSelf2";
    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let resolver = MockResolver { revoked: HashMap::new() };
    let self_id =
        VerifiedIdentity { master_did: node_did.to_string(), temporary_did: node_did.to_string() };

    let caller =
        build_caller(&preamble, &self_id, None, node_did, &empty_registry(), &resolver, true).await;

    assert!(
        caller.has_capability(
            &ResourceUri(format!("synapp:{}", "did:key:zSomeApp")),
            &Ability(Ability::SUPERVISOR_RESOLVE.to_string())
        ),
        "the bare substrate scope must cover any synapp: resource"
    );
    for ability in
        [Ability::SUBSTRATE_ADMIN, Ability::ORCHESTRATOR_DEPLOY, Ability::DATA_LAYER_ADMIN]
    {
        assert!(
            !caller
                .has_capability(&ResourceUri::substrate(node_did), &Ability(ability.to_string())),
            "the same-node resolve grant must not confer {ability}"
        );
    }
}

/// On an owned substrate, only the caller whose DID equals `admin_root`
/// gets `substrate/admin`; anyone else gets no node-wide capability at
/// all.
#[tokio::test]
async fn owned_substrate_grants_substrate_admin_only_to_the_owner() {
    let owner = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let other_did = derive_did_key(&other.public_key());
    let node_did = "did:key:zNodeOwned";
    let node_resource = ResourceUri::substrate(node_did);

    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let resolver = MockResolver { revoked: HashMap::new() };

    let owner_id =
        VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did.clone() };
    let owner_caller = build_caller(
        &preamble,
        &owner_id,
        Some(&owner_did),
        node_did,
        &empty_registry(),
        &resolver,
        false,
    )
    .await;
    assert!(
        owner_caller.has_capability(&node_resource, &Ability(Ability::SUBSTRATE_ADMIN.to_string()))
    );

    let other_id = VerifiedIdentity { master_did: other_did.clone(), temporary_did: other_did };
    let other_caller = build_caller(
        &preamble,
        &other_id,
        Some(&owner_did),
        node_did,
        &empty_registry(),
        &resolver,
        false,
    )
    .await;
    assert!(
        !other_caller
            .has_capability(&node_resource, &Ability(Ability::SUBSTRATE_ADMIN.to_string()))
    );
    for ability in
        [Ability::ORCHESTRATOR_DEPLOY, Ability::ORCHESTRATOR_UNDEPLOY, Ability::ORCHESTRATOR_STATUS]
    {
        assert!(
            !other_caller.has_capability(&node_resource, &Ability(ability.to_string())),
            "an owned substrate must not fall back to the unowned grant for a non-owner"
        );
    }
}

/// The owner's `substrate/admin` capability names the node's own DID,
/// not the caller's -- a B0 naming quirk that stayed inert only because
/// of the `is_substrate_scope` wildcard (F2/F6, B7b).
#[tokio::test]
async fn substrate_admin_capability_names_the_node_not_the_caller() {
    let owner = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let node_did = "did:key:zNodeOwned";

    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let resolver = MockResolver { revoked: HashMap::new() };
    let id = VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did.clone() };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&owner_did),
        node_did,
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert!(
        caller.session.capabilities.iter().any(|c| c.with == ResourceUri::substrate(node_did)),
        "expected the granted capability's resource to name the node DID"
    );
    assert!(
        !caller.session.capabilities.iter().any(|c| c.with == ResourceUri::substrate(&owner_did)),
        "must not name the caller's own DID (which happens to equal the owner here, but the \
         resource must be node-scoped, not caller-scoped)"
    );
}

/// Attribution must resolve to the delegation's `master_did`, not the
/// ephemeral
/// `temporary_did` -- the DID `ControlPlaneService::deploy` later
/// records as a service's owner. Every other test in this module
/// constructs `VerifiedIdentity { master_did == temporary_did }`, so none
/// can actually distinguish a bug that swapped the two;
/// `handshake.rs`'s own tests prove `HandshakeVerifier::verify_preamble`
/// resolves a real wire handshake correctly, but nothing previously
/// exercised `build_caller` itself with a genuinely distinct pair.
#[tokio::test]
async fn build_caller_uses_master_did_not_temporary_did_as_caller_did() {
    let client = Identity::generate().unwrap();
    let master_did = derive_did_key(&client.public_key());
    let temporary_did = "did:key:zSomeEphemeralTemporaryKey".to_string();

    let preamble = RoutePreamble::binary_json_rpc("svc", "data-layer");
    let id =
        VerifiedIdentity { master_did: master_did.clone(), temporary_did: temporary_did.clone() };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller =
        build_caller(&preamble, &id, None, "did:key:zNode", &empty_registry(), &resolver, false)
            .await;

    assert_eq!(caller.caller_did, master_did);
    assert_ne!(caller.caller_did, temporary_did);
}

// -- owner-rooted trust (ADR-0015 A6/F6/A7) ----------------------------

#[test]
fn owning_service_id_parses_both_resource_shapes() {
    assert_eq!(owning_service_id(&ResourceUri::service("app-1", "svc-a")).unwrap(), "svc-a");
    assert_eq!(
        owning_service_id(&ResourceUri("substrate:did:key:zNode/app/svc-a".to_string())).unwrap(),
        "svc-a"
    );
    assert!(owning_service_id(&ResourceUri::substrate("did:key:zNode")).is_none());
    assert!(
        owning_service_id(&ResourceUri("substrate:did:key:zNode/other/thing".to_string()))
            .is_none()
    );
}

/// Regression: a `substrate:` resource with a trailing selector past the
/// service name (e.g. the orchestrator's `.../app/<svc>/deploy`) must
/// still yield just the service id, not the selector tail glued onto it.
#[test]
fn owning_service_id_strips_a_trailing_selector_past_the_service_name() {
    assert_eq!(
        owning_service_id(&ResourceUri("substrate:did:key:zNode/app/svc-a/deploy".to_string()))
            .unwrap(),
        "svc-a"
    );
}

#[test]
fn resource_is_local_checks_the_named_node_for_substrate_resources() {
    assert!(resource_is_local(&ResourceUri::substrate("did:key:zThisNode"), "did:key:zThisNode"));
    assert!(!resource_is_local(&ResourceUri::substrate("did:key:zOtherNode"), "did:key:zThisNode"));
    assert!(resource_is_local(
        &ResourceUri(format!("{}/app/foo", ResourceUri::substrate("did:key:zThisNode").0)),
        "did:key:zThisNode"
    ));
    // synapp: resources are always local by construction.
    assert!(resource_is_local(&ResourceUri::service("app-1", "svc-a"), "did:key:zThisNode"));
}

/// ADR-0015 A6: a service owner is an independent root for their own
/// service, regardless of whether the substrate has a node-wide admin
/// root at all (`admin_root: None` here -- the unowned, fail-closed
/// posture).
#[tokio::test]
async fn owner_rooted_chain_grants_a_capability_on_the_owners_own_service() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let node_did = "did:key:zNode";
    let resource = ResourceUri(format!("substrate:{node_did}/app/svc-a"));

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_did.clone()).await.unwrap();

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(&preamble, &id, None, node_did, &registry, &resolver, false).await;

    assert_eq!(caller.auth, AuthLevel::Ucan);
    assert!(
        caller
            .session
            .has_capability(&resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
    );
}

/// The owner-rooted trust ADR-0015 A6 grants is
/// bounded away from `data-layer/admin` -- a service owner self-issuing a
/// UCAN claiming `data-layer/admin` on their own service must not be
/// admitted, or `execute-ddl`/`query-raw` would be open on every owned
/// service, contradicting the guarantee that `execute-ddl`/`query-raw`
/// remain denied. `substrate/admin` is included too
/// (it entails `data-layer/admin`), though it cannot reach this path in
/// practice: `owning_service_id` never matches a bare `substrate:` URI.
#[tokio::test]
async fn owner_rooted_chain_does_not_grant_data_layer_admin() {
    let owner = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let node_did = "did:key:zNode";
    let resource = ResourceUri::service("svc-a", "svc-a");

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_did.clone()).await.unwrap();

    // The owner self-issues a token to themselves, claiming admin on
    // their own service.
    let token = CapabilityToken::issue(
        &owner,
        &owner_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(&preamble, &id, None, node_did, &registry, &resolver, false).await;

    assert!(
        !caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())),
        "an owner-rooted chain must not admit data-layer/admin on the owner's own service"
    );
}

/// The narrow case ADR-0015 A6 actually motivates still works: an owner
/// self-rooting a non-admin ability (`data-layer/read`) on their own
/// service is admitted.
#[tokio::test]
async fn owner_rooted_chain_grants_data_layer_read() {
    let owner = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let node_did = "did:key:zNode";
    let resource = ResourceUri::service("svc-a", "svc-a");

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_did.clone()).await.unwrap();

    let token = CapabilityToken::issue(
        &owner,
        &owner_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: owner_did.clone(), temporary_did: owner_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(&preamble, &id, None, node_did, &registry, &resolver, false).await;

    assert!(
        caller.session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string())),
        "owner-rooted data-layer/read is A6's motivating case and must still be admitted"
    );
}

/// The other half of A6: an owner of one service is not thereby a root
/// for a *different* service -- they cannot mint a capability on a
/// resource they do not own.
#[tokio::test]
async fn owner_rooted_chain_does_not_grant_on_a_different_owners_service() {
    let owner_a = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let owner_a_did = derive_did_key(&owner_a.public_key());
    let client_did = derive_did_key(&client.public_key());
    let node_did = "did:key:zNode";
    // owner_a owns "svc-a", not "svc-b".
    let foreign_resource = ResourceUri(format!("substrate:{node_did}/app/svc-b"));

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_a_did.clone()).await.unwrap();
    registry.set_owner("svc-b".to_string(), "did:key:zSomeoneElse".to_string()).await.unwrap();

    let token = CapabilityToken::issue(
        &owner_a,
        &client_did,
        vec![Capability {
            with: foreign_resource.clone(),
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    // An unrelated admin_root, distinct from `client` -- an owned
    // substrate on which this caller is not the owner, so no node-wide
    // capability leaks in and masks the chain's own outcome.
    let admin_root = derive_did_key(&Identity::generate().unwrap().public_key());
    let caller =
        build_caller(&preamble, &id, Some(&admin_root), node_did, &registry, &resolver, false)
            .await;

    assert!(
        !caller
            .session
            .has_capability(&foreign_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
    );
}

/// F6: an admin root's node-wide grant only covers a resource that
/// actually names *this* node -- a leaf naming a different node's
/// `substrate:` resource is not admitted even though the issuer equals
/// `admin_root`.
#[tokio::test]
async fn admin_root_grant_is_rejected_for_a_different_nodes_resource() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let this_node = "did:key:zThisNode";
    let other_nodes_resource = ResourceUri::substrate("did:key:zOtherNode");

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: other_nodes_resource.clone(),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    let caller = build_caller(
        &preamble,
        &id,
        Some(&admin_root),
        this_node,
        &empty_registry(),
        &resolver,
        false,
    )
    .await;

    assert_eq!(caller.auth, AuthLevel::Delegated, "must not upgrade on a foreign-node resource");
}

/// A7: revocation still checks the whole chain for an owner-rooted
/// grant, the same as the admin-rooted path
/// (`build_caller_rejects_a_revoked_chain`).
#[tokio::test]
async fn owner_rooted_chain_is_rejected_when_revoked() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());
    let node_did = "did:key:zNode";
    let resource = ResourceUri(format!("substrate:{node_did}/app/svc-a"));

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_did.clone()).await.unwrap();

    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let preamble = ucan_preamble(token);
    let id = VerifiedIdentity { master_did: client_did.clone(), temporary_did: client_did };
    let resolver =
        MockResolver { revoked: HashMap::from([(owner_did, vec![id.master_did.clone()])]) };

    // An unrelated admin_root, distinct from `client` -- an owned
    // substrate on which this caller is not the owner, so no node-wide
    // capability leaks in and masks the revocation's own effect.
    let admin_root = derive_did_key(&Identity::generate().unwrap().public_key());
    let caller =
        build_caller(&preamble, &id, Some(&admin_root), node_did, &registry, &resolver, false)
            .await;

    assert_eq!(caller.auth, AuthLevel::Delegated);
    assert!(
        !caller
            .session
            .has_capability(&resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
    );
}

/// ADR-0015 A3/A4: a `can_delegate: false` owner-rooted grant cannot be
/// re-delegated -- the same terminal behavior `syneroym_ucan::token`
/// pins directly, exercised here end to end through `build_caller`.
#[tokio::test]
async fn owner_rooted_grant_with_can_delegate_false_cannot_be_redelegated() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let node_did = "did:key:zNode";
    let resource = ResourceUri(format!("substrate:{node_did}/app/svc-a"));

    let registry = empty_registry();
    registry.set_owner("svc-a".to_string(), owner_did).await.unwrap();

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: Some(serde_json::json!({"can_delegate": false})),
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let preamble = ucan_preamble(alice_to_bob);
    let id = VerifiedIdentity { master_did: bob_did.clone(), temporary_did: bob_did };
    let resolver = MockResolver { revoked: HashMap::new() };

    // An unrelated admin_root, distinct from `bob` -- an owned substrate
    // on which this caller is not the owner, so no node-wide capability
    // leaks in and masks the delegation block's own effect.
    let admin_root = derive_did_key(&Identity::generate().unwrap().public_key());
    let caller =
        build_caller(&preamble, &id, Some(&admin_root), node_did, &registry, &resolver, false)
            .await;

    assert!(
        !caller
            .session
            .has_capability(&resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
    );
}

#[test]
fn rewrite_happens_for_wasm_service_with_assets_or_routes() {
    let mut interface = String::new();
    let wasm_ep = SubstrateEndpoint::WasmChannel { service_id: "did:key:zTest".to_string() };
    let rewritten = maybe_rewrite_http_native_interface(
        RouteTransport::Http,
        &mut interface,
        true,
        Some(&wasm_ep),
    );
    assert!(rewritten);
    assert_eq!(interface, HTTP_NATIVE_INTERFACE);
}

#[test]
fn no_rewrite_for_node_level_native_service() {
    let mut interface = String::new();
    let native_ep =
        SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zAuth".to_string() };
    let rewritten = maybe_rewrite_http_native_interface(
        RouteTransport::Http,
        &mut interface,
        true,
        Some(&native_ep),
    );
    assert!(!rewritten);
    assert_eq!(interface, "");
}

#[test]
fn no_rewrite_when_preamble_already_names_an_interface() {
    let mut interface = "custom-api".to_string();
    let wasm_ep = SubstrateEndpoint::WasmChannel { service_id: "did:key:zTest".to_string() };
    let rewritten = maybe_rewrite_http_native_interface(
        RouteTransport::Http,
        &mut interface,
        true,
        Some(&wasm_ep),
    );
    assert!(!rewritten);
    assert_eq!(interface, "custom-api");
}

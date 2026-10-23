#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The guest-facing durable outbox, end to end across two real substrates
//! (ADR-0023 §2, §4, §5).
//!
//! Every *property* these two cases touch already has an in-process test in
//! `syneroym-router`. What only an e2e can prove is the **sequence**: a
//! guest hands a call to its outbox, the process that owns that outbox is
//! torn down and restarted, and the call is still there afterwards and
//! still delivers. A restart with a live queue is not expressible in a
//! unit test, which is the whole reason these two exist.
//!
//! Each stage is asserted through the `proxy-outbox` operator verb --
//! queued, still queued as the *same* item after the restart, gone once
//! delivered -- rather than inferred from a side effect. That is the
//! correction an earlier review had to make to its equivalent test, and
//! copying its earlier shape would have reproduced the same weakness.
//!
//! Skips when the `proxy-test`/`greeter` wasm artifacts are absent
//! (`cargo build --target wasm32-wasip2 --release` in each), matching every
//! other guest-driven test in this tree.

use std::time::Duration;

use common::{
    SubstrateNode,
    outbox::{
        DELIVERY_ATTEMPT_BUDGET, DLQ_ATTEMPT_BUDGET, artifacts, configure_queue_node,
        dead_letter_keys, deploy_guest, guest_enqueue, outbox_keys, publish_endpoint, wait_until,
    },
};
use rustls::crypto::ring;
use syneroym_core::{dht_registry::RegistryClient, test_constants};
use syneroym_identity::{Identity, substrate};
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

/// **Why a target that is merely restarting does not dead-letter here.**
/// When the target node comes back it republishes its own endpoint record
/// before its services finish coming up, so a delivery attempt lands in a
/// window where the address is right and the service is not there yet. A
/// "service not found" answer is therefore treated as retryable rather
/// than terminal -- bounded by the ordinary attempt budget, so a target
/// that is genuinely gone still dead-letters, just not on the first hit.
/// Without that, the item would be given up on during exactly the outage
/// this queue exists to survive. This test relies on that rule from the
/// sender's side: the target's record stays unpublished through node B's
/// whole reboot, so every attempt in that time fails as "unknown service"
/// and must be retried.
///
/// The sequence no in-process test can cover: a guest queues a call to a
/// node that is down, the **calling** substrate restarts, the node comes
/// back, and the call lands -- with the outbox itself asserted at every
/// stage rather than inferred.
#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear queued call delivery after node recovery")]
async fn a_queued_guest_call_to_an_offline_node_lands_after_it_returns() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");

    let owner = Identity::generate().unwrap();

    // Node A hosts the registry and restarts below, so it must come back on
    // the same ports and under the same on-disk identity -- reuse one
    // captured builder for both boots. `caller_dir` outlives the reboot.
    let caller_dir = tempfile::tempdir().unwrap();
    let node_a_builder = SubstrateNode::builder()
        .owner(&owner)
        .base_path(caller_dir.path())
        .configure(configure_queue_node(DELIVERY_ATTEMPT_BUDGET))
        .inject_kek_bytes([0xcd; 32]);
    let mut node_a = node_a_builder.clone().boot().await;
    let shared_registry = node_a.registry_url().to_string();

    let target_dir = tempfile::tempdir().unwrap();
    let node_b = SubstrateNode::builder()
        .owner(&owner)
        .base_path(target_dir.path())
        .shared_registry(&shared_registry)
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;

    // The target: an ordinary greeter on node B, published so node A's
    // proxy can resolve it.
    let target_master = Identity::generate().unwrap();
    let target_did = substrate::derive_did_key(&target_master.public_key());
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm.clone(),
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target deploy failed");
    let node_b_info =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    let node_b_did = node_b.did().to_string();
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;

    // The caller: the guest fixture on node A, certified so `enqueue` is
    // not refused up front.
    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut node_a, &caller_master, proxy_wasm.clone()).await;

    // Node B goes down. The record stays in the registry, so the proxy
    // resolves an address and then fails to connect -- a *transport*
    // failure, which is the retryable case the outbox exists for.
    node_b.teardown().await;
    time::sleep(Duration::from_secs(1)).await;
    // Re-published while it is down, on purpose. A target whose record has
    // gone resolves to nothing, and "no such service" is *terminal* -- so
    // without this the enqueue would be refused outright rather than
    // queued, and the test would be exercising a different case than the
    // one it means to.
    publish_endpoint(&target_did, &node_b_did, node_b_info, &target_master, &shared_registry).await;

    // The guest enqueues. Fire-and-forget: it gets no delivery outcome.
    guest_enqueue(&node_a, &guest_did, &target_did, "msg-1")
        .await
        .expect("enqueue must be accepted for delivery");

    assert!(
        wait_until(Duration::from_secs(30), || async {
            outbox_keys(&node_a, &guest_did).await == vec!["msg-1".to_string()]
        })
        .await,
        "the queued call must be visible in the outbox, not merely inferred"
    );

    // A second enqueue of the same logical operation is a no-op at the
    // sender: the queue key *is* the idempotency key.
    guest_enqueue(&node_a, &guest_did, &target_did, "msg-1").await.ok();
    assert_eq!(
        outbox_keys(&node_a, &guest_did).await,
        vec!["msg-1".to_string()],
        "re-enqueueing one logical operation must not produce a second queued item"
    );

    // Restart the calling substrate -- the step no in-process test covers.
    // The reused builder brings it back on the same ports and identity.
    node_a.teardown().await;
    let mut node_a = node_a_builder.boot().await;
    assert_eq!(
        outbox_keys(&node_a, &guest_did).await,
        vec!["msg-1".to_string()],
        "the same item must still be queued across the restart, not a different or duplicated one"
    );

    // The community registry keeps its records **in memory**, so node A's
    // own restart empties the registry it hosts. The target's record is
    // deliberately *not* republished yet: until it is, every delivery
    // attempt fails at node A as "unknown service", which retries. It goes
    // back only once node B is fully up (below). A node that has just
    // booted is reachable before its KEK is injected, and it refuses a
    // keyed call in that window as permission denied, which the sender
    // treats as terminal -- so a record published any earlier lets the
    // worker dead-letter the item whenever a tick lands in that window.
    //
    // The caller master's anchor was wiped too, and the receiving node
    // needs it to verify the guest's instance certificate on every
    // delivery attempt. Without it the handshake is rejected, which is a
    // transport failure -- so the item neither lands nor dead-letters, it
    // just retries forever.
    RegistryClient::new(false, Some(shared_registry.clone()))
        .publish_master_anchor(&guest_did, vec![], None, &caller_master, true)
        .await
        .expect("failed to republish the caller master's anchor after the restart");
    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "a restart must not turn a waiting item into a dead letter"
    );

    // Node B returns: same identity and same directory, fresh ports. It
    // publishes its own substrate record while booting, but node A still
    // cannot reach it: the sender looks up the target's record first, and
    // takes node B's address from node B's own record only when the
    // target's record names it. `boot` returns once the KEK is injected.
    let node_b = SubstrateNode::builder()
        .owner(&owner)
        .base_path(target_dir.path())
        .shared_registry(&shared_registry)
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;
    // Re-deployed as well as rebooted: a substrate does not bring its
    // deployed services back up by itself, and this test is about node
    // A's outbox surviving, not about node B's deployment persistence.
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm,
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target redeploy failed");
    let listed = node_b.substrate_client.list_svcs().await.expect("list on node B failed");
    assert!(
        listed.iter().any(|svc| svc.service_id == target_did
            && svc.interfaces.contains(&test_constants::GREETER_INTERFACE_NAME.to_string())),
        "node B must know the target again before its record is republished, got: {:?}",
        listed.iter().map(|s| (&s.service_id, &s.interfaces)).collect::<Vec<_>>()
    );

    // Only now is the target resolvable again: node B holds its KEK and
    // hosts the target, so the first attempt that can reach it can also
    // land. Its fresh mechanisms, so the record describes where the
    // target is now.
    let node_b_info_after = node_b
        .substrate_client
        .lookup()
        .await
        .expect("node B lookup after reboot failed")
        .info
        .mechanisms;
    publish_endpoint(
        &target_did,
        node_b.did(),
        node_b_info_after,
        &target_master,
        &shared_registry,
    )
    .await;

    // `node_a`'s own connection sat idle through node B's full reboot,
    // redeploy, and lookup just above, long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed
    // throughout this crate's e2e tests). `outbox_keys` panics on any
    // error, so a stale connection here would abort the test with a
    // misleading "proxy-outbox failed" instead of the poll loop below ever
    // getting a chance to run -- redial once, before entering that loop,
    // rather than letting its first iteration find out the hard way.
    let _ = crate::call_with_reconnect!(
        node_a.substrate_client,
        node_a.substrate_client.proxy_outbox(guest_did.clone()).await
    );
    assert!(
        wait_until(Duration::from_secs(120), || async {
            outbox_keys(&node_a, &guest_did).await.is_empty()
        })
        .await,
        "the queued call must be delivered once its target returns; outbox still holds {:?}",
        outbox_keys(&node_a, &guest_did).await
    );
    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "a delivered call must leave through delivery, not through the dead-letter table"
    );

    // It stays delivered: nothing re-queues it on a later tick.
    time::sleep(Duration::from_secs(3)).await;
    assert!(
        outbox_keys(&node_a, &guest_did).await.is_empty(),
        "a delivered item must not reappear in the outbox"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}

/// The terminal half: a target that never comes back exhausts the attempt
/// budget, lands in the dead-letter table where an operator can see it, and
/// is replayable from there.
#[tokio::test]
async fn a_permanently_unreachable_target_lands_in_the_dlq_and_replays() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");

    let owner = Identity::generate().unwrap();

    let mut node_a = SubstrateNode::builder()
        .owner(&owner)
        .configure(configure_queue_node(DLQ_ATTEMPT_BUDGET))
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;
    let shared_registry = node_a.registry_url().to_string();

    let node_b = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(&shared_registry)
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;
    let target_master = Identity::generate().unwrap();
    let target_did = substrate::derive_did_key(&target_master.public_key());
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm,
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target deploy failed");
    let node_b_info =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    let node_b_did = node_b.did().to_string();
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;

    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut node_a, &caller_master, proxy_wasm).await;

    // Down for good this time. The record stays published, so every
    // attempt resolves an address and then fails to connect.
    node_b.teardown().await;
    time::sleep(Duration::from_secs(1)).await;
    // Re-published while down, for the same reason as the delivery case: a
    // target that resolves to nothing is terminal at the call, which is a
    // different failure than the permanently-unreachable one under test.
    publish_endpoint(&target_did, &node_b_did, node_b_info, &target_master, &shared_registry).await;

    guest_enqueue(&node_a, &guest_did, &target_did, "doomed-1")
        .await
        .expect("enqueue must be accepted for delivery");

    assert!(
        wait_until(Duration::from_secs(300), || async {
            dead_letter_keys(&node_a, &guest_did).await == vec!["doomed-1".to_string()]
        })
        .await,
        "the item must reach the dead-letter table once its budget is exhausted; outbox {:?}, \
         dead letters {:?}",
        outbox_keys(&node_a, &guest_did).await,
        dead_letter_keys(&node_a, &guest_did).await
    );
    assert!(
        outbox_keys(&node_a, &guest_did).await.is_empty(),
        "a dead-lettered item must have left the outbox"
    );

    // Replay puts it back on the outbox for another attempt -- it never
    // executes inline, so the dead letter is consumed and a queued item
    // appears in its place.
    let dead = node_a
        .substrate_client
        .proxy_dead_letters(guest_did.clone())
        .await
        .expect("proxy-dead-letters failed");
    assert_eq!(dead.len(), 1);
    node_a
        .substrate_client
        .proxy_replay(guest_did.clone(), dead[0].id)
        .await
        .expect("proxy-replay failed");

    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "replay must consume the dead letter"
    );
    assert!(
        wait_until(Duration::from_secs(10), || async {
            !outbox_keys(&node_a, &guest_did).await.is_empty()
                || !dead_letter_keys(&node_a, &guest_did).await.is_empty()
        })
        .await,
        "replay must re-enqueue the call rather than discard it"
    );

    node_a.teardown().await;
}

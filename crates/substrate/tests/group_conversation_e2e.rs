#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Group messaging across three real `syneroym-substrate` instances
//! (Alice, Bob, Charlie) — testing group creation, key distribution,
//! multi-peer delivery, DAG sync, epoch rekeying, and membership changes.

use std::{
    fs,
    time::{Duration, Instant},
};

use common::SubstrateNode;
use ed25519_dalek::VerifyingKey;
use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{
    config::AppSandboxRole,
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    test_constants,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::SyneroymClient;
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

const FIXTURE_INTERFACE: &str = "syneroym-test:dual-build-fixture/test-driver@0.1.0";

fn fast_conversation_role() -> AppSandboxRole {
    AppSandboxRole {
        conversation_tick_secs: 1,
        conversation_group_sync_secs: 1,
        conversation_group_rekey_secs: 10,
        ..AppSandboxRole::default()
    }
}

async fn publish_endpoint(
    service_id: &str,
    substrate_id: &str,
    mechanisms: Vec<EndpointMechanism>,
    signer: &Identity,
    registry_url: &str,
) {
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: substrate_id.to_string(),
        endpoint_type: EndpointType::Service,
        nickname: None,
        mechanisms,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    };
    let signed = info.sign(signer).unwrap();
    let res = Client::new()
        .post(format!("{registry_url}/register"))
        .json(&signed)
        .send()
        .await
        .expect("registry register request failed");
    assert!(res.status().is_success(), "registry rejected the record: {:?}", res.text().await);

    let readback = wait_until(Duration::from_secs(20), || {
        let url = format!("{registry_url}/lookup/{service_id}");
        async move { Client::new().get(&url).send().await.is_ok_and(|r| r.status().is_success()) }
    })
    .await;
    assert!(readback, "the registry never served back the record for {service_id}");
}

async fn deploy_fixture(node: &mut SubstrateNode, master: &Identity, wasm: Vec<u8>) -> String {
    let service_id = substrate::derive_did_key(&master.public_key());
    let identity = crate::call_with_reconnect!(
        node.substrate_client,
        node.substrate_client.instance_identity(&service_id).await
    );
    let pubkey_bytes: [u8; 32] = hex::decode(&identity.pubkey_hex)
        .expect("instance pubkey is not hex")
        .try_into()
        .expect("instance pubkey is not 32 bytes");
    let instance_pubkey = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
    let cert = DelegationCertificate::issue(
        master,
        instance_pubkey,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();

    node.substrate_client
        .deploy_svc_wasm(
            service_id.clone(),
            vec![FIXTURE_INTERFACE.to_string()],
            wasm,
            syneroym_sdk::Publication::Private,
            Some(cert),
        )
        .await
        .expect("fixture deploy failed");

    publish_master_anchor(&service_id, master, node.registry_url()).await;

    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, node.registry_url()).await;
    service_id
}

async fn publish_master_anchor(service_id: &str, master: &Identity, registry_url: &str) {
    RegistryClient::new(false, Some(registry_url.to_string()))
        .publish_master_anchor(service_id, vec![], None, master, true)
        .await
        .expect("failed to publish the master anchor");
}

async fn fixture_run(node: &SubstrateNode, service_id: &str, request: &Value) -> Value {
    let mut client = SyneroymClient::new_with_identity(
        service_id.to_string(),
        node.registry_url().to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    client.connect().await.expect("connect failed");
    let response = client
        .request(FIXTURE_INTERFACE, "run", json!([request.to_string()]))
        .await
        .expect("run request failed");
    client.shutdown().await.ok();
    let payload: Value = response.result;
    let raw = payload.as_str().expect("test-driver::run must return a JSON string");
    serde_json::from_str(raw).expect("fixture response is not valid JSON")
}

async fn wait_until<F, Fut>(budget: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if check().await {
            return true;
        }
        time::sleep(Duration::from_millis(1500)).await;
    }
    false
}

fn fixture_wasm() -> Option<Vec<u8>> {
    fs::read(test_constants::dual_build_fixture_wasm_path()).ok()
}

#[tokio::test]
async fn three_members_converge_to_byte_identical_transcripts() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = fixture_wasm().expect("dual-build-fixture wasm artifact not built");

    let owner = Identity::generate().unwrap();

    let mut node_a = SubstrateNode::builder()
        .owner(&owner)
        .inject_kek_bytes([0xcd; 32])
        .configure(|c| c.roles.app_sandbox = Some(fast_conversation_role()))
        .boot()
        .await;
    let shared_registry = node_a.registry_url().to_string();
    let shared_relay = node_a.relay_url().to_string();

    let mut node_b = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek_bytes([0xcd; 32])
        .configure(|c| c.roles.app_sandbox = Some(fast_conversation_role()))
        .boot()
        .await;

    let mut node_c = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek_bytes([0xcd; 32])
        .configure(|c| c.roles.app_sandbox = Some(fast_conversation_role()))
        .boot()
        .await;

    let master_a = Identity::generate().unwrap();
    let master_b = Identity::generate().unwrap();
    let master_c = Identity::generate().unwrap();

    let did_a = deploy_fixture(&mut node_a, &master_a, wasm.clone()).await;
    let did_b = deploy_fixture(&mut node_b, &master_b, wasm.clone()).await;
    let did_c = deploy_fixture(&mut node_c, &master_c, wasm.clone()).await;

    // Node A creates a group
    let create_res = fixture_run(&node_a, &did_a, &json!({"op": "create-group"})).await;
    let group_id = create_res["ok"]["conversation"]
        .as_str()
        .expect("create-group must return conversation id")
        .to_string();

    // Node A adds B and C
    let add_b_res = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "add-member", "conversation": group_id, "member_address": did_b}),
    )
    .await;
    assert_eq!(add_b_res["ok"]["added"], true);

    let add_c_res = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "add-member", "conversation": group_id, "member_address": did_c}),
    )
    .await;
    assert_eq!(add_c_res["ok"]["added"], true);

    // Wait for key delivery to settle
    time::sleep(Duration::from_secs(4)).await;

    // All three post messages to the group
    let send_a = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "send-message", "conversation": group_id, "body": "msg from Alice"}),
    )
    .await;
    assert!(send_a["ok"]["message"].is_string());

    let send_b = fixture_run(
        &node_b,
        &did_b,
        &json!({"op": "send-message", "conversation": group_id, "body": "msg from Bob"}),
    )
    .await;
    assert!(send_b["ok"]["message"].is_string());

    let send_c = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "send-message", "conversation": group_id, "body": "msg from Charlie"}),
    )
    .await;
    assert!(send_c["ok"]["message"].is_string());

    // Prompt sync on each
    let _ =
        fixture_run(&node_a, &did_a, &json!({"op": "sync-now", "conversation": group_id})).await;
    let _ =
        fixture_run(&node_b, &did_b, &json!({"op": "sync-now", "conversation": group_id})).await;
    let _ =
        fixture_run(&node_c, &did_c, &json!({"op": "sync-now", "conversation": group_id})).await;

    // Wait until all 3 nodes have 3 messages in history *and* 3 membership
    // events -- the two DAGs are gossiped over the same sync-now round but
    // are separate entry chains, so message convergence alone does not
    // guarantee membership-history has caught up too.
    let converged = wait_until(Duration::from_secs(30), || {
        let node_a = &node_a;
        let node_b = &node_b;
        let node_c = &node_c;
        let did_a = did_a.clone();
        let did_b = did_b.clone();
        let did_c = did_c.clone();
        let group_id = group_id.clone();
        async move {
            let _ =
                fixture_run(node_a, &did_a, &json!({"op": "sync-now", "conversation": group_id}))
                    .await;
            let _ =
                fixture_run(node_b, &did_b, &json!({"op": "sync-now", "conversation": group_id}))
                    .await;
            let _ =
                fixture_run(node_c, &did_c, &json!({"op": "sync-now", "conversation": group_id}))
                    .await;
            let hist_a = fixture_run(
                node_a,
                &did_a,
                &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
            )
            .await;
            let hist_b = fixture_run(
                node_b,
                &did_b,
                &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
            )
            .await;
            let hist_c = fixture_run(
                node_c,
                &did_c,
                &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
            )
            .await;
            let len_a = hist_a["ok"]["messages"].as_array().map_or(0, |v| v.len());
            let len_b = hist_b["ok"]["messages"].as_array().map_or(0, |v| v.len());
            let len_c = hist_c["ok"]["messages"].as_array().map_or(0, |v| v.len());

            let mem_a = fixture_run(
                node_a,
                &did_a,
                &json!({"op": "membership-history", "conversation": group_id}),
            )
            .await;
            let mem_b = fixture_run(
                node_b,
                &did_b,
                &json!({"op": "membership-history", "conversation": group_id}),
            )
            .await;
            let mem_c = fixture_run(
                node_c,
                &did_c,
                &json!({"op": "membership-history", "conversation": group_id}),
            )
            .await;
            let mlen_a = mem_a["ok"]["history"].as_array().map_or(0, |v| v.len());
            let mlen_b = mem_b["ok"]["history"].as_array().map_or(0, |v| v.len());
            let mlen_c = mem_c["ok"]["history"].as_array().map_or(0, |v| v.len());

            len_a == 3 && len_b == 3 && len_c == 3 && mlen_a == 3 && mlen_b == 3 && mlen_c == 3
        }
    })
    .await;
    assert!(converged, "all three nodes must converge to 3 messages and 3 membership events");

    // Read histories and compare message bodies and order
    let hist_a = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;
    let hist_b = fixture_run(
        &node_b,
        &did_b,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;
    let hist_c = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;

    let msgs_a: Vec<&str> = hist_a["ok"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    let msgs_b: Vec<&str> = hist_b["ok"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    let msgs_c: Vec<&str> = hist_c["ok"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();

    assert_eq!(msgs_a, msgs_b, "histories of A and B must match exactly");
    assert_eq!(msgs_b, msgs_c, "histories of B and C must match exactly");

    // Membership histories match too
    let mem_a = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "membership-history", "conversation": group_id}),
    )
    .await;
    let mem_b = fixture_run(
        &node_b,
        &did_b,
        &json!({"op": "membership-history", "conversation": group_id}),
    )
    .await;
    let mem_c = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "membership-history", "conversation": group_id}),
    )
    .await;

    // Three membership events so far: Alice's genesis add, Bob's add, Charlie's
    // add. A vacuous comparison (three empty arrays, or three `null`s from a
    // failed call) must not pass — this is what let the B5-05 `member_list_hash`
    // regression through undetected.
    let mem_a_events = mem_a["ok"]["history"].as_array().expect("membership-history events on A");
    assert_eq!(mem_a_events.len(), 3, "A's membership history: {mem_a:?}");
    assert_eq!(mem_a["ok"]["history"], mem_b["ok"]["history"]);
    assert_eq!(mem_b["ok"]["history"], mem_c["ok"]["history"]);

    // No durable group content on the pub/sub broker
    let broker_a = fixture_run(&node_a, &did_a, &json!({"op": "read-inbox"})).await;
    assert_eq!(broker_a["ok"]["entries"].as_array().unwrap().len(), 0);

    // Step 11: Alice and Bob exchange a message while Charlie does not call
    // `sync-now` itself. This is not a true "Charlie is offline" simulation —
    // Charlie's own node keeps running its periodic background sync pass
    // (`conversation_group_sync_secs: 1` in this fixture), so it can pick the
    // message up on its own before the explicit `sync-now` below runs. What
    // this step actually verifies is eventual consistency: Charlie ends up
    // with the message one way or the other, not specifically that it arrived
    // via a pull rather than a push.
    let send_a2 = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "send-message", "conversation": group_id, "body": "msg from Alice while Charlie is offline"}),
    )
    .await;
    assert!(send_a2["ok"]["message"].is_string());

    // Prompt sync only between Alice and Bob
    let _ =
        fixture_run(&node_b, &did_b, &json!({"op": "sync-now", "conversation": group_id})).await;
    let hist_b2 = fixture_run(
        &node_b,
        &did_b,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;
    assert_eq!(hist_b2["ok"]["messages"].as_array().unwrap().len(), 4);

    // Charlie now syncs and catches up to 4 messages
    let _ =
        fixture_run(&node_c, &did_c, &json!({"op": "sync-now", "conversation": group_id})).await;
    let hist_c2 = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;
    assert_eq!(hist_c2["ok"]["messages"].as_array().unwrap().len(), 4);

    // Step 12: Owner Alice removes Charlie and rekeys
    let remove_c = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "remove-member", "conversation": group_id, "member_address": did_c}),
    )
    .await;
    assert!(remove_c["ok"].is_object(), "remove-member should succeed: {remove_c:?}");

    // Bob syncs to learn of Charlie's removal and new epoch
    let _ =
        fixture_run(&node_b, &did_b, &json!({"op": "sync-now", "conversation": group_id})).await;

    // Alice sends post-removal message in the new epoch
    let send_a_post = fixture_run(
        &node_a,
        &did_a,
        &json!({"op": "send-message", "conversation": group_id, "body": "secret post-removal message"}),
    )
    .await;
    assert!(send_a_post["ok"]["message"].is_string());

    // Charlie syncs first — deterministically, rather than relying on the
    // background pass's timing — so the send attempt below is a genuine test
    // of the post-removal refusal, not a race against whether Charlie's own
    // periodic sync happened to run first.
    let _ =
        fixture_run(&node_c, &did_c, &json!({"op": "sync-now", "conversation": group_id})).await;

    // Charlie attempts to send into group post-removal — must be refused, not
    // merely unobserved by the other members.
    let send_c_post = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "send-message", "conversation": group_id, "body": "unauthorized msg from Charlie"}),
    )
    .await;
    assert!(
        send_c_post.get("err").is_some(),
        "a removed member must not be able to post: {send_c_post:?}"
    );
    // When Charlie tries to sync or read, Charlie cannot read the post-removal
    // message
    let _ =
        fixture_run(&node_c, &did_c, &json!({"op": "sync-now", "conversation": group_id})).await;
    let hist_c_final = fixture_run(
        &node_c,
        &did_c,
        &json!({"op": "read-history", "conversation": group_id, "limit": 10}),
    )
    .await;
    // Charlie history remains at 4 (or does not contain post-removal secret)
    let final_msgs: Vec<&str> = hist_c_final["ok"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !final_msgs.contains(&"secret post-removal message"),
        "Charlie must not read post-removal message"
    );

    node_c.teardown().await;
    node_b.teardown().await;
    node_a.teardown().await;
}

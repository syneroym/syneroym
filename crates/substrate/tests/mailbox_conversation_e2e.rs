#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Conversation messages to a peer that cannot be reached, end to end
//! across real substrates (ADR-0026): the message shows `deposited` while a
//! sealed copy waits at the peer's mailbox, and `delivered` once the peer
//! signs a receipt. The peer starts again and its own mailbox receiver pulls
//! the message (the tests ask for the pull instead of waiting for the timer).
//!
//! The first exchange between two peers is made while both are up, so a
//! session exists: first contact with an unreachable peer stays live.
//! The sender asks about a deposit once a minute, so a test that needs the
//! answer waits about that long.

use std::{path::PathBuf, time::Duration};

use common::{
    SubstrateNode,
    conversation_fixture::{
        FIXTURE_INTERFACE, fixture_run, fixture_wasm, history_messages, message_row,
        open_conversation, publish_master_anchor, send_message, wait_for_state,
    },
    conversation_peer::Peer,
    mailbox_receiver::MailboxReceiver,
    roym::{fast_conversation_role, wait_until},
};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::config::{AppSandboxRole, MailboxRole};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_observability::MemoryRecorder;
use syneroym_roym_core::{
    card::{MAILBOX_CARD_BODY_BYTES, card_body},
    deal::{Answers, DealTerms},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{QUOTE_VERSION, RECORD_QUOTE},
};
use syneroym_signed_record::{Envelope, RecordDraft};
use tempfile::TempDir;
use tokio::time;

mod common;

const KEK: [u8; 32] = [0xcd; 32];
const DELIVER_BUDGET: Duration = Duration::from_secs(30);
const DEPOSIT_BUDGET: Duration = Duration::from_secs(60);
/// The sender asks about a deposit once a minute.
const ANSWER_BUDGET: Duration = Duration::from_secs(150);

fn role() -> AppSandboxRole {
    fast_conversation_role(AppSandboxRole::default().conversation_max_pending_age_secs)
}

struct World {
    /// Hosts the registry.
    a: Peer,
    mailbox: SubstrateNode,
    /// The peer with a mailbox, until it is stopped.
    b: Option<Peer>,
    b_receiver: MailboxReceiver,
    /// What B needs to come back: its owner and its directory.
    b_owner: Identity,
    b_dir: TempDir,
    wasm: Vec<u8>,
}

/// B's node: the same owner and directory every time, new ports.
async fn boot_b(owner: &Identity, dir: PathBuf, registry: &str) -> SubstrateNode {
    SubstrateNode::builder()
        .owner(owner)
        .base_path(dir)
        .shared_registry(registry)
        .inject_kek_bytes(KEK)
        .configure(|c| c.roles.app_sandbox = Some(role()))
        .boot()
        .await
}

/// How many messages this process has parked at a mailbox so far.
fn deposits_so_far() -> u64 {
    MemoryRecorder::global()
        .expect("the substrate under test installs the recorder")
        .snapshot()
        .counters
        .get("substrate.conversation.outbox.deposited")
        .copied()
        .unwrap_or(0)
}

/// How many times mailbox deposit sealing has failed.
fn deposit_failed_so_far() -> u64 {
    MemoryRecorder::global()
        .expect("the substrate under test installs the recorder")
        .snapshot()
        .counters
        .get("substrate.proxy.mailbox.deposit_failed")
        .copied()
        .unwrap_or(0)
}

async fn world() -> World {
    let _ = ring::default_provider().install_default();
    let wasm = fixture_wasm().expect("dual-build-fixture wasm artifact not built");
    let a = Peer::boot(None, role(), &wasm).await;
    let mailbox = SubstrateNode::builder()
        .owner(&Identity::generate().unwrap())
        .shared_registry(a.registry())
        .configure(|c| {
            // The default visibility time: a message claimed too early after
            // a restart would stay hidden past every budget here.
            c.roles.mailbox = Some(MailboxRole {
                sweep_interval_secs: 1,
                republish_interval_secs: 2,
                ..Default::default()
            });
        })
        .inject_kek_bytes(KEK)
        .boot()
        .await;

    // B: the fixture deployed public, with the mailbox in its record. Its
    // own receiver registers the queue.
    let b_receiver = MailboxReceiver::new();
    let b_owner = Identity::generate().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let b_node = boot_b(&b_owner, b_dir.path().to_path_buf(), a.registry()).await;
    let did = b_receiver.deploy(&b_node, mailbox.did(), FIXTURE_INTERFACE, wasm.clone()).await;
    b_node.substrate_client.mailbox_pull_now().await.expect("B's first pass");
    // The deploy publishes B's record; B's replies need its anchor in A's
    // registry too, which `deploy` did.
    let master = Identity::from_bytes(&b_receiver.master.to_bytes());
    publish_master_anchor(&did, &master, a.registry()).await;
    let b = Peer { node: b_node, master, did };
    World { a, mailbox, b: Some(b), b_receiver, b_owner, b_dir, wasm }
}

impl World {
    fn b(&self) -> &Peer {
        self.b.as_ref().expect("B is running")
    }

    async fn stop_b(&mut self) {
        self.b.take().expect("B is running").teardown().await;
        time::sleep(Duration::from_secs(1)).await;
    }

    /// B comes back with the same identity and directory, its service
    /// deployed again, and one pass of its mailbox receiver.
    async fn restart_b_and_pull(&mut self) {
        let node = boot_b(&self.b_owner, self.b_dir.path().to_path_buf(), self.a.registry()).await;
        let did = self
            .b_receiver
            .deploy(&node, self.mailbox.did(), FIXTURE_INTERFACE, self.wasm.clone())
            .await;
        node.substrate_client.mailbox_pull_now().await.expect("B's pass after the restart");
        let master = Identity::from_bytes(&self.b_receiver.master.to_bytes());
        publish_master_anchor(&did, &master, self.a.registry()).await;
        self.b = Some(Peer { node, master, did });
    }

    async fn teardown(mut self) {
        if let Some(b) = self.b.take() {
            b.teardown().await;
        }
        self.mailbox.teardown().await;
        self.a.teardown().await;
    }
}

#[tokio::test]
async fn a_message_to_a_stopped_peer_is_delivered_by_the_peer_when_it_starts() {
    let mut world = world().await;
    let conv = open_conversation(&world.a.node, &world.a.did, &world.b().did).await;
    let first = send_message(&world.a.node, &world.a.did, &conv, "first, while both are up").await;
    assert!(wait_for_state(&world.a.node, &world.a.did, &first, "delivered", DELIVER_BUDGET).await);

    world.stop_b().await;
    let message = send_message(&world.a.node, &world.a.did, &conv, "while you are away").await;
    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "deposited", DEPOSIT_BUDGET).await,
        "the message must wait at the mailbox"
    );

    // The peer starts again, finds the message at its mailbox and says it
    // delivered it.
    world.restart_b_and_pull().await;

    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "delivered", ANSWER_BUDGET).await,
        "a receipt the peer signed must end it"
    );
    world.teardown().await;
}

#[tokio::test]
async fn a_group_push_from_the_mailbox_reaches_the_member() {
    let mut world = world().await;
    let c = Peer::boot(Some(world.a.registry()), role(), &world.wasm).await;
    let (b_did, c_did) = (world.b().did.clone(), c.did.clone());

    let created = fixture_run(&world.a.node, &world.a.did, &json!({"op": "create-group"})).await;
    let group = created["ok"]["conversation"].as_str().unwrap().to_string();
    for member in [&b_did, &c_did] {
        let added = fixture_run(
            &world.a.node,
            &world.a.did,
            &json!({"op": "add-member", "conversation": group, "member_address": member}),
        )
        .await;
        assert_eq!(added["ok"]["added"], true, "{added}");
    }
    // The group key reaches both members while they are up.
    time::sleep(Duration::from_secs(4)).await;

    world.stop_b().await;
    let deposits_before = deposits_so_far();
    let message = send_message(&world.a.node, &world.a.did, &group, "to the whole group").await;

    // The member that is up gets it, and the message stays pending while the
    // other member's copy waits at its mailbox.
    assert!(
        wait_until(DELIVER_BUDGET, || async {
            history_messages(&c.node, &c_did, &group).await.iter().any(|m| m["id"] == message)
        })
        .await,
        "the member that is up must receive the push"
    );
    assert!(
        wait_until(DEPOSIT_BUDGET, || async { deposits_so_far() > deposits_before }).await,
        "the push to the stopped member must wait at its mailbox"
    );

    // One member's copy waits at a mailbox, so the group message is not
    // delivered yet; `deposited` is a state of the member's copy only.
    let row = message_row(&world.a.node, &world.a.did, &group, &message).await.unwrap();
    assert_eq!(row["state"], "pending", "{row}");

    world.restart_b_and_pull().await;

    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "delivered", ANSWER_BUDGET).await,
        "every member settled, so the message is delivered"
    );
    c.teardown().await;
    world.teardown().await;
}

fn make_real_quote_card_body(target_bytes: usize) -> String {
    let signer = Identity::generate().unwrap();
    let issuer = derive_did_key(&signer.public_key());
    let now = 1_700_000_000;
    let exp_secs = now + 3600;

    let build = |pad_len: usize| -> (String, usize) {
        let padding = "x".repeat(pad_len);
        let terms_val = json!({
            "currency": "EUR",
            "quote_expires_at_secs": exp_secs,
            "scope": { "lang": "en", "text": padding },
            "cancellation_terms": { "lang": "en", "text": "Cancel 24h prior" },
            "refund_terms": { "lang": "en", "text": "Full refund" },
            "dispute_path": { "lang": "en", "text": "disputes@example.com" },
            "commitments": [
                {
                    "commitment_id": "c1",
                    "giver": "quote-author",
                    "resource_kind": "goods-or-service",
                    "kind": "fulfilment",
                    "description": { "lang": "en", "text": "Task description" },
                    "declared_value_minor": 50_000,
                    "after": [],
                },
                {
                    "commitment_id": "c2",
                    "giver": "counterparty",
                    "resource_kind": "currency",
                    "kind": "currency",
                    "description": { "lang": "en", "text": "Payment" },
                    "declared_value_minor": 50_000,
                    "payee": "did:key:z6Mks4PayeePlaceholderToReachExactLength53Chars",
                    "payment_methods": ["card", "cash"],
                    "tax_minor": 5_000,
                    "fees_minor": 2_000,
                    "after": ["c1"],
                }
            ]
        });
        let terms: DealTerms = serde_json::from_value(terms_val).unwrap();
        let plain = PlainQuote {
            quote_id: "quo_z6Mks4PlaceholderQuoteIdToFitFixedFieldLengthZBase32".to_string(),
            conversation: "conv_z6Mks4PlaceholderConversationIdFixedLength".to_string(),
            sequence: 1,
            answers: Answers::Enquiry {
                record_id: "rec_z6Mks4PlaceholderRecordIdToFitFixedLength".to_string(),
            },
            counterparty_did: "did:key:z6Mks4PlaceholderCounterpartyDidKeyToFit".to_string(),
            terms,
        };
        let (payload, disclosures) = conceal_quote(plain, 1);
        let payload_val = serde_json::to_value(&payload).unwrap();
        let draft = RecordDraft {
            version: QUOTE_VERSION,
            record_type: RECORD_QUOTE.to_string(),
            subject: "quo_z6Mks4PlaceholderQuoteIdToFitFixedFieldLengthZBase32".to_string(),
            payload: payload_val,
            expires_at_secs: Some(exp_secs),
            supersedes: None,
        };
        let (mut env, bytes) = Envelope::unsigned(draft, issuer.clone(), None, now).unwrap();
        let sig = z32::encode(&signer.sign(&bytes).to_bytes());
        env.attach_signature(sig).unwrap();
        let env_json = env.to_json().unwrap();
        let body = card_body(RECORD_QUOTE, QUOTE_VERSION, &env_json, &disclosures).unwrap();
        let len = body.len();
        (body, len)
    };

    let mut low = 0;
    let mut high = target_bytes;
    let mut best = build(low).0;
    while low <= high {
        let mid = (low + high) / 2;
        let (candidate, len) = build(mid);
        if len <= target_bytes {
            best = candidate;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    best
}

#[tokio::test]
async fn a_card_of_the_mailbox_size_waits_at_the_mailbox_and_a_larger_one_at_the_sender() {
    let mut world = world().await;
    let conv = open_conversation(&world.a.node, &world.a.did, &world.b().did).await;
    let first = send_message(&world.a.node, &world.a.did, &conv, "first, while both are up").await;
    assert!(wait_for_state(&world.a.node, &world.a.did, &first, "delivered", DELIVER_BUDGET).await);

    world.stop_b().await;

    // Send a real quote card of at most MAILBOX_CARD_BODY_BYTES and within 64 bytes
    // of it
    let target = MAILBOX_CARD_BODY_BYTES;
    let fit_body = make_real_quote_card_body(target);
    assert!(
        fit_body.len() <= target && fit_body.len() >= target.saturating_sub(64),
        "fit body length {} not within 64 bytes of target {}",
        fit_body.len(),
        target
    );

    let deposits_before = deposits_so_far();
    let fit_msg = send_message(&world.a.node, &world.a.did, &conv, &fit_body).await;
    assert!(
        wait_for_state(&world.a.node, &world.a.did, &fit_msg, "deposited", DEPOSIT_BUDGET).await,
        "fitting card must wait at the mailbox"
    );
    assert_eq!(deposits_so_far(), deposits_before + 1);

    // Send a real quote card ~2 KiB larger
    let failed_before = deposit_failed_so_far();
    let large_body = make_real_quote_card_body(target + 2048);
    let large_msg = send_message(&world.a.node, &world.a.did, &conv, &large_body).await;

    assert!(
        wait_until(DEPOSIT_BUDGET, || async { deposit_failed_so_far() > failed_before }).await,
        "larger card must fail mailbox deposit"
    );

    let row = message_row(&world.a.node, &world.a.did, &conv, &large_msg).await.unwrap();
    assert_eq!(row["state"], "pending", "{row}");
    assert_eq!(deposits_so_far(), deposits_before + 1);

    // B restarts; both messages reach delivered
    world.restart_b_and_pull().await;

    assert!(
        wait_for_state(&world.a.node, &world.a.did, &fit_msg, "delivered", ANSWER_BUDGET).await,
        "first message must be delivered from mailbox"
    );
    assert!(
        wait_for_state(&world.a.node, &world.a.did, &large_msg, "delivered", ANSWER_BUDGET).await,
        "second message must be delivered by outbox direct retry"
    );

    world.teardown().await;
}

//! Card sync follows the host's feed of newly visible messages, so a card
//! whose admission is answered after a later card was already read is still
//! filed, and a full sync reads everything again.

use serde_json::{Value, json};
use syneroym_roym_core::transaction;

use super::{fixtures::*, helpers::*};

/// Puts a card in both stores as a message the app has not answered yet.
async fn stash_unanswered_card(h: &Harness, id: &str, conv: &str, env: &str, ts: i64) {
    let svc = did_for_service("conversation");
    let card = inbound_card(
        id,
        conv,
        &peer_did(),
        ts,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        env,
    );
    for stack in [&h.wasm_conversation, &h.native_conversation] {
        let store = stack.store_for(&svc).await.unwrap();
        let guard = store.conn().lock().unwrap();
        let tx = guard.unchecked_transaction().unwrap();
        store
            .insert_incoming_if_absent(
                &tx,
                conv,
                id,
                &peer_did(),
                ts,
                &card.content_type,
                &card.body,
                &[0u8; 64],
                ts,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
    }
}

/// Makes the stashed card due, then lets the host ask the app about it.
async fn answer_stashed_card(h: &Harness, id: &str) {
    let svc = did_for_service("conversation");
    for stack in [&h.wasm_conversation, &h.native_conversation] {
        let store = stack.store_for(&svc).await.unwrap();
        store.update_undecided_retry(id, 0, 0).unwrap();
    }
    h.ask_undecided_now().await;
}

async fn deliver_request(h: &Harness, id: &str, conv: &str, env: &str) {
    deliver_peer_card(
        h,
        id,
        conv,
        &peer_did(),
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        env,
    )
    .await;
}

async fn sync(h: &Harness, conv: &str, full: bool) -> Value {
    let (w, n) =
        both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": full })).await;
    assert_eq!(w["result"]["filed"], n["result"]["filed"], "both builds file the same cards");
    w["result"].clone()
}

#[tokio::test]
async fn scenario_222_a_card_answered_after_a_later_one_was_read_is_still_filed() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    let (_, first) = peer_signed_request(&conv, 1, 1_000_000);
    let (_, late) = peer_signed_request(&conv, 2, 1_000_000);
    let (_, last) = peer_signed_request(&conv, 3, 1_000_000);

    // The second card arrives first but is not answered; the third is read
    // before it is.
    stash_unanswered_card(&h, "m-late", &conv, &late, 1_005).await;
    deliver_request(&h, "m-first", &conv, &first).await;
    deliver_request(&h, "m-last", &conv, &last).await;
    assert_eq!(sync(&h, &conv, false).await["filed"], 2);

    answer_stashed_card(&h, "m-late").await;
    assert_eq!(sync(&h, &conv, false).await["filed"], 1, "the late card must not be skipped");
    assert_eq!(sync(&h, &conv, false).await["filed"], 0, "nothing is filed twice");
}

#[tokio::test]
async fn scenario_223_a_full_sync_reads_every_card_again_and_files_nothing_twice() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    for seq in 1..=3u32 {
        let (_, env) = peer_signed_request(&conv, seq, 1_000_000);
        deliver_request(&h, &format!("m-{seq}"), &conv, &env).await;
    }
    assert_eq!(sync(&h, &conv, false).await["filed"], 3);

    let again = sync(&h, &conv, true).await;
    assert_eq!(again["scanned"], 3, "a full sync starts from the beginning");
    assert_eq!(again["filed"], 0);
}

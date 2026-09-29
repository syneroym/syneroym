//! Multi-node Roym flows shared by the directory, booking and trust e2e
//! tests: the consumer's client loop, a stranger's wire call, and the
//! request -> quote -> accept -> booking -> payment -> fulfilment steps.
//! Each is a step a test reads as one line, parameterised only by what
//! differs between the tests that use it.

use std::time::Duration;

use serde_json::{Value, json};
use syneroym_identity::Identity;
use syneroym_roym_core::transaction::DEFAULT_DATA_USE_NOTICE;
use syneroym_sdk::SyneroymClient;

use super::roym::{RoymNode as Node, wait_delivered, wait_until};

pub const DIRECTORY_INTERFACE: &str = "syneroym-roym:directory/api@0.1.0";

/// One JSON-RPC `invoke` frame delivered to `target_did`'s directory
/// interface over a real QUIC stream, from a freshly generated identity
/// with no delegation. The connection key is still verified by the
/// handshake, so the router reads `CallerOrigin::Verified(<generated
/// did>)` -- a stranger, not an anonymous caller. A truly key-less
/// `Anonymous` wire caller cannot be expressed over iroh; the parity
/// suite covers that arm with `AuthLevel::System`.
/// Returns the inner `envelope::Response`-shaped value the guest produced.
pub async fn stranger_wire_invoke(
    registry_url: &str,
    target_did: &str,
    method: &str,
    params: Value,
) -> Value {
    let frame = json!({ "method": method, "params": params }).to_string();
    let mut client = SyneroymClient::new_with_identity(
        target_did.to_string(),
        registry_url.to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    client.connect().await.expect("anonymous caller failed to connect to the directory");
    let resp = client
        .request(DIRECTORY_INTERFACE, "invoke", json!([frame]))
        .await
        .expect("anonymous invoke returned a wire error");
    let _ = client.shutdown().await;
    let payload = resp.result.as_str().expect("guest returns a JSON string").to_string();
    serde_json::from_str(&payload).expect("guest payload is JSON")
}

pub fn hits(result: &Value) -> Vec<Value> {
    result["hits"].as_array().cloned().unwrap_or_default()
}

/// Drive the consumer client loop the way `roymctl roym directory find` and
/// the Hub do: `start-run`, one `query-source` per source (respecting
/// `max_concurrency`), then `merge`. Returns `(run_id, merge_result)`.
pub async fn run_client_loop(node: &Node, query: Value) -> (String, Value) {
    let start = node.rpc_ok("directory.start-run", json!({})).await;
    let run_id = start["run_id"].as_str().unwrap().to_string();
    let sources: Vec<String> = start["sources"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let max_concurrency = start["max_concurrency"].as_u64().unwrap_or(1).max(1) as usize;
    for chunk in sources.chunks(max_concurrency) {
        for source in chunk {
            let _ = node
                .rpc(
                    "directory.query-source",
                    json!({ "run_id": run_id, "source": source, "query": query }),
                )
                .await;
        }
    }
    let merged = node.rpc_ok("directory.merge", json!({ "run_id": run_id })).await;
    (run_id, merged)
}

pub async fn listing_envelope(node: &Node, listing_id: &str) -> String {
    let row = node.rpc_ok("listing.get", json!({ "listing_id": listing_id })).await;
    row["envelope"].as_str().unwrap().to_string()
}

/// `conversation.open` on a raw address with no contact entry, one message
/// sent and driven to `delivered` with retries.
pub async fn deliver_one_message(from: &Node, to_label: &str, address: &str, body: &str) {
    let opened = from.rpc_ok("conversation.open", json!({ "address": address })).await;
    let conv = opened["conversation_id"].as_str().unwrap().to_string();
    let sent =
        from.rpc_ok("conversation.send", json!({ "conversation": conv, "body": body })).await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();
    assert_eq!(sent["state"], "pending", "born pending, from the host");
    let delivered = wait_until(Duration::from_secs(150), || {
        let (from, message_id) = (from, message_id.clone());
        async move {
            let _ = from.rpc("conversation.retry", json!({ "message_id": message_id })).await;
            let s = from
                .rpc_ok("conversation.delivery-status", json!({ "message_id": message_id }))
                .await;
            s["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered, "{} -> {to_label} message must deliver: {body}", from.label);
}

/// Opens a conversation to the provider at `peer_address` and files one
/// request in it, delivered. Returns `(conversation id, request record id)`.
pub async fn open_request_conv(
    node: &Node,
    peer_address: &str,
    description: &str,
    categories: &[&str],
) -> (String, String) {
    let opened = node.rpc_ok("conversation.open", json!({ "address": peer_address })).await;
    let conv_id = opened["conversation_id"].as_str().unwrap().to_string();
    let req = node
        .rpc_ok(
            "request.set",
            json!({
                "conversation": conv_id,
                "description": description,
                "categories": categories,
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let msg_id = req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node, &msg_id).await, "request delivered to provider");
    (conv_id, req["record_id"].as_str().unwrap().to_string())
}

/// The provider's own conversation id for the consumer at `peer_address`,
/// once the consumer's first message has arrived.
pub async fn provider_conv_for(y: &Node, peer_address: &str) -> String {
    let found = wait_until(Duration::from_secs(20), || async {
        let list = y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"]
            .as_array()
            .map(|cs| cs.iter().any(|c| c["peer_address"] == peer_address))
            .unwrap_or(false)
    })
    .await;
    assert!(found, "provider sees a conversation with {peer_address}");
    let list = y.rpc_ok("conversation.list", json!({})).await;
    list["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["peer_address"] == peer_address)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Files the request that arrived in `provider_conv` and returns its record
/// id -- what `quote.set` names.
pub async fn request_record_on_provider(provider: &Node, provider_conv: &str) -> String {
    let sync = provider.rpc_ok("transaction.sync", json!({ "conversation": provider_conv })).await;
    assert_eq!(sync["filed"], 1, "the provider files the one request: {sync}");
    let thread =
        provider.rpc_ok("transaction.thread", json!({ "conversation": provider_conv })).await;
    thread["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card_type"] == "request")
        .expect("request card in the provider's thread")["record_id"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn quote_terms(payee: &str, amount_minor: u64) -> Value {
    json!({
        "scope": "Clear garden waste and green bin collection",
        "currency": "EUR",
        "amount_minor": amount_minor,
        "tax_minor": 0,
        "fees_minor": 0,
        "payment_methods": ["cash"],
        "payee": payee,
        "payment_timing": "after-work",
        "location": { "where": "at-customer", "address": "1 Garden Lane" },
        "cancellation_terms": "24 hours notice required",
        "refund_terms": "Full refund if work not completed",
        "dispute_path": "Informal mediation",
    })
}

/// The provider quotes `request_record_id` against `slot_id`, delivered.
/// Returns the quote's record id.
pub async fn send_quote(
    provider: &Node,
    request_record_id: &str,
    listing_id: &str,
    slot_id: &str,
    terms: Value,
) -> String {
    let quote = provider
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": request_record_id,
                "listing_id": listing_id,
                "slot_id": slot_id,
                "expires_in_secs": 3600,
                "terms": terms,
            }),
        )
        .await;
    let msg = quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(provider, &msg).await, "the quote is delivered");
    quote["record_id"].as_str().unwrap().to_string()
}

/// The consumer syncs the quote in and accepts it; the accept is delivered
/// and the pair is one half.
pub async fn accept_quote(consumer: &Node, conversation: &str, quote_record_id: &str) {
    consumer.rpc_ok("transaction.sync", json!({ "conversation": conversation })).await;
    let accept =
        consumer.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(accept["pair"]["state"], "half");
    let msg = accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(consumer, &msg).await, "the accept is delivered");
}

pub async fn assert_no_directory(nodes: &[&Node]) {
    for node in nodes {
        let sources = node.rpc_ok("directory.sources", json!({})).await;
        let empty = sources["sources"].as_array().map(Vec::is_empty).unwrap_or(true);
        assert!(empty, "{} has no directory sources", node.label);
    }
}

/// The provider's countersigned receipt reaches the winner, whose booking
/// then reads `scheduled`.
pub async fn wait_and_verify_winner_scheduled(
    node_y: &Node,
    owner_y_did: &str,
    winner_node: &Node,
    winner_quote_record_id: &str,
    winner_conv_id: &str,
    winner_conv_on_y: &str,
) {
    let y_thread_winner =
        node_y.rpc_ok("transaction.thread", json!({ "conversation": winner_conv_on_y })).await;
    let y_cards_winner = y_thread_winner["cards"].as_array().unwrap();
    let y_prov_card = y_cards_winner
        .iter()
        .find(|c| c["card_type"] == "agreement-receipt" && c["issuer"] == owner_y_did)
        .expect("provider card in Y thread for winner");
    let prov_msg_id = y_prov_card["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_y, prov_msg_id).await, "countersigned receipt delivered to winner");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let winner_booking =
        winner_node.rpc_ok("booking.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(winner_booking["state"], "scheduled");
}

/// Payment request, both acknowledgements, both fulfilment signatures, and
/// both sides' bookings `completed`. `payee` is the quote's own payee.
pub async fn complete_winner_lifecycle(
    winner_node: &Node,
    node_y: &Node,
    winner_quote_record_id: &str,
    winner_conv_id: &str,
    winner_conv_on_y: &str,
    payee: &str,
) {
    let pay_req =
        node_y.rpc_ok("payment.request", json!({ "agreement": winner_quote_record_id })).await;
    let pay_req_msg = pay_req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &pay_req_msg).await, "payment request delivered");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let winner_thread =
        winner_node.rpc_ok("transaction.thread", json!({ "conversation": winner_conv_id })).await;
    let pay_req_card = winner_thread["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card_type"] == "payment-request")
        .expect("payment-request card in winner's thread");
    assert_eq!(pay_req_card["agreement_payee"], payee);

    let x_ack = winner_node
        .rpc_ok("payment.acknowledge", json!({ "agreement": winner_quote_record_id }))
        .await;
    let x_ack_msg = x_ack["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(winner_node, &x_ack_msg).await, "consumer's payment ack delivered");

    node_y.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_on_y })).await;
    let y_payment_after_consumer =
        node_y.rpc_ok("payment.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(y_payment_after_consumer["track"], "claimed");

    let y_ack =
        node_y.rpc_ok("payment.acknowledge", json!({ "agreement": winner_quote_record_id })).await;
    assert_ne!(y_ack["state"], "already-recorded");
    let y_payment_after_provider =
        node_y.rpc_ok("payment.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(y_payment_after_provider["track"], "acknowledged");

    let y_fulfil =
        node_y.rpc_ok("fulfilment.sign", json!({ "agreement": winner_quote_record_id })).await;
    let y_fulfil_msg = y_fulfil["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &y_fulfil_msg).await, "provider's fulfilment sign delivered");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let x_fulfil =
        winner_node.rpc_ok("fulfilment.sign", json!({ "agreement": winner_quote_record_id })).await;
    let x_fulfil_msg = x_fulfil["message_id"].as_str().unwrap().to_string();
    assert!(
        wait_delivered(winner_node, &x_fulfil_msg).await,
        "consumer's fulfilment sign delivered"
    );

    assert_booking_completed(
        node_y,
        winner_node,
        winner_quote_record_id,
        winner_conv_on_y,
        winner_conv_id,
    )
    .await;
}

/// Both sides sync until their booking for the agreement reads `completed`.
async fn assert_booking_completed(
    node_y: &Node,
    winner_node: &Node,
    quote_record_id: &str,
    conv_on_y: &str,
    conv_on_winner: &str,
) {
    let synced = wait_until(Duration::from_secs(30), || async {
        node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_on_y })).await;
        let b = node_y.rpc_ok("booking.get", json!({ "agreement": quote_record_id })).await;
        b["state"] == "completed"
    })
    .await;
    assert!(synced, "provider's booking reaches completed");

    let synced_consumer = wait_until(Duration::from_secs(30), || async {
        winner_node.rpc_ok("transaction.sync", json!({ "conversation": conv_on_winner })).await;
        let b = winner_node.rpc_ok("booking.get", json!({ "agreement": quote_record_id })).await;
        b["state"] == "completed"
    })
    .await;
    assert!(synced_consumer, "winner's own progress reaches completed");
}

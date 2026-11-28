use serde_json::{Value, json};
use syneroym_roym_core::record_forward::{RECORD_FORWARD_CONTENT_TYPE, RecordForwardBody};

use super::{deal_fixtures::peer_signed_quote, fixtures::*, helpers::*};

async fn forward_record(
    h: &Harness,
    conv: &str,
    record_type: &str,
    envelope: &str,
) -> (Value, Value) {
    both_rpc(
        h,
        "conversation.forward-record",
        json!({
            "conversation": conv,
            "record_type": record_type,
            "envelope": envelope,
        }),
    )
    .await
}

async fn history(h: &Harness, conv: &str) -> (Value, Value) {
    let (mut w, mut n) = both_rpc(h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    normalize_message_ids(&mut w);
    normalize_message_ids(&mut n);
    (w, n)
}

fn forward_body_for(view: &Value, envelope: &str) -> RecordForwardBody {
    let rows = view["result"]["messages"].as_array().expect("history rows");
    for row in rows {
        let text = row["body"].as_str().unwrap_or_default();
        let Ok(body) = RecordForwardBody::parse(text.as_bytes()) else { continue };
        if body.envelope == envelope {
            return body;
        }
    }
    panic!("no forwarded record row for this envelope");
}

#[tokio::test]
async fn scenario_289_a_forwarded_record_travels_as_itself_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    enrol_signing(&h, "profile").await;
    let conv = open_conv(&h, "did:key:zPeer289").await;

    let (_, gw, gn) =
        set_and_get(&h, full_listing_params("forward-289", "Forwarded listing")).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);
    let listing_env = gw["result"]["envelope"].as_str().unwrap().to_string();

    let (pw, pn) = both_rpc(
        &h,
        "profile.set",
        json!({ "display_name": "Alice", "conversation_address": "svc:alice", "lang": "en" }),
    )
    .await;
    assert_eq!(pw["result"]["envelope"], pn["result"]["envelope"]);
    let profile_env = pw["result"]["envelope"].as_str().unwrap().to_string();

    let (_, quote_env, _) =
        peer_signed_quote(&conv, 1, "rec_289", "did:key:zPeer289", None, 1_000_000);

    for (record_type, envelope) in [
        ("listing", listing_env.as_str()),
        ("profile", profile_env.as_str()),
        ("quote", quote_env.as_str()),
    ] {
        let (fw, fnat) = forward_record(&h, &conv, record_type, envelope).await;
        assert!(fw["result"]["message_id"].is_string(), "{record_type} wasm: {fw}");
        assert!(fnat["result"]["message_id"].is_string(), "{record_type} native: {fnat}");

        let (hw, hn) = history(&h, &conv).await;
        assert_eq!(hw, hn, "{record_type} history");
        let body = forward_body_for(&hw, envelope);
        assert_eq!(body.record_type, record_type);
        assert_eq!(body.envelope, envelope);
    }

    // The receiver's own services check each record, and a quote with no
    // disclosures verifies.
    let (lw, ln) = both_rpc(&h, "listing.verify", json!({ "envelope": listing_env })).await;
    assert_eq!(lw, ln);
    assert_eq!(lw["result"]["verified"], true, "{lw}");

    let (qw, qn) =
        both_rpc(&h, "quote.verify", json!({ "envelope": quote_env, "disclosures": [] })).await;
    assert_eq!(qw, qn);
    assert_eq!(qw["result"]["verified"], true, "{qw}");
    assert_eq!(qw["result"]["payload"]["terms"]["_sd"].as_array().unwrap().len(), 5);

    let (prw, prn) =
        both_rpc(&h, "profile.verify-envelope", json!({ "envelope": profile_env })).await;
    assert_eq!(prw, prn);
    assert_eq!(prw["result"]["verified"], true, "{prw}");
    assert_eq!(prw["result"]["payload"]["display_name"], "Alice");
}

#[tokio::test]
async fn scenario_290_forward_record_refuses_a_type_and_a_bad_envelope_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer290").await;

    for (record_type, envelope, code) in [
        ("enquiry", "{}", "record-type-not-supported"),
        ("card", "{}", "record-type-not-supported"),
        ("listing", "not json", "record-not-readable"),
        ("quote", "[1, 2]", "record-not-readable"),
    ] {
        let (w, n) = forward_record(&h, &conv, record_type, envelope).await;
        assert_eq!(w, n, "{record_type}");
        assert_eq!(w["error"]["code"], -32602, "{record_type}: {w}");
        assert_eq!(w["error"]["data"]["code"], code, "{record_type}: {w}");
    }

    let too_large = format!("{{\"payload\":\"{}\"}}", "x".repeat(160 * 1024));
    let (w, n) = forward_record(&h, &conv, "listing", &too_large).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602, "{w}");
    assert_eq!(w["error"]["data"]["code"], "record-too-large");

    let (hw, hn) = history(&h, &conv).await;
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_291_send_with_the_record_forward_type_is_reserved_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer291").await;

    let body = json!({
        "record_forward_version": 1,
        "record_type": "profile",
        "envelope": "{\"record_type\":\"profile\"}",
    })
    .to_string();
    let (w, n) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv, "body": body, "content_type": RECORD_FORWARD_CONTENT_TYPE }),
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602, "{w}");
    assert_eq!(w["error"]["message"], "this content type is reserved");
}

#[tokio::test]
async fn scenario_292_a_record_forward_body_is_not_indexed_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer292").await;

    let envelope = json!({ "record_type": "profile", "display_name": "Zebra Keeper" }).to_string();
    let (w, n) = forward_record(&h, &conv, "profile", &envelope).await;
    assert!(w["result"]["message_id"].is_string(), "{w}");
    assert!(n["result"]["message_id"].is_string(), "{n}");

    for query in ["Zebra", "record_forward_version", "record_type"] {
        let (w, n) = both_rpc(&h, "conversation.search", json!({ "query": query })).await;
        assert_eq!(w, n, "{query}");
        assert!(
            w["result"]["matches"].as_array().unwrap().is_empty(),
            "{query} found a record forward: {w}"
        );
    }

    let (w, n) = both_rpc(&h, "conversation.search", json!({ "query": "profile" })).await;
    assert_eq!(w, n);
}

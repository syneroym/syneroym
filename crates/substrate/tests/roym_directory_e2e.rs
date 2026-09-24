#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! The Roym product's Directory service -- the search half -- end to end
//! across three genuinely independent `syneroym-substrate` instances, each
//! running the full Roym SynApp (the `wasm32-wasip2` build) under its own
//! owner identity, over real transports and one shared registry.
//!
//! Three nodes:
//!   * **Z** runs the SynOrg (the directory a provider publishes to and a
//!     consumer queries).
//!   * **Y** is the provider: it signs a listing and publishes it to Z.
//!   * **X** is the consumer: it adds Z as a source, runs the client fan-out
//!     loop (`start-run` -> `query-source` -> `merge`), verifies every returned
//!     envelope on its own node, and engages the provider from the search
//!     result's own conversation address.
//!
//! It proves the non-visual half of the directory acceptance test: a
//! directory is a query target and never a required hub (the
//! find-and-engage path runs with no directory in
//! it, both before any publication exists and again at the end after one
//! has); results carry source and freshness, and the freshness a person
//! sees is computed on their own clock; missing evidence renders as
//! `unknown`, never as a positive default; the directory verifies nothing
//! on the consumer's behalf; a publication past the SynOrg's limit is
//! refused visibly to the provider; a stranger dialling in from a
//! self-minted identity reaches `directory.search` and is refused
//! `member.list`, and -- because a generated key is still a verified
//! connection -- is admitted to `VerifiedOnly` `directory.publish` (the
//! truly key-less anonymous arm lives in the parity suite); a stale or
//! absent certificate on
//! the provider's own node is indistinguishable, at the provider, from
//! "this directory does not want you"; two directories disagreeing about a
//! version surface the disagreement rather than resolve it silently; and
//! `directory.unpublish` removes a listing from future search without
//! touching a copy a consumer already holds.
//!
//! Each node's substrate comes from `common::SubstrateNode` (with the Roym
//! config layered on through `.configure`); the domain `Node` here keeps the
//! deploy / login machinery on top. Only the first node hosts the community
//! registry -- three registry servers plus three iroh relays in one process
//! starve the first node's own registry out of its registration window on a
//! loaded machine, and the next heartbeat is an hour away -- so a node that
//! shares a registry drops its own `community_registry` role.
//!
//! No step here restarts a substrate, so there is no redeploy-after-restart
//! path to get wrong; the certificate-dependency sub-step uses a fresh
//! fourth node instead.
//!
//! Skips when the Roym wasm artifacts or the UI bundle are absent
//! (`mise run build:roym` / `mise run build:roym-ui`).

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;

mod common;

use common::roym::{RoymNode as Node, roym_artifacts_present, wait_until};

const DIRECTORY_INTERFACE: &str = "syneroym-roym:directory/api@0.1.0";

/// One JSON-RPC `invoke` frame delivered to `target_did`'s directory
/// interface over a real QUIC stream, from a freshly generated identity
/// with no delegation. The connection key is still verified by the
/// handshake, so the router reads `CallerOrigin::Verified(<generated
/// did>)` -- a stranger, not an anonymous caller. A truly key-less
/// `Anonymous` wire caller cannot be expressed over iroh; the parity
/// suite covers that arm with `AuthLevel::System`.
/// Returns the inner `envelope::Response`-shaped value the guest produced.
async fn stranger_wire_invoke(
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

fn hits(result: &Value) -> Vec<Value> {
    result["hits"].as_array().cloned().unwrap_or_default()
}

/// Drive the consumer client loop the way `roymctl roym directory find` and
/// the Hub do: `start-run`, one `query-source` per source (respecting
/// `max_concurrency`), then `merge`. Returns `(run_id, merge_result)`.
async fn run_client_loop(node: &Node, query: Value) -> (String, Value) {
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

async fn listing_envelope(node: &Node, listing_id: &str) -> String {
    let row = node.rpc_ok("listing.get", json!({ "listing_id": listing_id })).await;
    row["envelope"].as_str().unwrap().to_string()
}

fn listing_params(title: &str, summary: &str) -> Value {
    json!({
        "title": title,
        "summary": summary,
        "categories": ["cycling"],
        "payment": {
            "currency": "EUR", "model": "per-hour", "amount_minor": 4000,
            "tax_included": true, "payee": "provider"
        }
    })
}

/// `conversation.open` on a raw address with no contact entry, one message
/// sent and driven to `delivered` with retries.
async fn deliver_one_message(from: &Node, to_label: &str, address: &str, body: &str) {
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

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear roym directory search across three substrates")]
async fn roym_directory_search_half_across_three_substrates() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let dir_z = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_z = Identity::generate().unwrap();

    // --- Step 1: three nodes boot, deploy Roym, enrol signing. -----------
    let mut node_x = Node::boot_default(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot_default(
        "node-y",
        dir_y.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
    )
    .await;
    node_y.full_bring_up().await;

    let mut node_z = Node::boot_default(
        "node-z",
        dir_z.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_z.to_bytes()),
    )
    .await;
    node_z.full_bring_up().await;

    let z_dir_did = node_z.dids["directory"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    // --- Step 2: Z creates the SynOrg; X reads it back over a real
    //     transport with directory.probe-info. ---------------------------
    let settings = json!({
        "name": "South Bengaluru Trades",
        "rules": "Be honest. Show up. Fix what you break.",
        "area": [],
        "categories": ["cycling", "plumbing"],
        "support_contact": "help@example.org",
        "dispute_path": "Email support; unresolved after 14 days goes to arbitration.",
        "retention_secs": 30 * 24 * 3600,
        "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
    });
    node_z.rpc_ok("directory.set-settings", settings).await;

    let z_info = node_x.rpc_ok("directory.probe-info", json!({ "did": z_dir_did })).await;
    assert_eq!(
        z_info["name"], "South Bengaluru Trades",
        "X reads Z's SynOrg over the wire: {z_info}"
    );
    assert_eq!(z_info["retention_secs"], 30 * 24 * 3600);
    assert!(z_info.get("members").is_none(), "directory.info carries no roster: {z_info}");
    assert_eq!(z_info["member_count"], 0);

    // --- Step 3: Y creates a profile and a signed listing, in no
    //     directory. -----------------------------------------------------
    node_y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Yara", "conversation_address": y_conv_did }),
        )
        .await;
    let y_listing = node_y
        .rpc_ok("listing.set", listing_params("Bike repair", "Same-day, at your door."))
        .await;
    let y_listing_id = y_listing["listing_id"].as_str().unwrap().to_string();
    let y_envelope = listing_envelope(&node_y, &y_listing_id).await;

    // --- Step 4: X reaches Y by direct link, no directory in the path.
    //     Runs BEFORE any publication exists anywhere. ------------------
    let x_verify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(x_verify["verified"], true, "X verifies Y's listing with no directory: {x_verify}");
    assert_eq!(x_verify["conversation_address"], y_conv_did);
    deliver_one_message(&node_x, "node-y", &y_conv_did, "hello via direct link").await;

    // --- Step 5: Z adds Y to the roster. No credential is issued
    //     (a later cross-installation-trust concern). ------------------
    let member = node_z
        .rpc_ok("member.add", json!({ "did": owner_y_did, "note": "verified provider" }))
        .await;
    assert_eq!(member["did"], owner_y_did);
    let z_info_after = node_x.rpc_ok("directory.probe-info", json!({ "did": z_dir_did })).await;
    assert_eq!(z_info_after["member_count"], 1, "info reports the roster size, not the roster");

    // --- Step 6: Y publishes to Z through directory.publish-to-source,
    //     over the wire, verified. --------------------------------------
    node_y
        .rpc_ok(
            "directory.add-source",
            json!({ "did": z_dir_did, "label": "South Bengaluru Trades" }),
        )
        .await;
    let published = node_y
        .rpc_ok(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_id }),
        )
        .await;
    assert_eq!(published["listing_id"], y_listing_id, "Y publishes to Z: {published}");
    let z_pubs = node_z.rpc_ok("directory.publications", json!({})).await;
    assert_eq!(
        z_pubs["publications"].as_array().map(Vec::len),
        Some(1),
        "Z holds exactly Y's one publication: {z_pubs}"
    );

    // --- Step 7: X adds Z as a source and runs the client loop. --------
    let x_add = node_x.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    assert!(x_add["source"]["last_error"].is_null(), "X's probe of Z succeeded: {x_add}");

    let (run_id, merged) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    let merged_hits = hits(&merged);
    assert_eq!(merged_hits.len(), 1, "one hit from Z: {merged}");
    let hit = merged_hits[0].clone();
    assert_eq!(hit["listing_id"], y_listing_id);
    assert_eq!(hit["issuer"], owner_y_did);
    assert_eq!(hit["revocation_status"], "unknown", "revocation renders unknown, never positive");
    assert_eq!(hit["credential"], "unknown", "membership renders unknown, never positive");
    assert!(hit["verified"].as_bool().unwrap_or(false), "X's own verdict, not Z's: {hit}");
    assert!(
        hit["age_secs"].as_u64().unwrap() < 3600,
        "age is computed on X's own clock, not Z's received_at: {hit}"
    );
    let hit_sources = hit["sources"].as_array().cloned().unwrap_or_default();
    assert_eq!(hit_sources.len(), 1);
    assert_eq!(hit_sources[0]["directory"], z_dir_did, "the source is Z: {hit}");
    assert!(merged["refused"].as_array().is_none_or(|r| r.is_empty()), "nothing refused: {merged}");

    let record_id = hit["record_id"].as_str().unwrap().to_string();
    let run_env = node_x
        .rpc_ok("directory.run-envelope", json!({ "run_id": run_id, "record_id": record_id }))
        .await;
    assert_eq!(
        run_env["envelope"].as_str(),
        Some(y_envelope.as_str()),
        "run-envelope returns bytes byte-identical to what Y signed"
    );

    // --- Step 8: X starts a conversation from the search result's own
    //     conversation_address, with no prior contact entry. -----------
    let result_address = hit["conversation_address"].as_str().unwrap().to_string();
    assert_eq!(result_address, y_conv_did);
    deliver_one_message(&node_x, "node-y", &result_address, "found you through the directory")
        .await;

    // --- Step 9: a stranger dialling in over the wire from a self-minted
    //     identity reaches directory.search, is refused member.list, and
    //     -- because a generated key is still a *verified* connection --
    //     is admitted to the VerifiedOnly directory.publish. -------------
    let stranger_search = stranger_wire_invoke(
        &shared_registry,
        &z_dir_did,
        "directory.search",
        json!({ "categories": ["cycling"] }),
    )
    .await;
    assert!(
        stranger_search["result"]["hits"].as_array().is_some(),
        "a stranger may read a directory: {stranger_search}"
    );
    let stranger_members =
        stranger_wire_invoke(&shared_registry, &z_dir_did, "member.list", json!({})).await;
    assert_eq!(
        stranger_members["error"]["code"], -32013,
        "member.list is never reachable off the node: {stranger_members}"
    );
    // `member.list` being refused proves only that the method is
    // unlisted, not anything about the caller's identity. `search`
    // (Open) and `publish` (VerifiedOnly) together do: this stranger is
    // admitted to both, which is only possible for a `Verified` caller.
    // A key-less, truly `Anonymous` wire caller -- refused `publish`
    // with -32013 -- is not expressible over iroh (every connection is
    // keyed); that arm is covered by parity scenario 80.
    let stranger_publish = stranger_wire_invoke(
        &shared_registry,
        &z_dir_did,
        "directory.publish",
        json!({ "envelope": y_envelope }),
    )
    .await;
    assert_ne!(
        stranger_publish["error"]["code"].as_i64(),
        Some(-32013),
        "a self-minted identity is a verified connection, so VerifiedOnly admits it: \
         {stranger_publish}"
    );

    // --- Step 10: Y publishes past the limit and is refused with a
    //     retry_after_secs, visible to Y. -------------------------------
    node_z
        .rpc_ok("directory.set-limits", json!({ "window_secs": 24 * 3600, "max_per_window": 1 }))
        .await;
    let y_listing_2 = node_y
        .rpc_ok("listing.set", listing_params("Wheel truing", "Bring the wheel, wait ten minutes."))
        .await;
    let y_listing_2_id = y_listing_2["listing_id"].as_str().unwrap().to_string();
    let refused = node_y
        .rpc_err(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_2_id }),
        )
        .await;
    assert_eq!(refused["code"], -32602, "over the limit is refused visibly: {refused}");
    assert!(
        refused["data"]["retry_after_secs"].as_u64().unwrap_or(0) > 0,
        "the refusal carries a retry_after_secs Y can act on: {refused}"
    );
    node_z
        .rpc_ok("directory.set-limits", json!({ "window_secs": 24 * 3600, "max_per_window": 20 }))
        .await;

    // --- Step 11: Z unpublishes Y's listing; X's next search no longer
    //     returns it, and X's already-held copy is untouched. ----------
    node_z.rpc_ok("directory.unpublish", json!({ "listing_id": y_listing_id })).await;
    let (_, after_unpublish) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    assert!(
        hits(&after_unpublish).is_empty(),
        "the unpublished listing is gone from Z's search: {after_unpublish}"
    );
    let held_still_valid = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(
        held_still_valid["verified"], true,
        "unpublish never touches a copy the consumer already holds"
    );

    // --- Step 12: the no-directory regression. X removes Z as a source
    //     and step 4's whole path is re-run and passes -- at the end,
    //     after a directory has existed. -----------------------------
    node_x.rpc_ok("directory.remove-source", json!({ "did": z_dir_did })).await;
    let (_, empty_run) = run_client_loop(&node_x, json!({})).await;
    assert!(hits(&empty_run).is_empty(), "no sources -> zero hits, no error: {empty_run}");
    let reverify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(reverify["verified"], true, "the direct-link path still works with no directory");
    deliver_one_message(&node_x, "node-y", &y_conv_did, "still reachable without a directory")
        .await;

    // --- Step 13: two directories disagree about a version. Y's own node
    //     becomes a second SynOrg holding the older version of Y's
    //     listing; a search over both merges to one hit with
    //     versions_differ. ---------------------------------------------
    node_y
        .rpc_ok(
            "directory.set-settings",
            json!({
                "name": "Yara's picks",
                "rules": "personal recommendations",
                "area": [],
                "categories": ["cycling"],
                "support_contact": "yara@example.org",
                "dispute_path": "n/a",
                "retention_secs": 30 * 24 * 3600,
                "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
            }),
        )
        .await;
    // The older version to Y's own directory (local publish, Internal
    // caller); a fresh, current version to Z.
    node_y.rpc_ok("directory.publish", json!({ "envelope": y_envelope })).await;
    node_y
        .rpc_ok(
            "listing.set",
            json!({
                "listing_id": y_listing_id,
                "title": "Bike repair",
                "summary": "Same-day, at your door. Now with loan bikes.",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "per-hour", "amount_minor": 4500,
                    "tax_included": true, "payee": "provider"
                }
            }),
        )
        .await;
    node_y
        .rpc_ok(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_id }),
        )
        .await;

    let y_dir_did = node_y.dids["directory"].clone();
    node_x.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    node_x.rpc_ok("directory.add-source", json!({ "did": y_dir_did })).await;
    let (_, two_dir) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    let two_hits = hits(&two_dir);
    assert_eq!(two_hits.len(), 1, "the two directories merge to one hit: {two_dir}");
    assert_eq!(two_hits[0]["versions_differ"], true, "the disagreement is surfaced: {two_dir}");
    assert_eq!(
        two_hits[0]["sources"].as_array().map(Vec::len),
        Some(2),
        "both directories are named in sources[]: {two_dir}"
    );

    // --- Step 13b: the loop at MAX_SOURCES, most sources unreachable. --
    for i in 0..6 {
        let bogus = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
        node_x
            .rpc_ok("directory.add-source", json!({ "did": bogus, "label": format!("bogus-{i}") }))
            .await;
    }
    let (_, ceiling_run) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    assert_eq!(
        hits(&ceiling_run).len(),
        1,
        "the two live directories still contribute: {ceiling_run}"
    );
    let x_sources = node_x.rpc_ok("directory.sources", json!({})).await;
    let source_rows = x_sources["sources"].as_array().cloned().unwrap_or_default();
    assert_eq!(source_rows.len(), 8, "X holds MAX_SOURCES sources: {x_sources}");
    let errored = source_rows.iter().filter(|s| !s["last_error"].is_null()).count();
    assert!(errored >= 6, "the unreachable sources each carry an error: {x_sources}");
    for s in &source_rows {
        assert_ne!(
            s["last_error"]["kind"], "not-started",
            "no source is blamed for this node's own admission limit: {s}"
        );
    }

    // --- Step 14: export / import of Z's directory bundles, then
    //     reindex, then an identical search. ---------------------------
    let z_before = node_z.rpc_ok("directory.search", json!({ "categories": ["cycling"] })).await;
    let z_bundle = node_z.rpc_ok("directory.export", json!({})).await;
    node_z.rpc_ok("directory.import", json!({ "bundle": z_bundle })).await;
    node_z.rpc_ok("directory.reindex", json!({})).await;
    let z_after = node_z.rpc_ok("directory.search", json!({ "categories": ["cycling"] })).await;
    assert_eq!(
        hits(&z_before).len(),
        hits(&z_after).len(),
        "search returns the same after export/import/reindex: {z_after}"
    );

    // --- Step 7b: the certificate dependency, over a real transport.
    //     A fresh provider whose directory service holds no instance
    //     certificate at all: publish-to-source fails with -32013,
    //     indistinguishable from "not yours to call". X stays up so the
    //     shared registry keeps resolving Z's directory for the newcomer.
    let dir_w = tempfile::tempdir().unwrap();
    let owner_w = Identity::generate().unwrap();
    let mut node_w = Node::boot_default(
        "node-w",
        dir_w.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_w.to_bytes()),
    )
    .await;
    node_w.cert_overrides.skip_instance_cert = vec!["directory".to_string()];
    node_w.full_bring_up().await;
    node_w
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Wes", "conversation_address": node_w.dids["conversation"] }),
        )
        .await;
    let w_listing =
        node_w.rpc_ok("listing.set", listing_params("Frame welding", "Steel and titanium.")).await;
    let w_listing_id = w_listing["listing_id"].as_str().unwrap().to_string();
    node_w.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    let w_refused = node_w
        .rpc_err(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": w_listing_id }),
        )
        .await;
    let w_text = w_refused.to_string();
    assert!(
        w_text.contains("32013"),
        "a stale/absent instance cert reads as 'not yours to call': {w_refused}"
    );

    node_w.teardown().await;
    node_z.teardown().await;
    node_y.teardown().await;
    node_x.teardown().await;
}

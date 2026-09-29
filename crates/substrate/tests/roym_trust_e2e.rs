#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! Cross-installation trust, end to end across three genuinely independent
//! `syneroym-substrate` instances, each running the full Roym SynApp (the
//! `wasm32-wasip2` build) under its own owner, over real transports and one
//! shared registry.
//!
//! Three people, three installations:
//!   * **Z** runs a SynOrg -- a directory whose owner issues signed membership
//!     credentials, and hosts the registry.
//!   * **Y** is a provider: a member of Z's SynOrg.
//!   * **X** is a consumer, who knows nothing but Z's directory address.
//!
//! The one test walks a consumer from a stranger's directory to a finished,
//! signed booking with a provider on a third installation, then watches the
//! SynOrg withdraw that provider: the verdict on the consumer's node is the
//! consumer's own, computed there from signed evidence, and a withdrawal
//! reaches a copy the consumer already holds only when they check again. It
//! ends by carrying the consumer's data, trust records included, onto a
//! clean installation.
//!
//! Skips when the Roym wasm artifacts or the UI bundle are absent
//! (`mise run build:roym` / `mise run build:roym-ui`).

use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rustls::crypto::ring;
use serde_json::{Map, Value, json};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;

mod common;

use common::{
    roym::{RoymNode as Node, fast_conversation_role, roym_artifacts_present},
    roym_flow::{
        accept_quote, complete_winner_lifecycle, hits, open_request_conv, provider_conv_for,
        quote_terms, request_record_on_provider, run_client_loop, send_quote, stranger_wire_invoke,
        wait_and_verify_winner_scheduled,
    },
};

const CREDENTIAL_DAYS: u64 = 30;
const PAYEE: &str = "Y Cycles";
/// A quoted slot far enough ahead that no test run reaches it.
const SLOT_START_SECS: u64 = 1_800_000_000;

/// The three installations and what the steps below pass between them.
struct Trio {
    z: Node,
    y: Node,
    x: Node,
    z_owner_did: String,
    y_owner_did: String,
    x_owner: Identity,
}

async fn boot_node(
    label: &'static str,
    dir: &Path,
    registry: Option<String>,
    owner: &Identity,
) -> Node {
    let mut node = Node::boot(
        label,
        dir.to_path_buf(),
        registry,
        Identity::from_bytes(&owner.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node.full_bring_up().await;
    node
}

/// Z hosts the registry (three registry servers in one process starve each
/// other's registration window); Y and X resolve through it.
async fn boot_trio(dirs: &[&Path; 3]) -> Trio {
    let (owner_z, owner_y, owner_x) = (
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
    );
    let z = boot_node("node-z", dirs[0], None, &owner_z).await;
    let registry = Some(z.registry_url.clone());
    let y = boot_node("node-y", dirs[1], registry.clone(), &owner_y).await;
    let x = boot_node("node-x", dirs[2], registry, &owner_x).await;
    Trio {
        z,
        y,
        x,
        z_owner_did: substrate::derive_did_key(&owner_z.public_key()),
        y_owner_did: substrate::derive_did_key(&owner_y.public_key()),
        x_owner: owner_x,
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// Steps 1-2: Z declares its SynOrg; Y has a profile, a `cycling` listing
/// and one bookable slot. Returns `(listing_id, slot_id)`.
async fn set_up_synorg_and_provider(t: &Trio) -> (String, String) {
    t.z.rpc_ok(
        "directory.set-settings",
        json!({
            "name": "Cycling Guild",
            "rules": "Be honest. Show up. Fix what you break.",
            "area": [],
            "categories": ["cycling"],
            "support_contact": "help@example.org",
            "dispute_path": "Email support; unresolved after 14 days goes to arbitration.",
            "retention_secs": 30 * 24 * 3600,
            "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
        }),
    )
    .await;

    t.y.rpc_ok(
        "profile.set",
        json!({ "display_name": "Yara", "conversation_address": t.y.dids["conversation"] }),
    )
    .await;
    let listing =
        t.y.rpc_ok(
            "listing.set",
            json!({
                "title": "Bike repair",
                "summary": "Same-day, at your door.",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 6000,
                    "tax_included": true, "payee": PAYEE
                }
            }),
        )
        .await;
    let listing_id = listing["listing_id"].as_str().unwrap().to_string();
    let slots = json!([{ "start_secs": SLOT_START_SECS, "end_secs": SLOT_START_SECS + 3600, "capacity": 1 }]);
    let avail =
        t.y.rpc_ok("availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    (listing_id, avail["slot_ids"][0].as_str().unwrap().to_string())
}

/// Steps 3-5: a non-member's publish is refused with the reason; after Z
/// issues Y a credential, the same publish is accepted. Returns the
/// credential's record id and the expiry Z set.
async fn admit_the_provider(t: &Trio, listing_id: &str) -> (String, u64) {
    let z_dir = &t.z.dids["directory"];
    t.y.rpc_ok("directory.add-source", json!({ "did": z_dir, "label": "Cycling Guild" })).await;
    let publish = json!({ "source": z_dir, "listing_id": listing_id });

    let refused = t.y.rpc_err("directory.publish-to-source", publish.clone()).await;
    assert_eq!(refused["data"]["admission"], "not-admitted", "a non-member is refused: {refused}");
    assert_eq!(refused["data"]["membership"]["state"], "none", "and told why: {refused}");

    let expires_at_secs = now_secs() + CREDENTIAL_DAYS * 24 * 3600;
    let issued =
        t.z.rpc_ok(
            "credential.issue",
            json!({
                "member_did": t.y_owner_did,
                "categories": ["cycling"],
                "expires_at_secs": expires_at_secs,
            }),
        )
        .await;
    let published = t.y.rpc_ok("directory.publish-to-source", publish).await;
    assert_eq!(published["listing_id"], listing_id, "a member's publish is accepted: {published}");
    (issued["record_id"].as_str().unwrap().to_string(), expires_at_secs)
}

/// Step 6: X adds Z knowing nothing but Z's directory address, and every
/// service X will later dial (Y's catalog and conversation, Z's directory)
/// has a registry record it can resolve. Returns Z's issuer as X pinned it.
async fn consumer_adds_the_directory(t: &Trio) -> String {
    let z_dir = &t.z.dids["directory"];
    let added = t.x.rpc_ok("directory.add-source", json!({ "did": z_dir })).await;
    assert!(added["source"]["last_error"].is_null(), "X's probe of Z succeeded: {added}");
    let issuer = added["source"]["issuer_did"].as_str().unwrap().to_string();
    assert_eq!(issuer, t.z_owner_did, "X pinned the issuer Z's info named");

    let registry = RegistryClient::new(false, Some(t.z.registry_url.clone()));
    for (who, did) in [
        ("Y's catalog", &t.y.dids["catalog"]),
        ("Y's conversation", &t.y.dids["conversation"]),
        ("Z's directory", z_dir),
    ] {
        assert!(registry.lookup(did, true).await.is_ok(), "the registry resolves {who}");
    }
    issuer
}

/// Step 6b: a stranger with no token reads Y's evidence from Z, cannot call
/// a local-only verb, and cannot resolve a service that publishes no record.
async fn a_stranger_resolves_only_what_is_published(t: &Trio) {
    let registry = t.z.registry_url.clone();
    let standing = stranger_wire_invoke(
        &registry,
        &t.z.dids["directory"],
        "directory.standing",
        json!({ "member_did": t.y_owner_did }),
    )
    .await;
    assert_eq!(
        standing["result"]["evidence"]["credentials"].as_array().map(Vec::len),
        Some(1),
        "a stranger resolves Z's directory and reads Y's evidence: {standing}"
    );
    let listing =
        stranger_wire_invoke(&registry, &t.z.dids["directory"], "credential.list", json!({})).await;
    assert_eq!(listing["error"]["code"], -32013, "credential.list is local-only: {listing}");

    let profile_did = &t.y.dids["profile"];
    let lookup = RegistryClient::new(false, Some(registry.clone())).lookup(profile_did, true).await;
    assert!(lookup.is_err(), "a private service publishes no registry record");
    let mut client = SyneroymClient::new_with_identity(
        profile_did.clone(),
        registry,
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    assert!(client.connect().await.is_err(), "and so cannot be dialled either");
}

/// Step 7: X searches; the one hit's membership verdict was computed on X's
/// node from Z's signed evidence.
async fn consumer_searches(t: &Trio, listing_id: &str, expires_at_secs: u64) -> Value {
    let (_run, merged) = run_client_loop(&t.x, json!({ "categories": ["cycling"] })).await;
    let found = hits(&merged);
    assert_eq!(found.len(), 1, "one hit from Z: {merged}");
    let hit = found[0].clone();
    assert_eq!(hit["listing_id"], listing_id);
    let membership = &hit["sources"][0]["membership"];
    assert_eq!(membership["state"], "valid", "{hit}");
    assert_eq!(membership["issuer"], t.z_owner_did, "{hit}");
    assert_eq!(membership["scope"]["categories"], json!(["cycling"]), "{hit}");
    assert_eq!(membership["expires_at_secs"], expires_at_secs, "{hit}");
    hit
}

/// Step 8: X hires Y from the hit's own conversation address, through to
/// both sides' signed fulfilment. Nothing about Y but what the hit carried.
/// Returns the agreement's quote record id.
async fn hire_the_provider(t: &Trio, hit: &Value, listing_id: &str, slot_id: &str) -> String {
    let address = hit["conversation_address"].as_str().unwrap();
    t.x.rpc_ok(
        "profile.set",
        json!({ "display_name": "Xavi", "conversation_address": t.x.dids["conversation"] }),
    )
    .await;
    let (x_conv, _request) = open_request_conv(&t.x, address, "Fix my bike", &["cycling"]).await;
    let y_conv = provider_conv_for(&t.y, &t.x.dids["conversation"]).await;
    let request = request_record_on_provider(&t.y, &y_conv).await;
    let quote = send_quote(&t.y, &request, listing_id, slot_id, quote_terms(PAYEE, 6000)).await;
    accept_quote(&t.x, &x_conv, &quote).await;

    t.y.rpc_ok("transaction.sync", json!({ "conversation": y_conv })).await;
    wait_and_verify_winner_scheduled(&t.y, &t.y_owner_did, &t.x, &quote, &x_conv, &y_conv).await;
    complete_winner_lifecycle(&t.x, &t.y, &quote, &x_conv, &y_conv, PAYEE).await;
    quote
}

/// The verdict X's node holds for Y from Z's directory, as `directory.
/// memberships` reports it: `(verdict state, as_of_secs)`.
async fn held_verdict(t: &Trio) -> (String, u64) {
    let held = t.x.rpc_ok("directory.memberships", json!({ "member_did": t.y_owner_did })).await;
    let row = &held["memberships"][0];
    (row["verdict"]["state"].as_str().unwrap().to_string(), row["as_of_secs"].as_u64().unwrap())
}

/// Steps 9-11: Z suspends Y. The result vanishes from X's next search; X's
/// held copy still says `valid` and keeps its old date -- the product does
/// not claim an instant removal -- until X checks again.
async fn suspension_reaches_the_consumer_only_on_check(t: &Trio) {
    let (state_before, as_of_before) = held_verdict(t).await;
    assert_eq!(state_before, "valid");

    t.z.rpc_ok(
        "member.suspend",
        json!({ "member_did": t.y_owner_did, "rule": "r1", "reason": "test" }),
    )
    .await;

    let (_run, merged) = run_client_loop(&t.x, json!({ "categories": ["cycling"] })).await;
    assert!(hits(&merged).is_empty(), "the suspended member's listing is gone: {merged}");

    let (state_stale, as_of_stale) = held_verdict(t).await;
    assert_eq!(state_stale, "valid", "the copy X already holds is not rewritten by a search");
    assert_eq!(as_of_stale, as_of_before, "and keeps its own date");

    let checked =
        t.x.rpc_ok(
            "directory.check-standing",
            json!({ "source": t.z.dids["directory"], "member_did": t.y_owner_did }),
        )
        .await;
    assert_eq!(checked["verdict"]["state"], "suspended", "{checked}");
    assert_eq!(checked["refreshed"], true, "{checked}");
}

/// Step 12: Z revokes the credential; X's next check says so.
async fn revocation_reaches_the_consumer_on_check(t: &Trio, credential_id: &str) {
    t.z.rpc_ok(
        "revocation.issue",
        json!({ "credential_record_id": credential_id, "reason": "left the guild" }),
    )
    .await;
    let checked =
        t.x.rpc_ok(
            "directory.check-standing",
            json!({ "source": t.z.dids["directory"], "member_did": t.y_owner_did }),
        )
        .await;
    assert_eq!(checked["verdict"]["state"], "revoked", "{checked}");
}

const DATA_SERVICES: &[&str] = &["profile", "catalog", "conversation", "transaction", "directory"];

/// Step 13: X's data goes onto a clean installation under X's own owner
/// identity. The held membership for Y still evaluates `revoked` there --
/// re-evaluated from the stored evidence, not a stored verdict -- and the
/// finished agreement is intact.
async fn consumer_leaves_with_their_data(t: &Trio, dir: &Path, quote_record_id: &str) {
    let mut bundles = Map::new();
    for svc in DATA_SERVICES {
        bundles.insert((*svc).to_string(), t.x.rpc_ok(&format!("{svc}.export"), json!({})).await);
    }
    let x2 = boot_node("node-x2", dir, Some(t.z.registry_url.clone()), &t.x_owner).await;
    for svc in DATA_SERVICES {
        let imported = x2.rpc(&format!("{svc}.import"), json!({ "bundle": bundles[*svc] })).await;
        assert!(imported.get("error").is_none(), "{svc} import onto the clean node: {imported}");
    }

    let held = x2.rpc_ok("directory.memberships", json!({ "member_did": t.y_owner_did })).await;
    assert_eq!(held["memberships"][0]["verdict"]["state"], "revoked", "{held}");

    let booking = x2.rpc_ok("booking.get", json!({ "agreement": quote_record_id })).await;
    assert_eq!(booking["state"], "completed", "the finished booking came across: {booking}");

    let agreement = x2.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    let verified = x2
        .rpc_ok("agreement.verify", json!({ "envelope": agreement["consumer"]["envelope"] }))
        .await;
    assert_eq!(verified["verified"], true, "X's own signed agreement still verifies: {verified}");
    x2.teardown().await;
}

#[tokio::test]
async fn a_consumer_hires_a_member_found_through_a_synorg_on_a_third_installation() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }
    let started = Instant::now();
    let dirs =
        [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let x2_dir = tempfile::tempdir().unwrap();

    let t = boot_trio(&[dirs[0].path(), dirs[1].path(), dirs[2].path()]).await;
    let (listing_id, slot_id) = set_up_synorg_and_provider(&t).await;
    let (credential_id, expires_at_secs) = admit_the_provider(&t, &listing_id).await;
    consumer_adds_the_directory(&t).await;
    a_stranger_resolves_only_what_is_published(&t).await;
    let hit = consumer_searches(&t, &listing_id, expires_at_secs).await;
    let quote = hire_the_provider(&t, &hit, &listing_id, &slot_id).await;
    suspension_reaches_the_consumer_only_on_check(&t).await;
    revocation_reaches_the_consumer_on_check(&t, &credential_id).await;
    consumer_leaves_with_their_data(&t, x2_dir.path(), &quote).await;

    eprintln!("roym_trust_e2e wall time: {:?}", started.elapsed());
    t.x.teardown().await;
    t.y.teardown().await;
    t.z.teardown().await;
}

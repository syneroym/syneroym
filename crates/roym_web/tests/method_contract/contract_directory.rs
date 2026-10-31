//! Parameter error tests and contract checks for directory and web methods.

use serde_json::{Value, json};
use syneroym_roym_core::services;

use super::{
    contract::post_rpc,
    fixtures::{
        both_rpc, enrol_signing, ensure_synorg, env, full_listing_params, wire_invoke_with,
    },
    helpers::{HttpDriver, foreign_searcher_caller, harness, owner_did},
    trust_fixtures::{FIXTURE_CATEGORIES, grant_owner_credential, stranger_did},
};

#[tokio::test]
async fn directory_parameter_errors() {
    let h = harness().await;

    // member.add with unknown key colour
    let (w, _) =
        post_rpc(&h, "member.add", Some(json!({ "did": stranger_did(), "colour": "blue" }))).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "colour");

    // credential.issue with expires_at_secs: "soon"
    let (w, _) = post_rpc(
        &h,
        "credential.issue",
        Some(json!({
            "member_did": stranger_did(),
            "categories": FIXTURE_CATEGORIES,
            "expires_at_secs": "soon",
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "expires_at_secs");

    // member.suspend with until_secs: -1
    let (w, _) = post_rpc(
        &h,
        "member.suspend",
        Some(json!({
            "member_did": stranger_did(),
            "until_secs": -1,
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "until_secs");

    // directory.set-settings: unknown top-level key rejected, unknown key inside
    // publication_limits accepted
    let valid_settings = json!({
        "name": "Guild",
        "rules": "r",
        "area": [],
        "categories": FIXTURE_CATEGORIES,
        "support_contact": "s@example.org",
        "dispute_path": "d",
        "retention_secs": 2_592_000,
        "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
    });
    let mut bad_settings = valid_settings.clone();
    bad_settings["unknown_top"] = json!("bad");
    let (w, _) = post_rpc(&h, "directory.set-settings", Some(bad_settings)).await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "unknown_top");

    let mut nested_unknown = valid_settings.clone();
    nested_unknown["publication_limits"]["unknown_nested"] = json!("accepted");
    let (w, _) = post_rpc(&h, "directory.set-settings", Some(nested_unknown)).await;
    assert!(w.get("error").is_none(), "nested unknown key should be accepted: {w}");
    assert!(w.get("result").is_some());

    // directory.query-source with invalid open_to in query
    let (rw, _) = post_rpc(&h, "directory.start-run", None).await;
    let run_id = rw["result"]["run_id"].as_str().unwrap();
    let (w, _) = post_rpc(
        &h,
        "directory.query-source",
        Some(json!({
            "run_id": run_id,
            "source": "did:key:hForeignWire",
            "query": { "open_to": "everyone" }
        })),
    )
    .await;
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "query.open_to");
    assert_eq!(
        w["error"]["message"].as_str().unwrap(),
        "invalid params: query.open_to: unknown variant `everyone`, expected one of `anyone`, \
         `members`, `referral`, `past-counterparties`"
    );
}

#[tokio::test]
async fn directory_wire_unknown_fields_and_parameters() {
    let h = harness().await;

    // directory.search over wire with open_to: "everyone"
    let (w, n) = h
        .wire_invoke(
            services::DIRECTORY,
            &env("directory.search", json!({ "open_to": "everyone" })),
        )
        .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "open_to");

    // directory.search over wire with {"text": "hedge", "zz": 1}
    let (w, n) = h
        .wire_invoke(
            services::DIRECTORY,
            &env("directory.search", json!({ "text": "hedge", "zz": 1 })),
        )
        .await;
    assert_eq!(w, n);
    assert!(w.get("error").is_none(), "wire search with extra field should succeed: {w}");
    assert!(w.get("result").is_some());

    // directory.standing over wire with member_did and "zz": 1
    let (w, n) = h
        .wire_invoke(
            services::DIRECTORY,
            &env("directory.standing", json!({ "member_did": owner_did(), "zz": 1 })),
        )
        .await;
    assert_eq!(w, n);
    assert!(w.get("error").is_none(), "wire standing with extra field should succeed: {w}");
    assert!(w.get("result").is_some());

    // directory.info over wire with "zz": 1
    let (w, n) =
        h.wire_invoke(services::DIRECTORY, &env("directory.info", json!({ "zz": 1 }))).await;
    assert_eq!(w, n);
    assert!(w.get("error").is_none(), "wire info with extra field should succeed: {w}");
    assert!(w.get("result").is_some());

    // directory.publish over wire with valid envelope and "zz": 1
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (lw, _) = post_rpc(
        &h,
        "listing.set",
        Some(full_listing_params("wire-publish-extra-key", "Extra key listing")),
    )
    .await;
    let listing_id = lw["result"]["listing_id"].as_str().unwrap();
    let (gw, _) = post_rpc(&h, "listing.get", Some(json!({ "listing_id": listing_id }))).await;
    let envelope = gw["result"]["envelope"].as_str().unwrap();

    let (w, n) = h
        .wire_invoke(
            services::DIRECTORY,
            &env("directory.publish", json!({ "envelope": envelope, "zz": 1 })),
        )
        .await;
    assert_eq!(w, n);
    assert!(w.get("error").is_none(), "wire publish with extra field should succeed: {w}");
    assert_eq!(w["result"]["listing_id"], listing_id);
    assert!(w["result"]["record_id"].is_string());
}

#[tokio::test]
async fn whoami_rpc_rejects_unknown_fields() {
    let h = harness().await;
    let req =
        json!({ "jsonrpc": "2.0", "id": 42, "method": "session.whoami", "params": { "zz": 1 } })
            .to_string()
            .into_bytes();
    let r_w = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let r_n = h.native_http.post("/rpc", req, Some(h.caller())).await;
    let w: Value = serde_json::from_slice(&r_w.body).unwrap();
    let n: Value = serde_json::from_slice(&r_n.body).unwrap();
    assert_eq!(w, n);
    assert_eq!(w["id"], 42);
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "zz");
}

#[tokio::test]
async fn a_wire_publish_is_recorded_under_the_verified_caller() {
    let h = harness().await;

    // 1. Set limit to 1 per window
    let (w, n) = both_rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "RateLimitedGuild",
            "rules": "r",
            "area": [],
            "categories": FIXTURE_CATEGORIES,
            "support_contact": "s@example.org",
            "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 1 }
        }),
    )
    .await;
    assert_eq!(w, n);
    assert!(w.get("result").is_some());
    grant_owner_credential(&h).await;

    enrol_signing(&h, "catalog").await;
    let (l1, _) = post_rpc(
        &h,
        "listing.set",
        Some(full_listing_params("wire-caller-l1", "Caller listing 1")),
    )
    .await;
    let (g1, _) =
        post_rpc(&h, "listing.get", Some(json!({ "listing_id": l1["result"]["listing_id"] })))
            .await;
    let env1 = g1["result"]["envelope"].as_str().unwrap();

    // 2. Wire publish with foreign_searcher_caller()
    let caller = foreign_searcher_caller();
    let (pw1, pn1) = wire_invoke_with(
        &h,
        services::DIRECTORY,
        &env("directory.publish", json!({ "envelope": env1 })),
        caller.clone(),
    )
    .await;
    assert_eq!(pw1, pn1);
    assert!(pw1.get("result").is_some(), "first publish by foreign searcher should succeed: {pw1}");

    // Local directory.publications: published_by is did:key:zForeignSearcher
    let (pubs_w, pubs_n) = post_rpc(&h, "directory.publications", None).await;
    assert_eq!(pubs_w, pubs_n);
    let rows = pubs_w["result"]["publications"].as_array().unwrap();
    let row =
        rows.iter().find(|r| r["listing_id"] == l1["result"]["listing_id"]).expect("row present");
    assert_eq!(row["published_by"], "did:key:zForeignSearcher");

    // 3. Second wire publish by the same caller is refused with data.admission ==
    //    "rate-limited"
    let (l2, _) = post_rpc(
        &h,
        "listing.set",
        Some(full_listing_params("wire-caller-l2", "Caller listing 2")),
    )
    .await;
    let (g2, _) =
        post_rpc(&h, "listing.get", Some(json!({ "listing_id": l2["result"]["listing_id"] })))
            .await;
    let env2 = g2["result"]["envelope"].as_str().unwrap();

    let (pw2, pn2) = wire_invoke_with(
        &h,
        services::DIRECTORY,
        &env("directory.publish", json!({ "envelope": env2 })),
        caller,
    )
    .await;
    assert_eq!(pw2, pn2);
    assert_eq!(pw2["error"]["data"]["admission"], "rate-limited");

    // 4. Local publish by the owner after that is accepted: limit is keyed on
    //    caller, not owner
    let (pw3, pn3) = post_rpc(&h, "directory.publish", Some(json!({ "envelope": env2 }))).await;
    assert_eq!(pw3, pn3);
    assert!(pw3.get("result").is_some(), "local publish by owner should succeed: {pw3}");
}

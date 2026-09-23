use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use syneroym_core::config::AppSandboxRole;
use syneroym_roym_core::{
    directory::{
        DEFAULT_SOURCE_TIMEOUT_MS, DISPATCH_HEADROOM_MS, MAX_CLIENT_CONCURRENCY,
        MAX_HITS_PER_SOURCE, MAX_REFUSED_RESULTS,
    },
    services,
};
use syneroym_rpc::AuthLevel;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_76_directory_info_over_the_wire_has_no_roster_parity() {
    let h = harness().await;
    both_rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "Rules text", "area": [], "categories": [],
            "support_contact": "s@example.org", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
    both_rpc(&h, "member.add", json!({ "did": "did:key:zMember1", "note": "" })).await;

    for auth in [AuthLevel::Delegated, AuthLevel::System] {
        let (w, n) =
            wire_invoke_as(&h, services::DIRECTORY, &env("directory.info", json!({})), auth).await;
        assert_eq!(w, n);
        assert_eq!(w["result"]["name"], "Guild");
        assert_eq!(w["result"]["member_count"], 1);
        assert!(w["result"].get("members").is_none(), "info leaked a roster: {w}");
    }
}

#[tokio::test]
async fn scenario_77_member_add_list_remove_round_trip_parity() {
    let h = harness().await;
    let (aw, an) =
        both_rpc(&h, "member.add", json!({ "did": "did:key:zM", "note": "trusted" })).await;
    assert_eq!(stripped(&aw), stripped(&an));
    let (lw, ln) = both_rpc(&h, "member.list", json!({})).await;
    assert_eq!(stripped(&lw), stripped(&ln));
    assert_eq!(lw["result"]["members"][0]["note"], "trusted");
    let (rw, rn) = both_rpc(&h, "member.remove", json!({ "did": "did:key:zM" })).await;
    assert_eq!(rw, rn);
    assert_eq!(rw["result"]["removed"], true);
}

#[tokio::test]
async fn scenario_78_member_list_over_the_wire_is_refused_parity() {
    let h = harness().await;
    let (w, n) = h.wire_invoke(services::DIRECTORY, &env("member.list", json!({}))).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32013);
}

#[tokio::test]
async fn scenario_79_publish_from_verified_wire_caller_stores_the_envelope_byte_for_byte_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-79", "Hedge trimming")).await;
    let envelope = gw["result"]["envelope"].as_str().unwrap().to_string();

    let (pw, pn) = publish_signed_listing(&h, &envelope).await;
    assert_eq!(pw, pn);
    assert!(pw["result"]["listing_id"].is_string(), "{pw}");

    let (sw, sn) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    let hits = sw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["envelope"], json!(envelope), "the directory must store the exact bytes");
}

#[tokio::test]
async fn scenario_80_publish_from_anonymous_wire_caller_is_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-80", "Hedge trimming")).await;
    let envelope = gw["result"]["envelope"].as_str().unwrap().to_string();

    let (w, n) = wire_invoke_as(
        &h,
        services::DIRECTORY,
        &env("directory.publish", json!({ "envelope": envelope })),
        AuthLevel::System,
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32013);
}

#[tokio::test]
async fn scenario_81_publish_of_a_tampered_envelope_is_refused_with_a_reason_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-81", "Hedge trimming")).await;
    let mut envelope: Value =
        serde_json::from_str(gw["result"]["envelope"].as_str().unwrap()).unwrap();
    envelope["payload"]["title"] = json!("Tampered");

    let (w, n) = publish_signed_listing(&h, &envelope.to_string()).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert!(w["error"]["message"].as_str().unwrap().contains("signature"), "{w}");
}

#[tokio::test]
async fn scenario_83_publication_limiter_refuses_past_the_budget_with_a_usable_retry_after_secs_parity()
 {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    both_rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "r", "area": [], "categories": [],
            "support_contact": "s", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 1 }
        }),
    )
    .await;

    let (_id, gw, _gn) = set_and_get(&h, full_listing_params("hedge-trimming-83a", "First")).await;
    let e1 = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (pw1, pn1) = publish_signed_listing(&h, &e1).await;
    assert_eq!(pw1, pn1);
    assert!(pw1["result"].is_object(), "{pw1}");

    let (_id2, gw2, _gn2) =
        set_and_get(&h, full_listing_params("hedge-trimming-83b", "Second")).await;
    let e2 = gw2["result"]["envelope"].as_str().unwrap().to_string();
    let (pw2, pn2) = publish_signed_listing(&h, &e2).await;
    assert_eq!(stripped(&pw2), stripped(&pn2));
    assert_eq!(pw2["error"]["code"], -32602);
    assert!(pw2["error"]["data"]["retry_after_secs"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn scenario_84_a_withdrawn_publication_consumes_no_budget_and_clears_the_index_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-84", "Withdraw me")).await;
    let e1 = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e1).await;

    both_rpc(&h, "listing.withdraw", json!({ "listing_id": id })).await;
    let (gw2, gn2) = both_rpc(&h, "listing.get", json!({ "listing_id": id })).await;
    assert_eq!(stripped(&gw2), stripped(&gn2));
    let e2 = gw2["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = publish_signed_listing(&h, &e2).await;
    assert_eq!(pw, pn);
    assert_eq!(pw["result"]["withdrawn"], true);

    let (sw, sn) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert_eq!(sw["result"]["hits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_84b_republishing_with_fewer_service_areas_leaves_no_stale_index_rows_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let five_areas: Vec<Value> = (0..5)
        .map(|i| json!({ "kind": "circle", "lat_e6": 52_000_000 + i, "lon_e6": 13_000_000, "radius_m": 1000 }))
        .collect();
    let mut params = full_listing_params("hedge-trimming-84b", "Many areas");
    params["location"]["service_area"] = json!(five_areas);
    let (_id, gw, _gn) = set_and_get(&h, params).await;
    let e1 = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e1).await;

    let mut params2 = full_listing_params("hedge-trimming-84b", "Many areas");
    params2["location"]["service_area"] = json!([five_areas[0].clone(), five_areas[1].clone()]);
    let (_id2, gw2, _gn2) = set_and_get(&h, params2).await;
    let e2 = gw2["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = publish_signed_listing(&h, &e2).await;
    assert_eq!(pw, pn);
    assert!(pw["result"]["listing_id"].is_string(), "{pw}");

    let (sw, sn) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "area": { "kind": "bbox", "min_lat_e6": 0, "min_lon_e6": 0, "max_lat_e6": 90_000_000, "max_lon_e6": 90_000_000 } })),
    )
    .await;
    assert_eq!(stripped(&sw), stripped(&sn));
    // One listing, whatever the surviving area count -- if a stale row
    // referencing the old record_id survived, this would either double the
    // hit or leave a dangling reference `directory.search` cannot resolve.
    assert_eq!(sw["result"]["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_84c_a_draft_listing_is_refused_at_publish_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let mut params = full_listing_params("hedge-trimming-84c", "Draft");
    params["status"] = json!("draft");
    let (_id, gw, _gn) = set_and_get(&h, params).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (w, n) = publish_signed_listing(&h, &e).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert!(w["error"]["message"].as_str().unwrap().contains("draft"), "{w}");
}

#[tokio::test]
async fn scenario_86_search_by_category_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-86", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (hit_w, hit_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "categories": ["gardening"] })),
    )
    .await;
    assert_eq!(stripped(&hit_w), stripped(&hit_n));
    assert_eq!(hit_w["result"]["hits"].as_array().unwrap().len(), 1);

    let (miss_w, miss_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "categories": ["plumbing"] })),
    )
    .await;
    assert_eq!(stripped(&miss_w), stripped(&miss_n));
    assert_eq!(miss_w["result"]["hits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_87_search_by_free_text_case_insensitive_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-87", "Hedge Trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (w, n) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({ "text": "HEDGE" })))
            .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_88_89_geometric_search_refines_exactly_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    // The listing's own circle is centred at (52.0, 13.0) with radius 5 km.
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-89", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    // A box that intersects the listing's own bounding box.
    let (hit_w, hit_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env(
            "directory.search",
            json!({ "area": { "kind": "bbox", "min_lat_e6": 51_990_000, "min_lon_e6": 12_990_000, "max_lat_e6": 52_010_000, "max_lon_e6": 13_010_000 } }),
        ),
    )
    .await;
    assert_eq!(stripped(&hit_w), stripped(&hit_n));
    assert_eq!(hit_w["result"]["hits"].as_array().unwrap().len(), 1);
    assert_eq!(hit_w["result"]["hits"][0]["area_match"]["kind"], "geometric");

    // A box inside the circle's over-covering bounding box corner, but far
    // outside the true 5 km circle -- the exact refinement, not the sieve.
    let (miss_w, miss_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env(
            "directory.search",
            json!({ "area": { "kind": "bbox", "min_lat_e6": 52_040_000, "min_lon_e6": 13_060_000, "max_lat_e6": 52_041_000, "max_lon_e6": 13_061_000 } }),
        ),
    )
    .await;
    assert_eq!(stripped(&miss_w), stripped(&miss_n));
    assert_eq!(
        miss_w["result"]["hits"].as_array().unwrap().len(),
        0,
        "the sieve's over-coverage must be refined away: {miss_w}"
    );
}

#[tokio::test]
async fn scenario_90_a_named_area_listing_matches_only_by_label_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let mut params = full_listing_params("hedge-trimming-90", "Hedge trimming");
    params["location"]["service_area"] = json!([{ "kind": "named", "label": "Bengaluru" }]);
    let (_id, gw, _gn) = set_and_get(&h, params).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (geo_w, geo_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "area": { "kind": "bbox", "min_lat_e6": 0, "min_lon_e6": 0, "max_lat_e6": 90_000_000, "max_lon_e6": 90_000_000 } })),
    )
    .await;
    assert_eq!(stripped(&geo_w), stripped(&geo_n));
    assert_eq!(
        geo_w["result"]["hits"].as_array().unwrap().len(),
        0,
        "a named-only area must not match a geometric query"
    );

    let (label_w, label_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "area": { "kind": "named", "label": "bengaluru" } })),
    )
    .await;
    assert_eq!(stripped(&label_w), stripped(&label_n));
    assert_eq!(label_w["result"]["hits"].as_array().unwrap().len(), 1);
    assert_eq!(label_w["result"]["hits"][0]["area_match"]["kind"], "named");
}

#[tokio::test]
async fn scenario_91_no_location_block_shows_only_under_a_no_area_query_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let mut params = full_listing_params("hedge-trimming-91", "Hedge trimming");
    params.as_object_mut().unwrap().remove("location");
    let (_id, gw, _gn) = set_and_get(&h, params).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (none_w, none_n) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&none_w), stripped(&none_n));
    assert_eq!(none_w["result"]["hits"].as_array().unwrap().len(), 1);
    assert_eq!(none_w["result"]["hits"][0]["area_match"]["kind"], "no-area-stated");

    let (geo_w, geo_n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "area": { "kind": "bbox", "min_lat_e6": 0, "min_lon_e6": 0, "max_lat_e6": 90_000_000, "max_lon_e6": 90_000_000 } })),
    )
    .await;
    assert_eq!(stripped(&geo_w), stripped(&geo_n));
    assert_eq!(geo_w["result"]["hits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_93_a_search_response_carries_no_verification_verdict_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-93", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    let hit = &w["result"]["hits"][0];
    for key in ["verified", "revocation_status", "credential"] {
        assert!(hit.get(key).is_none(), "a directory's own answer must carry no '{key}': {w}");
    }
}

#[tokio::test]
async fn scenario_94_search_over_the_wire_anonymous_succeeds_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-94", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (w, n) = wire_invoke_as(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({})),
        AuthLevel::System,
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_96_client_sources_add_list_remove_round_trip_parity() {
    let h = harness().await;
    let (aw, an) = both_rpc(
        &h,
        "directory.add-source",
        json!({ "did": "did:key:hForeign", "label": "Neighbour Guild" }),
    )
    .await;
    assert_eq!(stripped(&aw), stripped(&an));
    let (lw, ln) = both_rpc(&h, "directory.sources", json!({})).await;
    assert_eq!(stripped(&lw), stripped(&ln));
    assert_eq!(lw["result"]["sources"].as_array().unwrap().len(), 1);
    let (rw, rn) =
        both_rpc(&h, "directory.remove-source", json!({ "did": "did:key:hForeign" })).await;
    assert_eq!(rw, rn);
    assert_eq!(rw["result"]["removed"], true);
}

#[tokio::test]
async fn scenario_97_client_fan_out_over_one_source_yields_a_merged_hit_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-97", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    // Published as this node's own SynOrg, through the wire path, so it is
    // reachable by `directory.search` on `did:key:hForeign` -- which
    // `TestWasmServiceProxy`/`TestNativeServiceProxy` route back to this
    // very directory over the local dispatch path.
    publish_signed_listing(&h, &e).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    // Each build mints and uses its own run id: `start-run` folds the
    // guest's own (unsynchronized) wall clock into the id.
    let (run_w, mw) = fan_out_one(&h, true, &["did:key:hForeign"]).await;
    let (run_n, mn) = fan_out_one(&h, false, &["did:key:hForeign"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["sources"][0]["directory"], "did:key:hForeign");
    assert!(
        hits[0].get("envelope").is_none(),
        "merge must return projections, not envelopes: {mw}"
    );

    let record_id = hits[0]["record_id"].as_str().unwrap();
    let ew = one_rpc(
        &h,
        true,
        "directory.run-envelope",
        json!({ "run_id": run_w, "record_id": record_id }),
    )
    .await;
    let en = one_rpc(
        &h,
        false,
        "directory.run-envelope",
        json!({ "run_id": run_n, "record_id": record_id }),
    )
    .await;
    assert_eq!(ew, en);
    assert_eq!(ew["result"]["envelope"], json!(e));
}

#[tokio::test]
async fn scenario_101_a_run_with_zero_sources_succeeds_with_zero_hits_parity() {
    let h = harness().await;
    // Per build, minting each build's own run id (the id folds in the
    // guest's own wall clock) -- `start-run`'s raw response is never
    // compared directly across builds for that reason.
    let start_w = one_rpc(&h, true, "directory.start-run", json!({})).await;
    let start_n = one_rpc(&h, false, "directory.start-run", json!({})).await;
    assert_eq!(start_w["result"]["sources"], json!([]));
    assert_eq!(start_n["result"]["sources"], json!([]));
    assert_eq!(start_w["result"]["max_concurrency"], start_n["result"]["max_concurrency"]);
    let run_w = start_w["result"]["run_id"].as_str().unwrap().to_string();
    let run_n = start_n["result"]["run_id"].as_str().unwrap().to_string();

    let mw = one_rpc(&h, true, "directory.merge", json!({ "run_id": run_w })).await;
    let mn = one_rpc(&h, false, "directory.merge", json!({ "run_id": run_n })).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    assert_eq!(mw["result"]["hits"], json!([]));
    assert!(mw["result"].get("error").is_none());
}

#[tokio::test]
async fn scenario_102_query_source_refuses_an_unregistered_source_and_a_foreign_run_id_parity() {
    let h = harness().await;

    // `start-run` folds the guest's own wall clock into the run id, and the
    // two builds' clocks are unsynchronized -- so each build is driven with
    // the run id it minted itself, never the other build's.
    for wasm in [true, false] {
        let start = one_rpc(&h, wasm, "directory.start-run", json!({})).await;
        let run_id = start["result"]["run_id"].as_str().unwrap().to_string();

        let u = one_rpc(
            &h,
            wasm,
            "directory.query-source",
            json!({ "run_id": run_id, "source": "did:key:hForeign", "query": {} }),
        )
        .await;
        assert!(is_err(&u, -32602), "an unregistered source must be refused: {u}");

        one_rpc(&h, wasm, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
        let r = one_rpc(
            &h,
            wasm,
            "directory.query-source",
            json!({
                "run_id": "run_this_node_never_minted",
                "source": "did:key:hForeign",
                "query": {}
            }),
        )
        .await;
        assert!(is_err(&r, -32602), "a run_id this node did not mint must be refused: {r}");
    }
}

#[tokio::test]
async fn scenario_106_listing_history_returns_only_the_named_listings_versions_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (id_a, _, _) = set_and_get(&h, full_listing_params("hedge-trimming-106a", "A")).await;
    let (id_b, _, _) = set_and_get(&h, full_listing_params("hedge-trimming-106b", "B")).await;
    set_and_get(&h, {
        let mut p = full_listing_params("hedge-trimming-106a", "A v2");
        p["slug"] = json!("hedge-trimming-106a");
        p
    })
    .await;

    let (w, n) = both_rpc(&h, "listing.history", json!({ "listing_id": id_a })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["history"].as_array().unwrap().len(), 2);
    let (w2, n2) = both_rpc(&h, "listing.history", json!({ "listing_id": id_b })).await;
    assert_eq!(w2, n2);
    assert_eq!(w2["result"]["history"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_106b_every_client_half_verb_is_refused_over_the_wire_parity() {
    let h = harness().await;
    for method in [
        "directory.start-run",
        "directory.query-source",
        "directory.merge",
        "directory.run-envelope",
    ] {
        let (w, n) = h.wire_invoke(services::DIRECTORY, &env(method, json!({}))).await;
        assert_eq!(w, n, "{method}");
        assert_eq!(w["error"]["code"], -32013, "{method}: {w}");
    }
    let (sw, sn) = h.wire_invoke(services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert_ne!(sw["error"]["code"].as_i64(), Some(-32013));
}

#[tokio::test]
async fn scenario_109_no_wire_reachable_method_calls_a_sibling_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    both_rpc(&h, "member.add", json!({ "did": "did:key:zMember109" })).await;
    enrol_signing(&h, "catalog").await;

    // Real state, not an empty directory: a published listing search can
    // return, settings info can read, and a valid envelope publish can
    // reach the handler body -- so the proxy counter has a genuine chance
    // to move if any handler were to call a sibling.
    let e = publish_listing_to_primary(&h, "hedge-109", "Hedge 109").await;

    let before_w = h.wasm_proxy.invocations.load(Ordering::SeqCst);
    let before_n = h.native_proxy.invocations.load(Ordering::SeqCst);

    // A valid-envelope publish (reaches the handler, consumes budget, writes
    // rows), an anonymous search that returns the hit, and info.
    let (_pw, _pn) = publish_signed_listing(&h, &e).await;
    let (sw, _sn) = wire_invoke_as(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({})),
        AuthLevel::System,
    )
    .await;
    assert!(
        !sw["result"]["hits"].as_array().unwrap().is_empty(),
        "the search must actually return a hit for this to prove anything: {sw}"
    );
    h.wire_invoke(services::DIRECTORY, &env("directory.info", json!({}))).await;
    // And the original degenerate case: a junk envelope returns before the
    // handler does anything.
    h.wire_invoke(services::DIRECTORY, &env("directory.publish", json!({ "envelope": "x" }))).await;

    assert_eq!(
        h.wasm_proxy.invocations.load(Ordering::SeqCst),
        before_w,
        "a wire-reachable directory method made a wasm proxy call"
    );
    assert_eq!(
        h.native_proxy.invocations.load(Ordering::SeqCst),
        before_n,
        "a wire-reachable directory method made a native proxy call"
    );
}

#[tokio::test]
async fn scenario_118_exactly_three_directory_verbs_are_wire_reachable_parity() {
    let h = harness().await;
    for &method in ALL_DIRECTORY_VERBS {
        // Every listed verb must be a real dispatch arm, not a stale or
        // mistyped name: a local call must not answer method-not-found.
        let (lw, ln) = h.local_invoke(services::DIRECTORY, &env(method, json!({}))).await;
        for (label, v) in [("wasm", &lw), ("native", &ln)] {
            assert_ne!(
                v["error"]["code"].as_i64(),
                Some(-32601),
                "{label} {method} is in ALL_DIRECTORY_VERBS but the dispatch does not know it: {v}"
            );
        }

        let (w, n) = h.wire_invoke(services::DIRECTORY, &env(method, json!({}))).await;
        let reachable = WIRE_REACHABLE_DIRECTORY_VERBS.contains(&method);
        for (label, v) in [("wasm", &w), ("native", &n)] {
            let code = v["error"]["code"].as_i64();
            if reachable {
                assert_ne!(
                    code,
                    Some(-32013),
                    "{label} {method} must be reachable over the wire: {v}"
                );
            } else {
                assert_eq!(
                    code,
                    Some(-32013),
                    "{label} {method} must answer -32013 over the wire: {v}"
                );
            }
        }
    }
}

#[tokio::test]
async fn scenario_110_search_with_an_extreme_radius_is_refused_not_a_crash_parity() {
    let h = harness().await;
    // Reachable by an anonymous stranger; before the fix this drove
    // `bounding_box`/`areas_intersect` into `i64`/`u64` overflow instead
    // of being refused by `Area::validate`.
    let (w, n) = wire_invoke_as(
        &h,
        services::DIRECTORY,
        &env(
            "directory.search",
            json!({ "area": { "kind": "circle", "lat_e6": 1, "lon_e6": 1, "radius_m": u64::MAX } }),
        ),
        AuthLevel::System,
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602, "{w}");
}

#[tokio::test]
async fn scenario_111_search_with_too_many_categories_is_refused_parity() {
    let h = harness().await;
    let categories: Vec<String> = (0..1000).map(|i| format!("cat{i}")).collect();
    let (w, n) = wire_invoke_as(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "categories": categories })),
        AuthLevel::System,
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602, "{w}");
}

#[tokio::test]
async fn scenario_112_publish_is_refused_on_a_node_with_no_synorg_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    // Deliberately no `ensure_synorg`: this node has never declared
    // itself a SynOrg (no `settings` row).
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-112", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (w, n) = publish_signed_listing(&h, &e).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602, "{w}");
    assert!(w["error"]["message"].as_str().unwrap().contains("no SynOrg"), "{w}");
}

#[tokio::test]
async fn scenario_113_a_local_publish_uses_this_installations_own_owner_as_published_by_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    ensure_synorg(&h).await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-113", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    // `both_rpc` drives this through `web`'s local dispatch path (`
    // Caller::Internal`), not the wire -- the case the review found
    // `publish()` could never complete before.
    let (pw, pn) = both_rpc(&h, "directory.publish", json!({ "envelope": e })).await;
    assert_eq!(pw, pn);
    assert!(pw["result"]["listing_id"].is_string(), "{pw}");

    let (lw, ln) = both_rpc(&h, "directory.publications", json!({})).await;
    assert_eq!(stripped(&lw), stripped(&ln));
    let pubs = lw["result"]["publications"].as_array().unwrap();
    assert_eq!(pubs.len(), 1);
    assert_eq!(pubs[0]["published_by"], h.owner_did);
}

#[tokio::test]
async fn scenario_114_search_filters_by_the_serde_spelling_of_a_multi_word_enum_value_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    ensure_synorg(&h).await;
    let mut params = full_listing_params("hedge-trimming-114", "Hedge trimming");
    params["relationship"] = json!({ "open_to": "existing-customers" });
    let (_id, gw, _gn) = set_and_get(&h, params).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    // Before the fix this indexed as `Debug`'s "existingcustomers" and a
    // query for the documented, serde-spelled value matched nothing.
    let (w, n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "open_to": "existing-customers" })),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["hits"].as_array().unwrap().len(), 1, "{w}");
}

#[tokio::test]
async fn scenario_115_a_replayed_older_envelope_is_refused_while_a_same_second_edit_is_not_parity()
{
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    ensure_synorg(&h).await;

    let (_id, gw1, _gn1) =
        set_and_get(&h, full_listing_params("hedge-trimming-115", "Version one")).await;
    let e1 = gw1["result"]["envelope"].as_str().unwrap().to_string();
    let (pw1, pn1) = publish_signed_listing(&h, &e1).await;
    assert_eq!(pw1, pn1);
    assert!(pw1["result"]["listing_id"].is_string(), "{pw1}");

    // A same-second edit: the parity harness pins the signing clock, so
    // this envelope's `issued_at_secs` ties `e1`'s exactly -- only a
    // correct `supersedes` chain (not the timestamp) tells the two
    // apart.
    let (_id2, gw2, _gn2) =
        set_and_get(&h, full_listing_params("hedge-trimming-115", "Version two")).await;
    let e2 = gw2["result"]["envelope"].as_str().unwrap().to_string();
    let (pw2, pn2) = publish_signed_listing(&h, &e2).await;
    assert_eq!(pw2, pn2);
    assert!(pw2["result"]["listing_id"].is_string(), "a same-second edit must be accepted: {pw2}");

    // Replaying the *first* envelope now must be refused: it neither
    // supersedes the currently-stored version (e2) nor postdates it.
    let (rw, rn) = publish_signed_listing(&h, &e1).await;
    assert_eq!(rw, rn);
    assert_eq!(rw["error"]["code"], -32602, "a replayed older envelope must be refused: {rw}");
}

#[tokio::test]
async fn scenario_116_two_query_source_calls_for_one_source_in_one_run_do_not_duplicate_a_listing_parity()
 {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw1, _gn1) =
        set_and_get(&h, full_listing_params("hedge-trimming-116", "Version one")).await;
    let e1 = gw1["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e1).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    // Per build, minting each build's own run id (the id folds in the
    // guest's own wall clock). First `query-source`: stores a row for e1's
    // record_id.
    let mint_run = |v: Value| v["result"]["run_id"].as_str().unwrap().to_string();
    let run_w = mint_run(one_rpc(&h, true, "directory.start-run", json!({})).await);
    let run_n = mint_run(one_rpc(&h, false, "directory.start-run", json!({})).await);
    let qs = |run: &str| json!({ "run_id": run, "source": "did:key:hForeign", "query": {} });
    one_rpc(&h, true, "directory.query-source", qs(&run_w)).await;
    one_rpc(&h, false, "directory.query-source", qs(&run_n)).await;

    // A newer, same-second (pinned-clock) edit, superseding e1 -- a real
    // signed version, not a forged duplicate.
    let (_id2, gw2, _gn2) =
        set_and_get(&h, full_listing_params("hedge-trimming-116", "Version two")).await;
    let e2 = gw2["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e2).await;

    // Second call: same run, same source -- e.g. a client retry -- now
    // stores a *different* row (different record_id) for the same
    // listing_id. Before the fix, `merge`'s per-source list carried both,
    // so this one source would appear twice in one hit's `sources[]`.
    one_rpc(&h, true, "directory.query-source", qs(&run_w)).await;
    one_rpc(&h, false, "directory.query-source", qs(&run_n)).await;

    let mw = one_rpc(&h, true, "directory.merge", json!({ "run_id": run_w })).await;
    let mn = one_rpc(&h, false, "directory.merge", json!({ "run_id": run_n })).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{mw}");
    let sources = hits[0]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "one source must appear once, not twice: {mw}");
    assert_eq!(mw["result"]["hits"][0]["versions_differ"], false, "{mw}");
}

#[tokio::test]
async fn scenario_117_directory_export_import_round_trip_reindexes_and_carries_the_bumped_schema_version_parity()
 {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-117", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (xw, xn) = both_rpc(&h, "directory.export", json!({})).await;
    assert_eq!(stripped(&xw), stripped(&xn));
    for section in ["synorg", "publications", "members", "publication_log", "sources"] {
        assert_eq!(
            xw["result"]["manifest"]["sections"][section]["schema_version"], 3,
            "section '{section}' must carry the bumped schema version: {xw}"
        );
    }

    let (iw, in_) = both_rpc(&h, "directory.import", json!({ "bundle": xw["result"] })).await;
    assert_eq!(iw, in_);
    assert!(iw["result"]["reindexed"].as_u64().unwrap() >= 1, "import must reindex: {iw}");

    // The imported state must still answer a search -- proving the
    // projection, not just the publication row, survived the round trip.
    let (sw, sn) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert_eq!(sw["result"]["hits"].as_array().unwrap().len(), 1, "{sw}");
}

#[tokio::test]
async fn scenario_98_two_directories_disagreeing_about_a_version_merge_to_one_hit_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    // Version one into the second directory; version two (same slug -> same
    // listing_id, superseding version one) into the primary. The two
    // directories now genuinely hold different current versions of one
    // listing.
    let e1 = publish_listing_to_dir2(&h, "hedge-trimming-98", "Version one").await;
    let e2 = publish_listing_to_primary(&h, "hedge-trimming-98", "Version two").await;
    assert_ne!(e1, e2, "the two versions must be distinct signed envelopes");

    let (run_w, run_n, mw, mn) =
        fan_out(&h, &["did:key:hForeignWire", "did:key:hForeignWire2"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "one listing_id, whatever the version disagreement: {mw}");
    assert_eq!(hits[0]["versions_differ"], true, "{mw}");
    assert_eq!(
        hits[0]["sources"].as_array().unwrap().len(),
        2,
        "both directories must be listed as sources: {mw}"
    );

    // Whichever version won the merge, its bytes must be retrievable and be
    // one of the two we actually published -- `merge` returns projections,
    // never envelopes.
    assert!(hits[0].get("envelope").is_none(), "merge leaked an envelope: {mw}");
    let kept = hits[0]["record_id"].as_str().unwrap();
    let ew =
        one_rpc(&h, true, "directory.run-envelope", json!({ "run_id": run_w, "record_id": kept }))
            .await;
    let en =
        one_rpc(&h, false, "directory.run-envelope", json!({ "run_id": run_n, "record_id": kept }))
            .await;
    assert_eq!(ew, en);
    let got = ew["result"]["envelope"].as_str().unwrap();
    assert!(
        got == e1 || got == e2,
        "run-envelope must return a published envelope byte-for-byte: {ew}"
    );
}

#[tokio::test]
async fn scenario_102c_a_source_is_capped_at_its_per_source_share_of_the_merged_page_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    // The second directory holds more valid, recent listings than one
    // source is allowed to contribute to a merged page; the primary holds
    // two. Splitting the stores alone does not bound one source's share
    // of the page -- only the per-source cap in `merge` does.
    let over = MAX_HITS_PER_SOURCE as usize + 1;
    for i in 0..over {
        publish_listing_to_dir2(&h, &format!("crowd-{i}"), &format!("Crowd {i}")).await;
    }
    for i in 0..2 {
        publish_listing_to_primary(&h, &format!("primary-{i}"), &format!("Primary {i}")).await;
    }

    let (_rw, _rn, mw, mn) = fan_out(&h, &["did:key:hForeignWire", "did:key:hForeignWire2"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();

    let from = |dir: &str| {
        hits.iter()
            .filter(|hit| hit["sources"].as_array().unwrap().iter().any(|s| s["directory"] == dir))
            .count()
    };
    assert_eq!(
        from("did:key:hForeignWire2"),
        MAX_HITS_PER_SOURCE as usize,
        "the crowding source contributes exactly its share, not all {over}: {mw}"
    );
    assert_eq!(from("did:key:hForeignWire"), 2, "the other source's results all survive: {mw}");
    assert_eq!(hits.len(), MAX_HITS_PER_SOURCE as usize + 2);
}

#[tokio::test]
async fn scenario_102d_forged_sources_are_refused_round_robined_and_crowd_out_no_hits_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    for i in 0..3 {
        publish_listing_to_primary(&h, &format!("good-{i}"), &format!("Good {i}")).await;
    }

    let (_rw, _rn, mw, mn) =
        fan_out(&h, &["did:key:hForeignWire", "did:key:hForge1", "did:key:hForge2"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));

    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 3, "forged sources reduce the genuine hit count by nothing: {mw}");
    for hit in hits {
        assert_eq!(hit["verified"], true, "{mw}");
    }

    let refused = mw["result"]["refused"].as_array().unwrap();
    assert_eq!(refused.len(), MAX_REFUSED_RESULTS as usize, "refused evidence fills its cap: {mw}");
    let from = |dir: &str| {
        refused.iter().filter(|r| r["sources"].as_array().unwrap().iter().any(|s| s == dir)).count()
    };
    let (a, b) = (from("did:key:hForge1"), from("did:key:hForge2"));
    assert_eq!(a + b, MAX_REFUSED_RESULTS as usize, "{mw}");
    assert!(
        a >= MAX_REFUSED_RESULTS as usize / 3 && b >= MAX_REFUSED_RESULTS as usize / 3,
        "one forger must not dominate the refused block: hForge1={a} hForge2={b}: {mw}"
    );
}

#[tokio::test]
async fn scenario_119_merged_page_is_round_robin_order_not_listing_id_order_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    // Three listings in one directory, two in the other. `merge` visits
    // sources in DID order (`hForeignWire` < `hForeignWire2`), so
    // round-robin produces exactly [W, W2, W, W2, W]. Iterating the
    // `seen` set instead (a `BTreeSet`) would emit the page sorted by
    // `listing_id` -- a content hash -- which cannot equal that sequence
    // except by a 1-in-120 coincidence, and is never *un*sorted.
    for i in 0..3 {
        publish_listing_to_primary(&h, &format!("rr-p-{i}"), &format!("Primary {i}")).await;
    }
    for i in 0..2 {
        publish_listing_to_dir2(&h, &format!("rr-d-{i}"), &format!("Dir2 {i}")).await;
    }

    let (_rw, _rn, mw, mn) = fan_out(&h, &["did:key:hForeignWire", "did:key:hForeignWire2"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 5, "all five listings make the page: {mw}");

    let source_of = |hit: &Value| {
        hit["sources"].as_array().unwrap()[0]["directory"].as_str().unwrap().to_string()
    };
    let sequence: Vec<String> = hits.iter().map(source_of).collect();
    assert_eq!(
        sequence,
        vec![
            "did:key:hForeignWire",
            "did:key:hForeignWire2",
            "did:key:hForeignWire",
            "did:key:hForeignWire2",
            "did:key:hForeignWire",
        ],
        "the page must be in round-robin source order: {mw}"
    );

    // The bulletproof half: a `BTreeSet` iteration is *always* sorted by
    // listing_id, so this fails 100% of the time against that regression.
    let ids: Vec<String> =
        hits.iter().map(|h| h["listing_id"].as_str().unwrap().to_string()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_ne!(ids, sorted, "the merged page must not be sorted by listing_id: {mw}");
}

#[tokio::test]
async fn scenario_120_a_source_that_truncates_says_so_per_source_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    // `did:key:hTrunc` answers with no hits but `truncated: true` -- the
    // "500 listings, wanted rows hashed late, zero results" case. The flag
    // must ride the `query-source` reply, since a zero-row source writes
    // no run rows for `merge` to carry it on.
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hTrunc" })).await;
    for wasm in [true, false] {
        let start = one_rpc(&h, wasm, "directory.start-run", json!({})).await;
        let run_id = start["result"]["run_id"].as_str().unwrap().to_string();
        let reply = one_rpc(
            &h,
            wasm,
            "directory.query-source",
            json!({ "run_id": run_id, "source": "did:key:hTrunc", "query": {} }),
        )
        .await;
        assert_eq!(reply["result"]["verified"], 0, "{reply}");
        assert_eq!(
            reply["result"]["truncated"], true,
            "the directory's truncated flag must reach the per-source reply: {reply}"
        );
    }
}

#[tokio::test]
async fn scenario_121_a_named_area_query_discriminates_by_label_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let mut blr_id = String::new();
    for (slug, label) in [("area-blr", "Bengaluru"), ("area-bom", "Mumbai")] {
        let mut params = full_listing_params(slug, label);
        params["location"]["service_area"] = json!([{ "kind": "named", "label": label }]);
        let (id, gw, _gn) = set_and_get(&h, params).await;
        if label == "Bengaluru" {
            blr_id = id;
        }
        let e = gw["result"]["envelope"].as_str().unwrap().to_string();
        publish_signed_listing(&h, &e).await;
    }

    let (w, n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "area": { "kind": "named", "label": "BENGALURU" } })),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    let hits = w["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "only the Bengaluru listing matches: {w}");
    assert_eq!(hits[0]["listing_id"], blr_id, "{w}");
    assert_eq!(hits[0]["area_match"]["kind"], "named");
}

#[test]
fn directory_source_timeout_and_concurrency_fit_the_real_sandbox_defaults() {
    // The constants in `roym_core::directory` are derived from node
    // limits, not chosen -- and `roym_core` is a guest crate that cannot
    // import the config crate, so the relationship is asserted here,
    // against the real `AppSandboxRole` defaults rather than a literal
    // copied by hand.
    let defaults = AppSandboxRole::default();
    let epoch_ms = defaults.dispatch_epoch_timeout_secs.saturating_mul(1000);
    assert!(
        u64::from(DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS) < epoch_ms,
        "source timeout ({DEFAULT_SOURCE_TIMEOUT_MS}) + headroom ({DISPATCH_HEADROOM_MS}) must \
         fit inside the dispatch epoch ({epoch_ms} ms)"
    );
    assert!(
        (MAX_CLIENT_CONCURRENCY as u32) < defaults.max_concurrent_guest_http_per_service,
        "client fan-out concurrency ({MAX_CLIENT_CONCURRENCY}) must stay below the guest-HTTP \
         admission limit ({})",
        defaults.max_concurrent_guest_http_per_service
    );
}

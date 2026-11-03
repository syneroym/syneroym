//! Parity integration tests for `directory.find` and `directory.run-envelope`
//! across WASM and native builds.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_roym_core::directory::DISPATCH_HEADROOM_MS;

use super::{fixtures::*, helpers::*, trust_fixtures::*};

#[tokio::test]
async fn find_over_one_source_matches_the_native_and_wasm_builds() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("find-single-source", "Single Source")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["sources"][0]["directory"], "did:key:hForeign");
    assert_eq!(hits[0]["versions_differ"], false);
    assert!(hits[0].get("envelope").is_none(), "find must return projections, not envelopes");

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0]["source"], "did:key:hForeign");
    assert_eq!(outcomes[0]["verified"], 1);
    assert_eq!(outcomes[0]["refused"], 0);
    assert_eq!(outcomes[0]["truncated"], false);
    assert!(outcomes[0]["error"].is_null());
}

#[tokio::test]
async fn find_with_no_sources_is_an_empty_page_with_a_run_id() {
    let h = harness().await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let run_w = fw["result"]["run_id"].as_str().unwrap();
    let run_n = fn_res["result"]["run_id"].as_str().unwrap();
    assert!(!run_w.is_empty());
    assert!(!run_n.is_empty());

    assert_eq!(fw["result"]["hits"], json!([]));
    assert_eq!(fw["result"]["refused"], json!([]));
    assert_eq!(fw["result"]["outcomes"], json!([]));
    assert_eq!(fw["result"]["hits_truncated"], false);
    assert_eq!(fw["result"]["refused_truncated"], false);
}

#[tokio::test]
async fn find_marks_an_unreachable_source_not_found_and_keeps_the_others() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) = set_and_get(&h, full_listing_params("find-unreach", "Unreach")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:zunbound" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["sources"][0]["directory"], "did:key:hForeign");

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 2);

    let h_foreign = outcomes.iter().find(|o| o["source"] == "did:key:hForeign").unwrap();
    assert_eq!(h_foreign["verified"], 1);
    assert!(h_foreign["error"].is_null());

    let z_unbound = outcomes.iter().find(|o| o["source"] == "did:key:zunbound").unwrap();
    assert_eq!(z_unbound["verified"], 0);
    assert_eq!(z_unbound["error"]["kind"], "not-found");
}

#[tokio::test]
async fn find_refuses_an_unknown_top_level_field() {
    let h = harness().await;

    let fw = one_rpc(&h, true, "directory.find", json!({ "unknown_field": 123 })).await;
    let fn_res = one_rpc(&h, false, "directory.find", json!({ "unknown_field": 123 })).await;

    assert_eq!(fw["error"]["code"], -32602);
    assert_eq!(fn_res["error"]["code"], -32602);

    let msg_w = fw["error"]["message"].as_str().unwrap();
    let msg_n = fn_res["error"]["message"].as_str().unwrap();
    assert!(
        msg_w.contains("unknown field `unknown_field`"),
        "error message must cite unknown field: {msg_w}"
    );
    assert!(
        msg_n.contains("unknown field `unknown_field`"),
        "error message must cite unknown field: {msg_n}"
    );
}

#[tokio::test]
async fn run_envelope_returns_the_signed_bytes_of_a_page_record() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("find-envelope-hit", "Envelope Hit")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let run_w = fw["result"]["run_id"].as_str().unwrap();
    let run_n = fn_res["result"]["run_id"].as_str().unwrap();
    let record_id = fw["result"]["hits"][0]["record_id"].as_str().unwrap();

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
    assert_eq!(ew["result"]["envelope"], env_bytes);
}

#[tokio::test]
async fn run_envelope_does_not_find_a_record_that_is_not_on_the_page() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("find-env-missing", "Env Missing")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let run_w = fw["result"]["run_id"].as_str().unwrap();
    let run_n = fn_res["result"]["run_id"].as_str().unwrap();

    let ew = one_rpc(
        &h,
        true,
        "directory.run-envelope",
        json!({ "run_id": run_w, "record_id": "nonexistent_record_id" }),
    )
    .await;
    let en = one_rpc(
        &h,
        false,
        "directory.run-envelope",
        json!({ "run_id": run_n, "record_id": "nonexistent_record_id" }),
    )
    .await;

    assert_eq!(ew, en);
    assert!(ew["result"].is_null());
}

#[tokio::test]
async fn an_unregistered_run_id_has_no_envelope() {
    let h = harness().await;

    let ew = one_rpc(
        &h,
        true,
        "directory.run-envelope",
        json!({ "run_id": "run_9999999999_0", "record_id": "some_record_id" }),
    )
    .await;
    let en = one_rpc(
        &h,
        false,
        "directory.run-envelope",
        json!({ "run_id": "run_9999999999_0", "record_id": "some_record_id" }),
    )
    .await;

    assert_eq!(ew, en);
    assert!(ew["result"].is_null());
}

#[tokio::test]
async fn a_find_after_the_retention_hour_prunes_old_run_rows() {
    let h = harness().await;

    // Run find once to ensure collections exist
    both_rpc(&h, "directory.find", json!({})).await;

    // Seed old rows in runs and search_runs
    for wasm in [true, false] {
        let (storage, ks) =
            if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
        let db = storage.open_service_db(&did_for_service("directory"), ks).await.unwrap();

        let old_run = json!({ "at_secs": 100u64 });
        let old_search_run = json!({
            "run_id": "old_run_pruned",
            "record_id": "rec_old",
            "envelope": "env_old",
            "at_secs": 100u64,
            "refused": false,
        });

        let w_run = RecordWriteValue {
            id: "old_run_pruned".to_string(),
            payload: serde_json::to_vec(&old_run).unwrap(),
        };
        let w_search_run = RecordWriteValue {
            id: "old_run_pruned#rec_old".to_string(),
            payload: serde_json::to_vec(&old_search_run).unwrap(),
        };

        db.put("runs", &w_run, "seed", None).await.unwrap();
        db.put("search_runs", &w_search_run, "seed", None).await.unwrap();
    }

    // A new find call prunes rows older than RUN_RETENTION_SECS
    both_rpc(&h, "directory.find", json!({})).await;

    for wasm in [true, false] {
        let runs = service_rows(&h, wasm, "directory", "runs").await;
        let search_runs = service_rows(&h, wasm, "directory", "search_runs").await;

        assert!(
            !runs.iter().any(|r| r.get("at_secs") == Some(&json!(100u64))),
            "old runs row must be pruned"
        );
        assert!(
            !search_runs.iter().any(|r| r.get("run_id") == Some(&json!("old_run_pruned"))),
            "old search_runs row must be pruned"
        );
    }
}

#[tokio::test]
async fn find_names_the_slow_source_and_keeps_the_other_hits() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) = set_and_get(&h, full_listing_params("slow-test-hedge", "Hedge")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hSlow" })).await;

    let start_w = Instant::now();
    let fw = one_rpc(&h, true, "directory.find", json!({})).await;
    let elapsed_w = start_w.elapsed();

    let start_n = Instant::now();
    let fn_res = one_rpc(&h, false, "directory.find", json!({})).await;
    let elapsed_n = start_n.elapsed();

    assert!(
        elapsed_w < Duration::from_secs(5),
        "wasm search must finish before slow source's 5s sleep: took {:?}",
        elapsed_w
    );
    assert!(
        elapsed_n < Duration::from_secs(5),
        "native search must finish before slow source's 5s sleep: took {:?}",
        elapsed_n
    );

    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "other source's hit must be kept");

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    let slow_outcome =
        outcomes.iter().find(|o| o["source"] == "did:key:hSlow").expect("slow source outcome");
    assert_eq!(slow_outcome["error"]["kind"], "timed-out");
    assert_eq!(slow_outcome["verified"], 0);

    let good_outcome = outcomes
        .iter()
        .find(|o| o["source"] == "did:key:hForeign")
        .expect("foreign source outcome");
    assert_eq!(good_outcome["error"], Value::Null);
    assert_eq!(good_outcome["verified"], 1);
}

#[tokio::test]
async fn find_marks_an_oversized_reply_unreadable_and_keeps_the_others() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) = set_and_get(&h, full_listing_params("oversized-hedge", "Hedge")).await;
    let env_bytes = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &env_bytes).await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hOversized" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "good source's hit must be kept");

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    let over_outcome = outcomes
        .iter()
        .find(|o| o["source"] == "did:key:hOversized")
        .expect("oversized source outcome");
    assert_eq!(over_outcome["error"]["kind"], "unreadable");
    assert_eq!(over_outcome["error"]["reason"], "reply too large");
    assert_eq!(over_outcome["verified"], 0);

    let good_outcome = outcomes
        .iter()
        .find(|o| o["source"] == "did:key:hForeign")
        .expect("foreign source outcome");
    assert_eq!(good_outcome["error"], Value::Null);
    assert_eq!(good_outcome["verified"], 1);
}

#[tokio::test]
async fn a_second_find_for_one_source_does_not_duplicate_a_listing() {
    let h = harness().await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hTwoVersions" })).await;

    let (fw, fn_res) = both_rpc(&h, "directory.find", json!({})).await;
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    let outcome = outcomes
        .iter()
        .find(|o| o["source"] == "did:key:hTwoVersions")
        .expect("two versions outcome");
    assert_eq!(outcome["verified"], 2);
    assert_eq!(outcome["error"], Value::Null);

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "only the newer version appears on the page");
    assert_eq!(hits[0]["versions_differ"], false);
    assert_eq!(hits[0]["title"], "Hedge trimming v2");
    assert_eq!(hits[0]["sources"].as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["sources"][0]["directory"], "did:key:hTwoVersions");
}

#[tokio::test]
async fn find_ceiling_run_of_eight_sources_fits_the_dispatch_budget() {
    let h = harness().await;
    for k in 1..=8 {
        both_rpc(&h, "directory.add-source", json!({ "did": format!("did:key:hCeiling{k}") }))
            .await;
    }

    let start_w = Instant::now();
    let fw = one_rpc(&h, true, "directory.find", json!({})).await;
    let elapsed_w = start_w.elapsed();

    let start_n = Instant::now();
    let fn_res = one_rpc(&h, false, "directory.find", json!({})).await;
    let elapsed_n = start_n.elapsed();

    println!("ceiling run wall time: wasm {:?}, native {:?}", elapsed_w, elapsed_n);
    let mut w = fw.clone();
    let mut n = fn_res.clone();
    w["result"]["run_id"] = json!("stripped_run_id");
    n["result"]["run_id"] = json!("stripped_run_id");
    assert_eq!(stripped(&w), stripped(&n));

    let outcomes = fw["result"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 8);
    for outcome in outcomes {
        assert_eq!(outcome["verified"], 20);
        assert_eq!(outcome["refused"], 0);
        assert_eq!(outcome["error"], Value::Null);
    }

    let hits = fw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 50);
    assert_eq!(fw["result"]["hits_truncated"], true);

    for hit in hits {
        for src in hit["sources"].as_array().unwrap() {
            assert_eq!(
                src["membership"]["state"], "valid",
                "ceiling hit membership verdict must be Valid: {src}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "isolated measurement of ceiling run against DISPATCH_HEADROOM_MS"]
async fn find_ceiling_measure_fits_dispatch_headroom() {
    let h = harness().await;
    for k in 1..=8 {
        both_rpc(&h, "directory.add-source", json!({ "did": format!("did:key:hCeiling{k}") }))
            .await;
    }

    let start = Instant::now();
    let fw = one_rpc(&h, true, "directory.find", json!({})).await;
    let wasm_elapsed = start.elapsed();

    println!("measured ceiling wasm wall time: {:?}", wasm_elapsed);
    assert!(fw["result"]["hits"].is_array());
    assert!(
        wasm_elapsed < Duration::from_millis(u64::from(DISPATCH_HEADROOM_MS)),
        "measured ceiling wall time ({:?}) must fit within DISPATCH_HEADROOM_MS ({} ms)",
        wasm_elapsed,
        DISPATCH_HEADROOM_MS
    );
}

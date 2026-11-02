//! Parity integration tests for `directory.find` and `directory.run-envelope`
//! across WASM and native builds.

use serde_json::json;
use syneroym_data_db::host_store::RecordWriteValue;

use super::{fixtures::*, helpers::*, trust_fixtures::*};

#[tokio::test]
async fn find_gives_the_page_the_old_loop_gave() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    // Publish test listings across primary and dir2
    let _e_p = publish_listing_to_primary(&h, "eq-hedge", "Hedge").await;
    let _e_d2 = publish_listing_to_dir2(&h, "eq-hedge", "Hedge v2").await;
    let _good0 = publish_listing_to_primary(&h, "eq-good-0", "Good 0").await;
    let _good1 = publish_listing_to_primary(&h, "eq-good-1", "Good 1").await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire2" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForge1" })).await;
    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hTrunc" })).await;

    let sources =
        ["did:key:hForeignWire", "did:key:hForeignWire2", "did:key:hForge1", "did:key:hTrunc"];

    for wasm in [true, false] {
        let start = one_rpc(&h, wasm, "directory.start-run", json!({})).await;
        let old_run_id = start["result"]["run_id"].as_str().unwrap().to_string();
        let mut qs_reports = Vec::new();
        for did in sources {
            let qs = one_rpc(
                &h,
                wasm,
                "directory.query-source",
                json!({ "run_id": old_run_id, "source": did, "query": {} }),
            )
            .await;
            qs_reports.push(qs["result"].clone());
        }
        let old_merged =
            one_rpc(&h, wasm, "directory.merge", json!({ "run_id": old_run_id })).await;

        let find_res = one_rpc(&h, wasm, "directory.find", json!({})).await;

        let find_result = &find_res["result"];
        let merge_result = &old_merged["result"];

        let find_hits = find_result["hits"].as_array().unwrap();
        let old_hits = merge_result["hits"].as_array().unwrap();
        assert!(!find_hits.is_empty(), "find hits must not be empty");
        assert!(!old_hits.is_empty(), "old merge hits must not be empty");

        let find_refused = find_result["refused"].as_array().unwrap();
        let old_refused = merge_result["refused"].as_array().unwrap();
        assert!(!find_refused.is_empty(), "find refused must not be empty");
        assert!(!old_refused.is_empty(), "old merge refused must not be empty");

        assert_eq!(stripped(&find_result["hits"]), stripped(&merge_result["hits"]));
        assert_eq!(find_result["hits_truncated"], merge_result["hits_truncated"]);
        assert_eq!(stripped(&find_result["refused"]), stripped(&merge_result["refused"]));
        assert_eq!(find_result["refused_truncated"], merge_result["refused_truncated"]);

        let outcomes = find_result["outcomes"].as_array().unwrap();
        assert_eq!(outcomes.len(), sources.len());
        for (i, outcome) in outcomes.iter().enumerate() {
            let qs = &qs_reports[i];
            assert_eq!(outcome["source"], qs["source"]);
            assert_eq!(outcome["verified"], qs["verified"]);
            assert_eq!(outcome["refused"], qs["refused"]);
            assert_eq!(outcome["truncated"], qs["truncated"]);
            assert_eq!(stripped(&outcome["error"]), stripped(&qs["error"]));
        }
    }
}

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
    assert_eq!(stripped(&fw), stripped(&fn_res));

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
    assert_eq!(stripped(&fw), stripped(&fn_res));

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
    assert_eq!(stripped(&fw), stripped(&fn_res));

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
async fn find_refuses_an_invalid_query_field_with_the_field_name() {
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

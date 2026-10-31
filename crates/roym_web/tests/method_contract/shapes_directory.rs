use serde_json::{Value, json};

use super::{
    fixtures::{ensure_dir2_synorg, ensure_synorg, full_listing_params, peer_did},
    helpers::{Driver, Harness, HttpDriver, harness, owner_did},
    shapes::{assert_shape, enrol_native_signing, rpc},
    trust_fixtures::{FIXTURE_CATEGORIES, fixture_credential_expires_at_secs, stranger_did},
};

#[tokio::test]
async fn server_shapes() {
    let h = harness().await;

    let ping = rpc(&h, "directory.ping", json!({})).await;
    assert_shape("directory_ping", &ping);

    let set_missing = rpc(&h, "directory.settings", json!({})).await;
    assert_shape("directory_settings_missing", &set_missing);

    let info_none = rpc(&h, "directory.info", json!({})).await;
    assert_shape("directory_info_no_synorg", &info_none);

    let set_settings = rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "Guild",
            "rules": "r",
            "area": [],
            "categories": FIXTURE_CATEGORIES,
            "support_contact": "s@example.org",
            "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
    assert_shape("directory_set_settings", &set_settings);

    let set_found = rpc(&h, "directory.settings", json!({})).await;
    assert_shape("directory_settings_found", &set_found);

    let (unconf, inst, conf) = enrol_native_signing(&h, "directory").await;
    assert_shape("directory_signing_status_unconfigured", &unconf);
    assert_shape("directory_install_signing_certificate", &inst);
    assert_shape("directory_signing_status", &conf);

    let info_found = rpc(&h, "directory.info", json!({})).await;
    assert_shape("directory_info_with_synorg", &info_found);

    let mem_empty = rpc(&h, "member.list", json!({})).await;
    assert_shape("member_list_empty", &mem_empty);

    let rem_missing = rpc(&h, "member.remove", json!({ "did": stranger_did() })).await;
    assert_shape("member_remove_missing", &rem_missing);

    let mem_add = rpc(&h, "member.add", json!({ "did": stranger_did(), "note": "welcome" })).await;
    assert_shape("member_add", &mem_add);

    let mem_rows = rpc(&h, "member.list", json!({})).await;
    assert_shape("member_list_with_rows", &mem_rows);

    let rem_found = rpc(&h, "member.remove", json!({ "did": stranger_did() })).await;
    assert_shape("member_remove_found", &rem_found);

    let lim = rpc(&h, "directory.limits", json!({})).await;
    assert_shape("directory_limits", &lim);

    let set_lim =
        rpc(&h, "directory.set-limits", json!({ "window_secs": 3600, "max_per_window": 10 })).await;
    assert_shape("directory_set_limits", &set_lim);

    let pubs_empty = rpc(&h, "directory.publications", json!({})).await;
    assert_shape("directory_publications_empty", &pubs_empty);

    let search_no_hit = rpc(&h, "directory.search", json!({})).await;
    assert_shape("directory_search_no_hit", &search_no_hit);

    let _ = rpc(
        &h,
        "credential.issue",
        json!({
            "member_did": owner_did(),
            "categories": FIXTURE_CATEGORIES,
            "expires_at_secs": fixture_credential_expires_at_secs(),
        }),
    )
    .await;

    let _ = enrol_native_signing(&h, "catalog").await;
    let set_res =
        rpc(&h, "listing.set", full_listing_params("hedge-trimming-shape", "Hedge trimming")).await;
    let listing_id = set_res["listing_id"].as_str().unwrap().to_string();
    let get_res = rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    let envelope = get_res["envelope"].as_str().unwrap().to_string();

    let pub_pub = rpc(&h, "directory.publish", json!({ "envelope": envelope })).await;
    assert_shape("directory_publish_published", &pub_pub);

    let pubs_row = rpc(&h, "directory.publications", json!({})).await;
    assert_shape("directory_publications_with_row", &pubs_row);

    let search_with_hit = rpc(&h, "directory.search", json!({})).await;
    assert_shape("directory_search_with_hit", &search_with_hit);

    let reindex = rpc(&h, "directory.reindex", json!({})).await;
    assert_shape("directory_reindex", &reindex);

    let unpub = rpc(&h, "directory.unpublish", json!({ "listing_id": listing_id })).await;
    assert_shape("directory_unpublish", &unpub);

    rpc(&h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    let get_withdrawn = rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    let withdrawn_env = get_withdrawn["envelope"].as_str().unwrap().to_string();
    let pub_withdrawn = rpc(&h, "directory.publish", json!({ "envelope": withdrawn_env })).await;
    assert_shape("directory_publish_withdrawn", &pub_withdrawn);

    let export = rpc(&h, "directory.export", json!({})).await;
    assert_shape("directory_export", &export);

    let import = rpc(&h, "directory.import", json!({ "bundle": export })).await;
    assert_shape("directory_import", &import);
}

#[tokio::test]
async fn credential_shapes() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let cred_issue = rpc(
        &h,
        "credential.issue",
        json!({
            "member_did": stranger_did(),
            "categories": FIXTURE_CATEGORIES,
            "expires_at_secs": fixture_credential_expires_at_secs(),
            "area": [],
            "note": "welcomed member"
        }),
    )
    .await;
    assert_shape("credential_issue", &cred_issue);
    let cred_rec_id = cred_issue["record_id"].as_str().unwrap().to_string();

    let cred_all = rpc(&h, "credential.list", json!({})).await;
    assert_shape("credential_list_all", &cred_all);

    let cred_filt = rpc(&h, "credential.list", json!({ "member_did": stranger_did() })).await;
    assert_shape("credential_list_filtered", &cred_filt);

    let rev_new = rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": cred_rec_id, "reason": "policy violation" }),
    )
    .await;
    assert_shape("revocation_issue_new", &rev_new);

    let rev_rep = rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": cred_rec_id, "reason": "policy violation" }),
    )
    .await;
    assert_shape("revocation_issue_repeated", &rev_rep);

    let rev_list = rpc(&h, "revocation.list", json!({})).await;
    assert_shape("revocation_list", &rev_list);

    let susp_until = rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": owner_did(),
            "rule": "conduct",
            "reason": "investigation",
            "until_secs": 1_900_000_000u64
        }),
    )
    .await;
    assert_shape("member_suspend_with_until", &susp_until);

    let susp_indef = rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": peer_did(),
            "rule": "conduct",
            "reason": "indefinite",
            "scope": { "kind": "membership" }
        }),
    )
    .await;
    assert_shape("member_suspend_without_until", &susp_indef);
    let susp_rec_id = susp_indef["record_id"].as_str().unwrap().to_string();

    let lift = rpc(
        &h,
        "member.lift",
        json!({ "decision_record_id": susp_rec_id, "reason": "reinstated" }),
    )
    .await;
    assert_shape("member_lift", &lift);

    let decs = rpc(&h, "member.decisions", json!({})).await;
    assert_shape("member_decisions", &decs);
}

#[tokio::test]
async fn client_shapes() {
    let h = harness().await;
    ensure_synorg(&h).await;
    let _ = enrol_native_signing(&h, "catalog").await;
    let set_res =
        rpc(&h, "listing.set", full_listing_params("client-shape-listing", "Client shape")).await;
    let listing_id = set_res["listing_id"].as_str().unwrap().to_string();
    let get_res = rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    let envelope = get_res["envelope"].as_str().unwrap().to_string();
    let _ = rpc(&h, "directory.publish", json!({ "envelope": envelope })).await;

    let add_clean = rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeign" })).await;
    assert_shape("directory_add_source_without_note", &add_clean);

    ensure_dir2_synorg(&h).await;
    let add_note = rpc(
        &h,
        "directory.add-source",
        json!({ "did": "did:key:hForeignWire2", "issuer_did": owner_did() }),
    )
    .await;
    assert_shape("directory_add_source_with_note", &add_note);

    let probe = rpc(&h, "directory.probe-info", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_shape("directory_probe_info_with_synorg", &probe);

    let rem_found =
        rpc(&h, "directory.remove-source", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_shape("directory_remove_source_found", &rem_found);

    let rem_missing =
        rpc(&h, "directory.remove-source", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_shape("directory_remove_source_missing", &rem_missing);

    rpc(&h, "directory.add-source", json!({ "did": "did:key:zUnbound" })).await;
    rpc(&h, "directory.add-source", json!({ "did": "did:key:hTrunc" })).await;
    rpc(&h, "directory.add-source", json!({ "did": "did:key:hForge1" })).await;

    let sources = rpc(&h, "directory.sources", json!({})).await;
    assert_shape("directory_sources", &sources);

    let start = rpc(&h, "directory.start-run", json!({})).await;
    assert_shape("directory_start_run", &start);
    let run_id = start["run_id"].as_str().unwrap().to_string();

    let qs_hits = rpc(
        &h,
        "directory.query-source",
        json!({ "run_id": run_id, "source": "did:key:hForeign", "query": {} }),
    )
    .await;
    assert_shape("directory_query_source_with_hits", &qs_hits);

    let qs_err = rpc(
        &h,
        "directory.query-source",
        json!({ "run_id": run_id, "source": "did:key:zUnbound", "query": {} }),
    )
    .await;
    assert_shape("directory_query_source_error", &qs_err);

    let qs_trunc = rpc(
        &h,
        "directory.query-source",
        json!({ "run_id": run_id, "source": "did:key:hTrunc", "query": {} }),
    )
    .await;
    assert_shape("directory_query_source_htrunc", &qs_trunc);

    rpc(
        &h,
        "directory.query-source",
        json!({ "run_id": run_id, "source": "did:key:hForge1", "query": {} }),
    )
    .await;

    let merged = rpc(&h, "directory.merge", json!({ "run_id": run_id })).await;
    assert_shape("directory_merge", &merged);

    let rec_id = merged["hits"][0]["record_id"].as_str().unwrap();
    let env_found =
        rpc(&h, "directory.run-envelope", json!({ "run_id": run_id, "record_id": rec_id })).await;
    assert_shape("directory_run_envelope_found", &env_found);

    let env_missing =
        rpc(&h, "directory.run-envelope", json!({ "run_id": run_id, "record_id": "nonexistent" }))
            .await;
    assert_shape("directory_run_envelope_missing", &env_missing);

    let _ = rpc(
        &h,
        "credential.issue",
        json!({
            "member_did": "did:key:zForeignSearcher",
            "categories": FIXTURE_CATEGORIES,
            "expires_at_secs": fixture_credential_expires_at_secs(),
        }),
    )
    .await;
    let pub_src = rpc(
        &h,
        "directory.publish-to-source",
        json!({ "source": "did:key:hForeignWire", "listing_id": listing_id }),
    )
    .await;
    assert_shape("directory_publish_to_source_accepted", &pub_src);
}

#[tokio::test]
async fn standing_shapes() {
    let h = harness().await;
    ensure_synorg(&h).await;

    rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire" })).await;

    let standing = rpc(&h, "directory.standing", json!({ "member_did": owner_did() })).await;
    assert_shape("directory_standing", &standing);

    let cs_refreshed = rpc(
        &h,
        "directory.check-standing",
        json!({ "source": "did:key:hForeignWire", "member_did": owner_did() }),
    )
    .await;
    assert_shape("directory_check_standing_refreshed", &cs_refreshed);

    rpc(
        &h,
        "directory.add-source",
        json!({ "did": "did:key:hForeignWire", "issuer_did": stranger_did() }),
    )
    .await;
    let cs_issuer = rpc(
        &h,
        "directory.check-standing",
        json!({ "source": "did:key:hForeignWire", "member_did": owner_did() }),
    )
    .await;
    assert_shape("directory_check_standing_issuer_changed", &cs_issuer);

    rpc(&h, "directory.add-source", json!({ "did": "did:key:zUnbound" })).await;

    let cs_stale = rpc(
        &h,
        "directory.check-standing",
        json!({ "source": "did:key:zUnbound", "member_did": owner_did() }),
    )
    .await;
    assert_shape("directory_check_standing_stale", &cs_stale);

    let memberships = rpc(&h, "directory.memberships", json!({})).await;
    assert_shape("directory_memberships", &memberships);
}

#[tokio::test]
async fn whoami_shapes() {
    let h = harness().await;

    let whoami_del = rpc(&h, "session.whoami", json!({})).await;
    assert_shape("session_whoami_delegated", &whoami_del);

    let req = json!({ "method": "session.whoami", "params": {} }).to_string().into_bytes();
    let r = h.native_http.post("/rpc", req, None).await;
    let resp: Value = serde_json::from_slice(&r.body).unwrap();
    let whoami_anon = resp.get("result").cloned().unwrap_or(Value::Null);
    assert_shape("session_whoami_anonymous_rpc", &whoami_anon);

    let invoke_req = json!({ "method": "session.whoami", "params": {} }).to_string();
    let raw = h.native.invoke_web(&invoke_req).await.unwrap();
    let resp: Value = serde_json::from_str(&raw).unwrap();
    let whoami_invoke = resp.get("result").cloned().unwrap_or(Value::Null);
    assert_shape("session_whoami_invoke", &whoami_invoke);
}

#[tokio::test]
async fn no_synorg_shapes() {
    let h = harness().await;

    let probe = rpc(&h, "directory.probe-info", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_shape("directory_probe_info_no_synorg", &probe);

    let add_source =
        rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire2" })).await;
    assert_shape("directory_add_source_no_synorg", &add_source);
}

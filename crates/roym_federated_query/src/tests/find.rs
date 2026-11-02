use serde_json::json;
use syneroym_app_host::types::proxy::{CallTarget, CalleeError, ProxyError};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    directory::{SearchQuery, SourceError},
    services,
};

use super::helpers::{
    FakeProxy, build_max_evidence, canned_search_hit, canned_search_response, sign_test_listing,
    sign_test_listing_with_issuer,
};
use crate::{FindError, Source, find, gather::MAX_SEARCH_REPLY_BYTES};

#[tokio::test]
async fn each_call_is_built_like_the_old_query_source() {
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let query = SearchQuery { text: Some("plumber".to_string()), ..Default::default() };
    let resp = canned_search_response(Vec::new(), false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    find(&proxy, &[source], &query, 1000).await.unwrap();

    let calls = proxy.calls();
    assert_eq!(calls.len(), 1);
    let req = &calls[0];
    match &req.target {
        CallTarget::Service(did) => assert_eq!(did, "did:key:s1"),
        other => panic!("expected Service target, got {other:?}"),
    }
    assert_eq!(req.interface, services::DIRECTORY.interface);
    assert_eq!(req.method, "invoke");

    let params_val: serde_json::Value = serde_json::from_str(&req.params).unwrap();
    let inner_str = params_val[0].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(inner_str).unwrap();
    assert_eq!(inner["method"], "directory.search");
    assert_eq!(inner["params"]["text"], "plumber");

    let opts = req.options.as_ref().unwrap();
    assert!(opts.idempotent);
    assert_eq!(opts.timeout_ms, Some(2000));
}

#[tokio::test]
async fn results_follow_the_source_order() {
    let s1 = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let s2 = Source { did: "did:key:s2".to_string(), pinned_issuer: None };
    let resp1 = canned_search_response(Vec::new(), false);
    let resp2 = canned_search_response(Vec::new(), false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp1), Ok(resp2)]));

    let res = find(&proxy, &[s1, s2], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes.len(), 2);
    assert_eq!(res.page.outcomes[0].source, "did:key:s1");
    assert_eq!(res.page.outcomes[1].source, "did:key:s2");
}

#[tokio::test]
async fn a_missing_service_is_not_found() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let proxy = FakeProxy::new(Ok(vec![Err(ProxyError::ServiceNotFound("s1".to_string()))]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].error, Some(SourceError::NotFound));
}

#[tokio::test]
async fn a_slow_call_is_timed_out() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let proxy = FakeProxy::new(Ok(vec![Err(ProxyError::TimedOut)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].error, Some(SourceError::TimedOut));
}

#[tokio::test]
async fn a_callee_error_is_refused_with_its_code() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let proxy = FakeProxy::new(Ok(vec![Err(ProxyError::Callee(CalleeError {
        code: -32602,
        message: "bad query".to_string(),
        data: None,
    }))]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(
        res.page.outcomes[0].error,
        Some(SourceError::Refused { code: -32602, message: "bad query".to_string() })
    );
}

#[tokio::test]
async fn another_proxy_error_is_refused_32603() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let proxy = FakeProxy::new(Ok(vec![Err(ProxyError::Internal("crash".to_string()))]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    match &res.page.outcomes[0].error {
        Some(SourceError::Refused { code, .. }) => assert_eq!(*code, -32603),
        other => panic!("expected Refused with code -32603, got {other:?}"),
    }
}

#[tokio::test]
async fn a_reply_that_is_not_json_rpc_is_unreadable() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let proxy = FakeProxy::new(Ok(vec![Ok("plain html 404".to_string())]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    match &res.page.outcomes[0].error {
        Some(SourceError::Unreadable { .. }) => {}
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[tokio::test]
async fn a_json_rpc_error_is_refused() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let error_body = json!({
        "jsonrpc": "2.0",
        "error": { "code": -32000, "message": "denied by policy" },
        "id": null
    })
    .to_string();
    let proxy = FakeProxy::new(Ok(vec![Ok(error_body)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(
        res.page.outcomes[0].error,
        Some(SourceError::Refused { code: -32000, message: "denied by policy".to_string() })
    );
}

#[tokio::test]
async fn hits_that_do_not_parse_are_unreadable() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let bad_body = json!({
        "jsonrpc": "2.0",
        "result": { "hits": "not an array", "truncated": false },
        "id": null
    })
    .to_string();
    let proxy = FakeProxy::new(Ok(vec![Ok(bad_body)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    match &res.page.outcomes[0].error {
        Some(SourceError::Unreadable { .. }) => {}
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[tokio::test]
async fn a_reply_over_the_size_cap_is_unreadable_and_does_not_stop_the_others() {
    let s_huge = Source { did: "did:key:sHuge".to_string(), pinned_issuer: None };
    let s_good = Source { did: "did:key:sGood".to_string(), pinned_issuer: None };

    let huge_text = "x".repeat(MAX_SEARCH_REPLY_BYTES + 1024);

    let (listing_id, record_id, envelope) = sign_test_listing("slug-good", "Good", 1000);
    let hit = canned_search_hit(&listing_id, &record_id, &envelope, Default::default(), 1000);
    let good_text = canned_search_response(vec![hit], false);

    let proxy = FakeProxy::new(Ok(vec![Ok(huge_text), Ok(good_text)]));

    let res = find(&proxy, &[s_huge, s_good], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes.len(), 2);
    assert_eq!(
        res.page.outcomes[0].error,
        Some(SourceError::Unreadable { reason: "reply too large".to_string() })
    );
    assert_eq!(res.page.outcomes[1].error, None);
    assert_eq!(res.page.outcomes[1].verified, 1);
    assert_eq!(res.page.hits.len(), 1);
}

#[tokio::test]
async fn fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read() {
    let issuer_key = Identity::generate().unwrap();
    let issuer_did = substrate::derive_did_key(&issuer_key.public_key());

    let mut hits = Vec::new();
    for i in 0..50 {
        let (listing_id, record_id, envelope) = sign_test_listing_with_issuer(
            &issuer_key,
            &issuer_did,
            &format!("max-slug-{i}"),
            &format!("Max Listing {i}"),
            1000,
        );
        let evidence = build_max_evidence(&issuer_key, &issuer_did, &issuer_did, &listing_id, 1000);
        hits.push(canned_search_hit(&listing_id, &record_id, &envelope, evidence, 1000));
    }

    let resp_text = canned_search_response(hits, true);
    assert!(resp_text.len() <= MAX_SEARCH_REPLY_BYTES, "response size {} > 8 MiB", resp_text.len());

    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: Some(issuer_did) };
    let proxy = FakeProxy::new(Ok(vec![Ok(resp_text)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].error, None);
    assert_eq!(res.page.outcomes[0].verified, 20);
    assert!(res.page.outcomes[0].truncated);
}

#[tokio::test]
async fn a_truncated_flag_that_is_not_a_boolean_is_unreadable() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let bad_trunc = json!({
        "jsonrpc": "2.0",
        "result": { "hits": [], "truncated": "yes" },
        "id": null
    })
    .to_string();
    let proxy = FakeProxy::new(Ok(vec![Ok(bad_trunc)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    match &res.page.outcomes[0].error {
        Some(SourceError::Unreadable { .. }) => {}
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[tokio::test]
async fn the_result_truncated_flag_is_reported_per_source() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let (l, r, env) = sign_test_listing("slug", "Title", 1000);
    let hit = canned_search_hit(&l, &r, &env, Default::default(), 1000);
    let resp = canned_search_response(vec![hit], true);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert!(res.page.outcomes[0].truncated);
}

#[tokio::test]
async fn a_source_with_no_hits_still_reports_truncated() {
    let s = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(Vec::new(), true);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[s], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].verified, 0);
    assert!(res.page.outcomes[0].truncated);
}

#[tokio::test]
async fn a_refused_batch_marks_every_source_not_started() {
    let s1 = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let s2 = Source { did: "did:key:s2".to_string(), pinned_issuer: None };

    // Case 1: Internal error (e.g. call-all busy)
    let proxy1 = FakeProxy::new(Err(ProxyError::Internal("call-all busy".to_string())));
    let res1 =
        find(&proxy1, &[s1.clone(), s2.clone()], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res1.page.outcomes[0].error, Some(SourceError::NotStarted));
    assert_eq!(res1.page.outcomes[1].error, Some(SourceError::NotStarted));

    // Case 2: UnsupportedTarget error (e.g. too many calls)
    let proxy2 = FakeProxy::new(Err(ProxyError::UnsupportedTarget("too many calls".to_string())));
    let res2 = find(&proxy2, &[s1, s2], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res2.page.outcomes[0].error, Some(SourceError::NotStarted));
    assert_eq!(res2.page.outcomes[1].error, Some(SourceError::NotStarted));
}

#[tokio::test]
async fn no_sources_makes_no_call() {
    let proxy = FakeProxy::new(Ok(Vec::new()));
    let res = find(&proxy, &[], &SearchQuery::default(), 1000).await.unwrap();
    assert!(res.page.outcomes.is_empty());
    assert!(res.page.hits.is_empty());
    assert!(proxy.calls().is_empty());
}

#[tokio::test]
async fn more_than_max_sources_is_an_error_and_no_call_runs() {
    let sources: Vec<Source> =
        (0..9).map(|i| Source { did: format!("did:key:s{i}"), pinned_issuer: None }).collect();
    let proxy = FakeProxy::new(Ok(Vec::new()));

    let err = find(&proxy, &sources, &SearchQuery::default(), 1000).await.unwrap_err();
    match err {
        FindError::TooManySources { count } => assert_eq!(count, 9),
    }
    assert!(proxy.calls().is_empty());
}

use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    directory::{AreaMatch, SearchHit, SearchQuery},
    membership::{
        MEMBERSHIP_CREDENTIAL_VERSION, MembershipCredentialPayload, MembershipEvidence,
        MembershipScope, MembershipVerdict,
    },
    record::RECORD_MEMBERSHIP_CREDENTIAL,
};
use syneroym_signed_record::RecordDraft;

use super::helpers::{
    FakeProxy, canned_search_hit, canned_search_response, sign_record, sign_test_listing,
    sign_test_listing_with_issuer, sign_test_listing_with_translation,
};
use crate::{Source, find};

#[tokio::test]
async fn at_most_ten_hits_are_read_before_checking() {
    let mut hits = Vec::new();
    for i in 0..25 {
        let (listing_id, record_id, envelope) =
            sign_test_listing(&format!("slug-{i}"), &format!("Title {i}"), 1000);
        hits.push(canned_search_hit(&listing_id, &record_id, &envelope, Default::default(), 1000));
    }

    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(hits, false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    // 10 hits are checked and kept (cap).
    assert_eq!(res.page.outcomes[0].verified, 10);
    // Page has 10 (merge per-source cap)
    assert_eq!(res.page.hits.len(), 10);
}

#[tokio::test]
async fn a_tampered_envelope_is_refused_evidence_not_a_hit() {
    let (listing_id, record_id, envelope) = sign_test_listing("slug-tamper", "Title", 1000);
    let mut val: serde_json::Value = serde_json::from_str(&envelope).unwrap();
    val["payload"]["title"] = serde_json::json!("Tampered Title");
    let tampered_env = val.to_string();

    let hit = canned_search_hit(&listing_id, &record_id, &tampered_env, Default::default(), 1000);
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(vec![hit], false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].verified, 0);
    assert_eq!(res.page.outcomes[0].refused, 1);
    assert!(res.page.hits.is_empty());
    assert_eq!(res.page.refused.len(), 1);
}

#[tokio::test]
async fn a_source_is_capped_at_ten_verified_hits() {
    let mut hits = Vec::new();
    for i in 0..15 {
        let (listing_id, record_id, envelope) =
            sign_test_listing(&format!("v-slug-{i}"), &format!("Verified {i}"), 1000);
        hits.push(canned_search_hit(&listing_id, &record_id, &envelope, Default::default(), 1000));
    }

    // 15 total hits in response: truncated to MAX_CHECKED_PER_SOURCE (10) before
    // checking.
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(hits, false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].verified, 10);
    assert_eq!(res.page.outcomes[0].refused, 0);
}

#[tokio::test]
async fn refused_hits_coming_first_are_recorded_up_to_the_cap() {
    let mut hits = Vec::new();
    for i in 0..5 {
        let (listing_id, record_id, envelope) =
            sign_test_listing(&format!("r-slug-{i}"), &format!("Refused {i}"), 1000);
        let mut val: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        val["payload"]["title"] = serde_json::json!("Tampered");
        hits.push(canned_search_hit(
            &listing_id,
            &record_id,
            &val.to_string(),
            Default::default(),
            1000,
        ));
    }
    for i in 0..10 {
        let (listing_id, record_id, envelope) =
            sign_test_listing(&format!("v-slug-{i}"), &format!("Verified {i}"), 1000);
        hits.push(canned_search_hit(&listing_id, &record_id, &envelope, Default::default(), 1000));
    }

    // 15 total hits: 5 refused + 10 verified. Truncated to 10 before checking
    // -> 5 refused and 5 verified checked.
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(hits, false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.outcomes[0].refused, 5);
    assert_eq!(res.page.outcomes[0].verified, 5);
    assert_eq!(res.page.refused.len(), 5);
    assert_eq!(res.page.hits.len(), 5);
}

#[tokio::test]
async fn membership_uses_the_pinned_issuer_of_that_source() {
    let synorg_key = Identity::generate().unwrap();
    let synorg_did = substrate::derive_did_key(&synorg_key.public_key());

    let member_key = Identity::generate().unwrap();
    let member_did = substrate::derive_did_key(&member_key.public_key());

    let (listing_id, record_id, envelope) = sign_test_listing_with_issuer(
        &member_key,
        &member_did,
        "guild-service",
        "Guild Service",
        1000,
    );

    let cred_payload = MembershipCredentialPayload {
        synorg_name: "Guild".to_string(),
        member_did: member_did.clone(),
        scope: MembershipScope { categories: vec!["services".to_string()], area: vec![] },
    };
    let draft = RecordDraft {
        version: MEMBERSHIP_CREDENTIAL_VERSION,
        record_type: RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
        subject: member_did.clone(),
        payload: serde_json::to_value(&cred_payload).unwrap(),
        expires_at_secs: Some(2000),
        supersedes: None,
    };
    let cred_env = sign_record(&synorg_key, &synorg_did, draft, 1000);
    let evidence = MembershipEvidence {
        credentials: vec![cred_env],
        revocations: Vec::new(),
        decisions: Vec::new(),
    };

    let hit = canned_search_hit(&listing_id, &record_id, &envelope, evidence, 1000);

    // Case 1: Pinned issuer matches
    let source_matched =
        Source { did: "did:key:dir1".to_string(), pinned_issuer: Some(synorg_did.clone()) };
    let proxy1 = FakeProxy::new(Ok(vec![Ok(canned_search_response(vec![hit.clone()], false))]));
    let res1 = find(&proxy1, &[source_matched], &SearchQuery::default(), 1000).await.unwrap();
    assert!(
        matches!(res1.page.hits[0].sources[0].membership, Some(MembershipVerdict::Valid { .. })),
        "expected Valid verdict, got {:?}",
        res1.page.hits[0].sources[0].membership
    );

    // Case 2: Pinned issuer does not match
    let source_other = Source {
        did: "did:key:dir1".to_string(),
        pinned_issuer: Some("did:key:other_synorg".to_string()),
    };
    let proxy2 = FakeProxy::new(Ok(vec![Ok(canned_search_response(vec![hit], false))]));
    let res2 = find(&proxy2, &[source_other], &SearchQuery::default(), 1000).await.unwrap();
    assert!(
        matches!(res2.page.hits[0].sources[0].membership, Some(MembershipVerdict::Refused { .. })),
        "expected Refused verdict when pinned issuer does not match, got {:?}",
        res2.page.hits[0].sources[0].membership
    );
}

#[tokio::test]
async fn evidence_is_kept_for_every_counted_hit_in_order() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    let (l1, r1, env1) = sign_test_listing_with_issuer(&key, &issuer, "slug-1", "Listing 1", 1000);
    let (l2, r2, env2) = sign_test_listing_with_issuer(&key, &issuer, "slug-2", "Listing 2", 1000);

    let ev1 = MembershipEvidence {
        credentials: vec!["cred_1".to_string()],
        revocations: Vec::new(),
        decisions: Vec::new(),
    };
    let ev2 = MembershipEvidence {
        credentials: vec!["cred_2".to_string()],
        revocations: Vec::new(),
        decisions: Vec::new(),
    };

    let hit1 = canned_search_hit(&l1, &r1, &env1, ev1, 1000);
    let hit2 = canned_search_hit(&l2, &r2, &env2, ev2, 1000);

    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(vec![hit1, hit2], false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.evidence.len(), 2);
    assert_eq!(res.evidence[0].evidence.credentials, vec!["cred_1"]);
    assert_eq!(res.evidence[1].evidence.credentials, vec!["cred_2"]);
}

#[tokio::test]
async fn evidence_of_a_hit_over_the_cap_is_not_kept() {
    let mut hits = Vec::new();
    for i in 0..25 {
        let (listing_id, record_id, envelope) =
            sign_test_listing(&format!("slug-{i}"), &format!("Title {i}"), 1000);
        let ev = MembershipEvidence {
            credentials: vec![format!("cred_{i}")],
            revocations: Vec::new(),
            decisions: Vec::new(),
        };
        hits.push(canned_search_hit(&listing_id, &record_id, &envelope, ev, 1000));
    }

    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(hits, false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    // Only 10 verified hits are kept under cap
    assert_eq!(res.page.outcomes[0].verified, 10);
    // Evidence is collected only for counted hits under the cap
    assert_eq!(res.evidence.len(), 10);
}

#[tokio::test]
async fn page_envelopes_cover_every_source_of_every_hit_once() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    let (listing_id, record_id_a, env_a) =
        sign_test_listing_with_issuer(&key, &issuer, "shared-slug", "Version A", 1000);
    let (_, record_id_b, env_b) =
        sign_test_listing_with_issuer(&key, &issuer, "shared-slug", "Version B", 2000);

    let hit_a = SearchHit {
        listing_id: listing_id.clone(),
        record_id: record_id_a.clone(),
        envelope: env_a.clone(),
        issued_at_secs: 1000,
        received_at_secs: 1000,
        area_match: AreaMatch::NotQueried,
        membership: Default::default(),
    };
    let hit_b = SearchHit {
        listing_id,
        record_id: record_id_b.clone(),
        envelope: env_b.clone(),
        issued_at_secs: 2000,
        received_at_secs: 2000,
        area_match: AreaMatch::NotQueried,
        membership: Default::default(),
    };

    let s_a = Source { did: "did:key:sA".to_string(), pinned_issuer: None };
    let s_b = Source { did: "did:key:sB".to_string(), pinned_issuer: None };

    let resp_a = canned_search_response(vec![hit_a], false);
    let resp_b = canned_search_response(vec![hit_b], false);

    let proxy = FakeProxy::new(Ok(vec![Ok(resp_a), Ok(resp_b)]));

    let res = find(&proxy, &[s_a, s_b], &SearchQuery::default(), 3000).await.unwrap();
    assert_eq!(res.page.hits.len(), 1);
    assert!(res.page.hits[0].versions_differ);
    assert_eq!(res.page.hits[0].sources.len(), 2);

    // page_envelopes covers both versions
    assert_eq!(res.page_envelopes.len(), 2);
    let env_records: Vec<&str> = res.page_envelopes.iter().map(|(r, _)| r.as_str()).collect();
    assert!(env_records.contains(&record_id_a.as_str()));
    assert!(env_records.contains(&record_id_b.as_str()));
}

#[tokio::test]
async fn verified_hit_preserves_lang_and_translations() {
    let (listing_id, record_id, envelope) =
        sign_test_listing_with_translation("slug-lang", "Title", "hi", "शीर्षक", 1000);
    let hit = canned_search_hit(&listing_id, &record_id, &envelope, Default::default(), 1000);
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let resp = canned_search_response(vec![hit], false);
    let proxy = FakeProxy::new(Ok(vec![Ok(resp)]));

    let res = find(&proxy, &[source], &SearchQuery::default(), 1000).await.unwrap();
    assert_eq!(res.page.hits.len(), 1);
    assert_eq!(res.page.hits[0].lang, "en");
    assert_eq!(res.page.hits[0].translations.len(), 1);
    assert_eq!(res.page.hits[0].translations[0].lang.as_str(), "hi");
    assert_eq!(res.page.hits[0].translations[0].title, "शीर्षक");
}

#[test]
fn refused_row_has_empty_lang_and_translations() {
    let source = Source { did: "did:key:s1".to_string(), pinned_issuer: None };
    let hit = SearchHit {
        listing_id: "l1".to_string(),
        record_id: "r1".to_string(),
        envelope: "invalid-envelope".to_string(),
        membership: Default::default(),
        issued_at_secs: 1000,
        received_at_secs: 1000,
        area_match: AreaMatch::NotQueried,
    };
    let verdict = syneroym_roym_core::listing::verify_envelope(&hit.envelope, 1000);
    assert!(!verdict.verified);
    let row = crate::verify::build_refused_row(&source, hit, verdict);
    assert_eq!(row.lang, "");
    assert!(row.translations.is_empty());
}

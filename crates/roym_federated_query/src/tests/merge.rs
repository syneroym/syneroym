use crate::merge::{Row, merge_rows};

fn test_row(source: &str, listing_id: &str, record_id: &str, issued_at: u64) -> Row {
    Row {
        listing_id: listing_id.to_string(),
        record_id: record_id.to_string(),
        source: source.to_string(),
        issuer: "did:key:issuer".to_string(),
        title: format!("Title {listing_id}"),
        summary: format!("Summary {listing_id}"),
        categories: vec!["services".to_string()],
        conversation_address: "did:key:conv".to_string(),
        status: "active".to_string(),
        reason: None,
        revocation_status: "valid".to_string(),
        membership: None,
        issued_at_secs: issued_at,
        received_at_secs: issued_at,
        ..Default::default()
    }
}

fn test_refused_row(source: &str, listing_id: &str, record_id: &str, reason: &str) -> Row {
    Row {
        listing_id: listing_id.to_string(),
        record_id: record_id.to_string(),
        source: source.to_string(),
        issuer: "did:key:issuer".to_string(),
        reason: Some(reason.to_string()),
        ..Default::default()
    }
}

#[test]
fn newer_version_wins_inside_one_source() {
    let row1 = test_row("did:key:s1", "listing_1", "rec_v1", 100);
    let row2 = test_row("did:key:s1", "listing_1", "rec_v2", 200);

    let res = merge_rows(vec![row1, row2], Vec::new(), 300);
    assert_eq!(res.hits.len(), 1);
    assert_eq!(res.hits[0].record_id, "rec_v2");
    assert_eq!(res.hits[0].age_secs, 100);
    assert!(!res.hits[0].versions_differ);
}

#[test]
fn one_source_gets_at_most_its_share_of_the_page() {
    let rows: Vec<Row> = (0..15)
        .map(|i| test_row("did:key:s1", &format!("l_{i}"), &format!("r_{i}"), 100))
        .collect();

    let res = merge_rows(rows, Vec::new(), 200);
    assert_eq!(res.hits.len(), 10);
    assert!(!res.hits_truncated);
}

#[test]
fn sources_take_turns_in_did_order_not_listing_id_order() {
    let a1 = test_row("did:key:sA", "L2", "r_a1", 100);
    let a2 = test_row("did:key:sA", "L4", "r_a2", 90);
    let b1 = test_row("did:key:sB", "L1", "r_b1", 100);
    let b2 = test_row("did:key:sB", "L3", "r_b2", 90);

    let res = merge_rows(vec![a1, a2, b1, b2], Vec::new(), 200);
    let ids: Vec<&str> = res.hits.iter().map(|h| h.listing_id.as_str()).collect();
    assert_eq!(ids, vec!["L2", "L1", "L4", "L3"]);
}

#[test]
fn the_page_is_cut_at_the_total_cap_and_says_so() {
    let mut rows = Vec::new();
    for s in 0..6 {
        for l in 0..10 {
            rows.push(test_row(
                &format!("did:key:s{s}"),
                &format!("listing_{s}_{l}"),
                &format!("rec_{s}_{l}"),
                100,
            ));
        }
    }

    let res = merge_rows(rows, Vec::new(), 200);
    assert_eq!(res.hits.len(), 50);
    assert!(res.hits_truncated);
}

#[test]
fn the_same_listing_from_two_sources_is_one_hit_with_two_sources() {
    let a = test_row("did:key:sA", "listing_shared", "rec_a", 100);
    let b = test_row("did:key:sB", "listing_shared", "rec_b", 200);

    let res = merge_rows(vec![a, b], Vec::new(), 300);
    assert_eq!(res.hits.len(), 1);
    assert_eq!(res.hits[0].record_id, "rec_b");
    assert_eq!(res.hits[0].sources.len(), 2);
    let source_dirs: Vec<&str> = res.hits[0].sources.iter().map(|s| s.directory.as_str()).collect();
    assert!(source_dirs.contains(&"did:key:sA"));
    assert!(source_dirs.contains(&"did:key:sB"));
}

#[test]
fn versions_differ_is_true_only_for_different_record_ids() {
    let a1 = test_row("did:key:sA", "l_same", "rec_same", 100);
    let b1 = test_row("did:key:sB", "l_same", "rec_same", 100);
    let res1 = merge_rows(vec![a1, b1], Vec::new(), 200);
    assert!(!res1.hits[0].versions_differ);

    let a2 = test_row("did:key:sA", "l_diff", "rec_diff_a", 100);
    let b2 = test_row("did:key:sB", "l_diff", "rec_diff_b", 100);
    let res2 = merge_rows(vec![a2, b2], Vec::new(), 200);
    assert!(res2.hits[0].versions_differ);
}

#[test]
fn refused_hits_are_capped_apart_and_cannot_crowd_out_a_hit() {
    let mut verified = Vec::new();
    for i in 0..5 {
        verified.push(test_row("did:key:sA", &format!("v_{i}"), &format!("rec_{i}"), 100));
    }

    let mut refused = Vec::new();
    for s in 0..3 {
        for i in 0..10 {
            refused.push(test_refused_row(
                &format!("did:key:s{s}"),
                &format!("r_{s}_{i}"),
                &format!("rec_{s}_{i}"),
                "bad signature",
            ));
        }
    }

    let res = merge_rows(verified, refused, 200);
    assert_eq!(res.hits.len(), 5);
    assert!(!res.hits_truncated);
    assert_eq!(res.refused.len(), 20);
    assert!(res.refused_truncated);
}

#[test]
fn a_forged_listing_id_that_sorts_first_cannot_fill_the_refused_page() {
    let mut refused = Vec::new();
    for i in 0..25 {
        refused.push(test_refused_row(
            "did:key:sHostile",
            &format!("000_forged_{i}"),
            &format!("rec_{i}"),
            "fake",
        ));
    }
    refused.push(test_refused_row("did:key:sHonest", "zzz_honest", "rec_honest", "expired"));

    let res = merge_rows(Vec::new(), refused, 200);
    assert_eq!(res.refused.len(), 20);
    assert!(res.refused.iter().any(|r| r.listing_id == "zzz_honest"));
}

#[test]
fn an_empty_input_gives_an_empty_page() {
    let res = merge_rows(Vec::new(), Vec::new(), 100);
    assert!(res.hits.is_empty());
    assert!(!res.hits_truncated);
    assert!(res.refused.is_empty());
    assert!(!res.refused_truncated);
}

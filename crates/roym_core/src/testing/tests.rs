use serde_json::json;
use syneroym_app_host::{
    AppDataLayer,
    types::data_layer::{QueryOptions, RecordWriteValue},
};

use super::{TestHost, data_layer::matches_filter};

#[test]
fn matches_filter_supports_gte_and_lte() {
    let payload = serde_json::to_vec(&json!({
        "recorded_at_secs": 100,
        "name": "alice"
    }))
    .unwrap();

    let f_gte_match = json!({"recorded_at_secs": {"$gte": 100}});
    assert!(matches_filter(&payload, &f_gte_match));

    let f_gte_match_lower = json!({"recorded_at_secs": {"$gte": 99}});
    assert!(matches_filter(&payload, &f_gte_match_lower));

    let f_gte_no_match = json!({"recorded_at_secs": {"$gte": 101}});
    assert!(!matches_filter(&payload, &f_gte_no_match));

    let f_lte_match = json!({"recorded_at_secs": {"$lte": 100}});
    assert!(matches_filter(&payload, &f_lte_match));

    let f_lte_match_higher = json!({"recorded_at_secs": {"$lte": 101}});
    assert!(matches_filter(&payload, &f_lte_match_higher));

    let f_lte_no_match = json!({"recorded_at_secs": {"$lte": 99}});
    assert!(!matches_filter(&payload, &f_lte_no_match));

    let f_range = json!({
        "recorded_at_secs": {
            "$gte": 100,
            "$lte": 100
        }
    });
    assert!(matches_filter(&payload, &f_range));
}

#[tokio::test]
async fn test_host_records_reads_for_get_and_query() {
    let host = TestHost::default();
    host.put(
        "records".to_string(),
        RecordWriteValue {
            id: "rec_1".to_string(),
            payload: serde_json::to_vec(&json!({"foo": "bar"})).unwrap(),
        },
    )
    .await
    .unwrap();

    let got = host.get("records".to_string(), "rec_1".to_string()).await.unwrap();
    assert!(got.is_some());

    let q = host
        .query(
            "records".to_string(),
            QueryOptions {
                filter: Some(json!({"foo": "bar"}).to_string()),
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(q.records.len(), 1);

    let reads = host.reads();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0], ("records".to_string(), "rec_1".to_string()));
    assert_eq!(reads[1], ("records".to_string(), json!({"foo": "bar"}).to_string()));
}

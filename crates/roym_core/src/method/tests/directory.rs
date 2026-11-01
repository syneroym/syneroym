use std::{fs, panic};

use serde_json::json;

use crate::{
    directory::SearchQuery,
    method::{
        MethodInfo, NoParams, SiblingError, WireNoParams,
        check::assert_service_table,
        directory::{DirectoryInfoOutput, PublishOutput, SearchOutput, StandingOutput},
        parse_value, read_reply,
    },
    services::DIRECTORY,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct DummyOutput {}

#[test]
fn read_reply_reads_a_null_result_as_none_and_keeps_error_data() {
    let null_raw = r#"{"jsonrpc":"2.0","result":null,"id":1}"#;
    let res: Result<Option<String>, SiblingError> = read_reply(null_raw);
    assert_eq!(res.unwrap(), None);

    let err_raw = r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"denied","data":{"reason":"not-admitted"}},"id":1}"#;
    let err_res: Result<Option<String>, SiblingError> = read_reply(err_raw);
    match err_res {
        Err(SiblingError::Refused(rpc_err)) => {
            assert_eq!(rpc_err.code, -32001);
            assert_eq!(rpc_err.message, "denied");
            assert_eq!(rpc_err.data.unwrap()["reason"], "not-admitted");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}

#[test]
fn check_allows_req_method_only_in_the_admit_call() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("directory.ping")];

    // Valid case: exactly one req.method on admit::admit line and one on
    // method_not_found line
    let temp_ok = tempfile::tempdir().unwrap();
    fs::write(
        temp_ok.path().join("app.rs"),
        "if let Err(r) = admit::admit(host, api::WIRE_REACHABLE, &req.method).await { return r; \
         }\nResponse::method_not_found(&req.method);\n",
    )
    .unwrap();
    assert_service_table(DIRECTORY, &rows, &[], temp_ok.path());

    // Failing case 1: admit::admit call plus another req.method on the same line
    let temp_extra = tempfile::tempdir().unwrap();
    fs::write(
        temp_extra.path().join("app.rs"),
        "admit::admit(host, api::WIRE_REACHABLE, &req.method); match req.method.as_str() \
         {}\nResponse::method_not_found(&req.method);\n",
    )
    .unwrap();
    let res_extra = panic::catch_unwind(|| {
        assert_service_table(DIRECTORY, &rows, &[], temp_extra.path());
    });
    let err_extra = res_extra.unwrap_err();
    let msg_extra = err_extra.downcast_ref::<String>().unwrap();
    assert!(msg_extra.contains("req.method used outside"), "got: {msg_extra}");

    // Failing case 2: admit::admit with a different list name
    let temp_other = tempfile::tempdir().unwrap();
    fs::write(
        temp_other.path().join("app.rs"),
        "if let Err(r) = admit::admit(host, api::OTHER_LIST, &req.method).await { return r; \
         }\nResponse::method_not_found(&req.method);\n",
    )
    .unwrap();
    let res_other = panic::catch_unwind(|| {
        assert_service_table(DIRECTORY, &rows, &[], temp_other.path());
    });
    let err_other = res_other.unwrap_err();
    let msg_other = err_other.downcast_ref::<String>().unwrap();
    assert!(msg_other.contains("req.method used outside"), "got: {msg_other}");
}

#[test]
fn wire_no_params_ignores_an_unknown_key() {
    let res: Result<WireNoParams, _> = parse_value(&json!({ "unknown_field": 123 }));
    assert!(res.is_ok());
}

#[test]
fn search_query_ignores_an_unknown_key_and_refuses_an_unknown_open_to_word() {
    let ok: Result<SearchQuery, _> =
        parse_value(&json!({ "extra_key": "ignored", "text": "garden" }));
    assert!(ok.is_ok());
    assert_eq!(ok.unwrap().text.as_deref(), Some("garden"));

    let err: Result<SearchQuery, _> = parse_value(&json!({ "open_to": "everyone" }));
    assert!(err.is_err());
    assert_eq!(err.unwrap_err().field.as_deref(), Some("open_to"));
}

#[test]
fn publish_output_writes_both_shapes_and_reads_them_back() {
    let p_published = PublishOutput {
        listing_id: "listing-1".to_string(),
        record_id: Some("record-1".to_string()),
        withdrawn: None,
    };
    let val_pub = serde_json::to_value(&p_published).unwrap();
    assert_eq!(val_pub["listing_id"], "listing-1");
    assert_eq!(val_pub["record_id"], "record-1");
    assert!(val_pub.get("withdrawn").is_none());
    let read_pub: PublishOutput = serde_json::from_value(val_pub).unwrap();
    assert_eq!(read_pub, p_published);

    let p_withdrawn = PublishOutput {
        listing_id: "listing-2".to_string(),
        record_id: None,
        withdrawn: Some(true),
    };
    let val_withdrawn = serde_json::to_value(&p_withdrawn).unwrap();
    assert_eq!(val_withdrawn["listing_id"], "listing-2");
    assert!(val_withdrawn.get("record_id").is_none());
    assert_eq!(val_withdrawn["withdrawn"], true);
    let read_withdrawn: PublishOutput = serde_json::from_value(val_withdrawn).unwrap();
    assert_eq!(read_withdrawn, p_withdrawn);
}

#[test]
fn search_output_reads_an_answer_with_only_hits_and_truncated() {
    let json_trunc = json!({
        "hits": [],
        "truncated": true,
        "zz": 1
    });
    let out: SearchOutput = serde_json::from_value(json_trunc).unwrap();
    assert_eq!(out.hits.len(), 0);
    assert!(out.truncated);
    assert_eq!(out.directory, "");
    assert_eq!(out.answered_at_secs, 0);
}

#[test]
fn standing_output_reads_an_answer_with_an_extra_key() {
    let val = json!({
        "member_did": "did:key:zMember1",
        "answered_at_secs": 1_700_000_000,
        "unexpected_key": "hello"
    });
    let out: StandingOutput = serde_json::from_value(val).unwrap();
    assert_eq!(out.member_did, "did:key:zMember1");
    assert_eq!(out.answered_at_secs, 1_700_000_000);
    assert_eq!(out.issuer_did, None);
}

#[test]
fn directory_info_output_reads_an_answer_with_only_name_and_counts() {
    let val = json!({
        "name": "Coop Guild",
        "retention_secs": 86400,
        "member_count": 5,
        "zz": 1
    });
    let out: DirectoryInfoOutput = serde_json::from_value(val).unwrap();
    assert_eq!(out.name, "Coop Guild");
    assert_eq!(out.retention_secs, 86400);
    assert_eq!(out.member_count, 5);
    assert_eq!(out.rules, "");
    assert!(out.area.is_empty());
    assert!(out.categories.is_empty());
    assert_eq!(out.support_contact, "");
    assert_eq!(out.dispute_path, "");
    assert_eq!(out.issuer_did, None);
    assert_eq!(out.signing_did, None);
}

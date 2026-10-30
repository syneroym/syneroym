use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost,
    types::{
        conversation::{DeliveryState, Message},
        proxy::{CallTarget, ProxyError},
        signing::SigningIdentity,
    },
};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_signed_record::{DelegationCertificate, SCOPE_RECORD_SIGNING};

use super::{
    MethodInfo, NoParams, ParamError, Reply, SiblingError, call_sibling,
    check::assert_service_table,
    parse_value,
    profile::{
        AdmissionKind, AdmitFirstContactOutput, BlockCheckOutput, BlockedSince, ProfileRecord,
        ResolveAddressOutput,
    },
    typed,
};
use crate::{
    backup::ImportInput,
    conversation::MessageView,
    envelope::{Request, Response},
    services::PROFILE,
    signing::{
        CertificateStatus, InstallCertificateInput, install_signing_certificate, signing_status,
        tests::TestHost,
    },
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeepTest {
    #[serde(alias = "alias_str")]
    val_str: String,
    val_num: u8,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MidTest {
    deep: DeepTest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TopTest {
    #[serde(alias = "alias_title")]
    title: String,
    count: u8,
    mid: MidTest,
}

#[derive(Debug, Deserialize, Serialize)]
struct WireOpenTest {
    title: String,
}

#[derive(Debug, Serialize)]
struct DummyOutput {
    ok: bool,
}

#[test]
fn parse_reads_null_as_empty_object() {
    let res: Result<NoParams, ParamError> = parse_value(&Value::Null);
    assert!(res.is_ok());
}

#[test]
fn parse_refuses_an_array() {
    let res: Result<NoParams, ParamError> = parse_value(&json!([1, 2]));
    let err = res.unwrap_err();
    assert_eq!(err.field, None);
    assert_eq!(err.message, "invalid params: expected an object");
}

#[test]
fn a_string_params_gives_an_error_with_no_field() {
    let res: Result<NoParams, ParamError> = parse_value(&json!("hello"));
    let err = res.unwrap_err();
    assert_eq!(err.field, None);
    assert_eq!(err.message, "invalid params: expected an object");
}

#[test]
fn test_wrong_type_top_and_nested() {
    let top_wrong = json!({
        "title": 123,
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&top_wrong).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("title"));
    assert!(err.message.contains("invalid type"));

    let deep_wrong = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": 123, "val_num": 1 } }
    });
    let err2 = parse_value::<TopTest>(&deep_wrong).unwrap_err();
    assert_eq!(err2.field.as_deref(), Some("mid.deep.val_str"));
    assert!(err2.message.contains("invalid type"));
}

#[test]
fn test_wrong_value_top_and_nested() {
    let top_wrong = json!({
        "title": "ok",
        "count": 300,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&top_wrong).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("count"));
    assert!(err.message.contains("invalid value"));

    let deep_wrong = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 300 } }
    });
    let err2 = parse_value::<TopTest>(&deep_wrong).unwrap_err();
    assert_eq!(err2.field.as_deref(), Some("mid.deep.val_num"));
    assert!(err2.message.contains("invalid value"));
}

#[test]
fn test_missing_field_top_and_nested() {
    let top_missing = json!({
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&top_missing).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("title"));
    assert_eq!(err.message, "invalid params: missing field `title`");

    let deep_missing = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_num": 1 } }
    });
    let err2 = parse_value::<TopTest>(&deep_missing).unwrap_err();
    assert_eq!(err2.field.as_deref(), Some("mid.deep.val_str"));
    assert_eq!(err2.message, "invalid params: mid.deep: missing field `val_str`");
}

#[test]
fn test_unknown_field_top_and_nested() {
    let top_unknown = json!({
        "extra": 1,
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&top_unknown).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("extra"));
    assert!(err.message.contains("unknown field `extra`"));

    let deep_unknown = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1, "nested_extra": 1 } }
    });
    let err2 = parse_value::<TopTest>(&deep_unknown).unwrap_err();
    assert_eq!(err2.field.as_deref(), Some("mid.deep.nested_extra"));
    assert!(err2.message.contains("unknown field `nested_extra`"));
}

#[test]
fn test_duplicate_field_top_and_nested() {
    let top_dup = json!({
        "title": "ok",
        "alias_title": "dup",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&top_dup).unwrap_err();
    assert!(err.field.as_deref() == Some("title") || err.field.as_deref() == Some("alias_title"));
    assert!(err.message.contains("duplicate field"));

    let deep_dup = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "alias_str": "dup", "val_num": 1 } }
    });
    let err2 = parse_value::<TopTest>(&deep_dup).unwrap_err();
    assert!(
        err2.field.as_deref() == Some("mid.deep.val_str")
            || err2.field.as_deref() == Some("mid.deep.alias_str")
    );
    assert!(err2.message.contains("duplicate field"));
}

#[test]
fn test_message_truncation_stops_at_256_bytes_on_char_boundary() {
    let long_str = "€".repeat(4000);
    let val = json!({ "count": long_str });
    let err = parse_value::<TopTest>(&val).unwrap_err();
    assert!(err.message.len() <= 256);
    assert!(err.message.is_char_boundary(err.message.len()));
    assert!(err.message.ends_with('€'));
}

#[test]
fn unknown_field_name_in_data_is_cut_to_256_bytes() {
    let long_key = "k".repeat(10_000);
    let val = json!({
        long_key: 1,
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_str": "ok", "val_num": 1 } }
    });
    let err = parse_value::<TopTest>(&val).unwrap_err();
    let field = err.field.expect("field present");
    assert!(field.len() <= 256);
    assert_eq!(field.len(), 256);
    assert!(field.is_char_boundary(field.len()));
}

#[tokio::test]
async fn typed_runs_the_handler_with_the_parsed_input() {
    let host = TestHost::default();
    let req = Request {
        method: "test.run".to_string(),
        params: json!({ "title": "hello", "count": 2, "mid": { "deep": { "val_str": "a", "val_num": 3 } } }),
    };
    let res =
        typed(
            &host,
            &req,
            |_h, inp: TopTest| async move { Ok(DummyOutput { ok: inp.count == 2 }) },
        )
        .await;
    assert_eq!(res.error, None);
    assert_eq!(res.result, Some(json!({ "ok": true })));
}

#[tokio::test]
async fn typed_returns_the_handler_error_unchanged() {
    let host = TestHost::default();
    let req = Request {
        method: "test.run".to_string(),
        params: json!({ "title": "hello", "count": 2, "mid": { "deep": { "val_str": "a", "val_num": 3 } } }),
    };
    let res = typed(&host, &req, |_h, _inp: TopTest| async move {
        Err::<DummyOutput, _>(Response::err(-32001, "custom error"))
    })
    .await;
    assert!(res.result.is_none());
    let err = res.error.unwrap();
    assert_eq!(err.code, -32001);
    assert_eq!(err.message, "custom error");
}

#[derive(Serialize)]
struct UnserializableOutput {
    #[serde(serialize_with = "fail_serialize")]
    bad: String,
}

fn fail_serialize<S: serde::Serializer>(_val: &str, _s: S) -> Result<S::Ok, S::Error> {
    Err(serde::ser::Error::custom("cannot serialize"))
}

#[tokio::test]
async fn typed_gives_an_internal_error_when_the_output_cannot_be_written() {
    let host = TestHost::default();
    let req = Request { method: "test.run".to_string(), params: json!({}) };
    let res = typed(&host, &req, |_h, _: NoParams| async move {
        Ok(UnserializableOutput { bad: "fail".to_string() })
    })
    .await;
    assert!(res.result.is_none());
    let err = res.error.unwrap();
    assert_eq!(err.code, -32603);
    assert!(err.message.contains("cannot serialize"));
}

mod test_service {
    use super::*;

    pub async fn h_ping<H: AppHost>(_host: &H, _input: NoParams) -> Reply<DummyOutput> {
        Ok(DummyOutput { ok: true })
    }

    pub async fn h_echo<H: AppHost>(_host: &H, _input: WireOpenTest) -> Reply<DummyOutput> {
        Ok(DummyOutput { ok: true })
    }

    crate::service_methods! {
        "profile.ping": h_ping [ NoParams => DummyOutput ],
        "profile.echo": h_echo [ WireOpenTest => DummyOutput ],
    }
}

#[tokio::test]
async fn service_methods_dispatches_listed_names_and_answers_none_for_others() {
    let host = TestHost::default();
    let rows = test_service::method_rows();
    assert_eq!(rows.len(), 2);

    let req_ok = Request { method: "profile.ping".to_string(), params: json!({}) };
    let resp = test_service::dispatch(&host, &req_ok).await;
    assert!(resp.is_some());
    assert_eq!(resp.unwrap().result, Some(json!({ "ok": true })));

    let req_unknown = Request { method: "profile.unknown".to_string(), params: json!({}) };
    let resp2 = test_service::dispatch(&host, &req_unknown).await;
    assert!(resp2.is_none());
}

#[test]
fn check_accepts_a_correct_table() {
    let rows = vec![
        MethodInfo::of::<NoParams, DummyOutput>("profile.ping"),
        MethodInfo::of::<WireOpenTest, DummyOutput>("profile.echo"),
    ];
    let temp_dir = std::env::temp_dir().join("test_correct_table");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(temp_dir.join("app.rs"), "fn route() { Response::method_not_found(&req.method); }")
        .unwrap();
    assert_service_table(PROFILE, &rows, &["profile.echo"], &temp_dir);
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
#[should_panic(expected = "duplicate method name")]
fn check_fails_on_duplicate_name() {
    let rows = vec![
        MethodInfo::of::<NoParams, DummyOutput>("profile.ping"),
        MethodInfo::of::<NoParams, DummyOutput>("profile.ping"),
    ];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &[], temp.path());
}

#[test]
#[should_panic(expected = "does not route to service")]
fn check_fails_on_wrong_service_route() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("conversation.send")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &[], temp.path());
}

#[test]
#[should_panic(expected = "contains Value")]
fn check_fails_on_value_in_types() {
    let rows = vec![MethodInfo::of::<Value, DummyOutput>("profile.ping")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &[], temp.path());
}

#[test]
#[should_panic(expected = "is not in wire, but does not reject unknown field")]
fn check_fails_on_non_wire_allowing_unknown() {
    let rows = vec![MethodInfo::of::<WireOpenTest, DummyOutput>("profile.ping")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &[], temp.path());
}

#[test]
#[should_panic(expected = "is in wire, but rejects unknown field")]
fn check_fails_on_wire_rejecting_unknown() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &["profile.ping"], temp.path());
}

#[test]
#[should_panic(expected = "wire method profile.missing is not in method table")]
fn check_fails_on_unlisted_wire_method() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &["profile.missing"], temp.path());
}

#[test]
#[should_panic(expected = "service_src_dir does not exist")]
fn check_fails_when_the_source_dir_is_missing() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    assert_service_table(PROFILE, &rows, &[], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "no .rs files found in")]
fn check_fails_when_no_rs_files_found() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    let temp = tempfile::tempdir().unwrap();
    assert_service_table(PROFILE, &rows, &[], temp.path());
}

#[test]
#[should_panic(expected = "hand-written match arm")]
fn check_fails_on_hand_written_arm() {
    let rows = vec![
        MethodInfo::of::<NoParams, DummyOutput>("profile.ping"),
        MethodInfo::of::<NoParams, DummyOutput>("profile.get"),
    ];
    let temp_dir = std::env::temp_dir().join("test_hand_written_arm");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(temp_dir.join("ops.rs"), "\"profile.get\" => handle_get(),\n").unwrap();
    let res = std::panic::catch_unwind(|| {
        assert_service_table(PROFILE, &rows, &[], &temp_dir);
    });
    let _ = fs::remove_dir_all(&temp_dir);
    if let Err(e) = res {
        std::panic::resume_unwind(e);
    }
}

#[test]
#[should_panic(expected = "req.method used outside allowed method_not_found")]
fn check_fails_on_req_method_inspection() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    let temp_dir = std::env::temp_dir().join("test_req_method");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(temp_dir.join("other.rs"), "if req.method == \"profile.ping\" {}").unwrap();
    let res = std::panic::catch_unwind(|| {
        assert_service_table(PROFILE, &rows, &[], &temp_dir);
    });
    let _ = fs::remove_dir_all(&temp_dir);
    if let Err(e) = res {
        std::panic::resume_unwind(e);
    }
}

#[test]
#[should_panic(
    expected = "string literal 'profile.not-in-table' routes to Service { name: \"profile\""
)]
fn check_fails_on_unlisted_method_literal() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    let temp_dir = std::env::temp_dir().join("test_unlisted_literal");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(temp_dir.join("ops.rs"), "let m = \"profile.not-in-table\";").unwrap();
    let res = std::panic::catch_unwind(|| {
        assert_service_table(PROFILE, &rows, &[], &temp_dir);
    });
    let _ = fs::remove_dir_all(&temp_dir);
    if let Err(e) = res {
        std::panic::resume_unwind(e);
    }
}

#[test]
fn message_view_writes_the_same_keys_as_the_old_json() {
    let mut m = Message {
        id: "msg-1".to_string(),
        conversation: "conv-1".to_string(),
        author: "did:key:zAuthor".to_string(),
        outgoing: true,
        sender_timestamp: 1_700_000_000,
        content_type: "text/plain".to_string(),
        body: b"Hello world".to_vec(),
        state: DeliveryState::Delivered,
        last_error: None,
        received_at: 1_700_000_010,
        refused: None,
        restored: false,
        visible_seq: 42,
        verified: true,
        deleted_at: None,
    };
    let view = MessageView::from(&m);
    let val = serde_json::to_value(&view).unwrap();
    let obj = val.as_object().unwrap();

    let expected_keys = [
        "id",
        "conversation",
        "author",
        "direction",
        "sender_timestamp_ms",
        "content_type",
        "body_encoding",
        "state",
        "last_error",
        "stored_at_secs",
        "refused",
        "restored",
        "visible_seq",
        "verified",
        "body",
    ];
    for k in expected_keys {
        assert!(obj.contains_key(k), "missing key {k} in non-deleted message");
    }
    assert!(!obj.contains_key("deleted_at_secs"));
    assert_eq!(obj["body"], json!("Hello world"));
    assert_eq!(obj["direction"], json!("outgoing"));
    assert_eq!(obj["body_encoding"], json!("utf8"));
    assert_eq!(obj["state"], json!("delivered"));
    assert_eq!(obj["last_error"], Value::Null);
    assert_eq!(obj["refused"], Value::Null);

    m.deleted_at = Some(1_700_000_100);
    let view_del = MessageView::from(&m);
    let val_del = serde_json::to_value(&view_del).unwrap();
    let obj_del = val_del.as_object().unwrap();
    assert!(obj_del.contains_key("deleted_at_secs"));
    assert_eq!(obj_del["deleted_at_secs"], json!(1_700_000_100));
    assert!(!obj_del.contains_key("body"));
}

#[test]
fn block_check_output_blocked_and_unblocked_shapes() {
    let unblocked = BlockCheckOutput { blocked: false, since: None };
    let json_unblocked = serde_json::to_value(&unblocked).unwrap();
    assert_eq!(json_unblocked, json!({ "blocked": false }));

    let blocked_with_data = BlockCheckOutput {
        blocked: true,
        since: Some(BlockedSince {
            reason: Some("spam".to_string()),
            since_secs: Some(1_700_000_000),
        }),
    };
    let json_blocked = serde_json::to_value(&blocked_with_data).unwrap();
    assert_eq!(
        json_blocked,
        json!({ "blocked": true, "reason": "spam", "since_secs": 1_700_000_000 })
    );

    let blocked_nulls = BlockCheckOutput {
        blocked: true,
        since: Some(BlockedSince { reason: None, since_secs: None }),
    };
    let json_nulls = serde_json::to_value(&blocked_nulls).unwrap();
    assert_eq!(
        json_nulls,
        json!({ "blocked": true, "reason": Value::Null, "since_secs": Value::Null })
    );
}

#[test]
fn admit_output_leaves_retry_out_unless_rate_limited() {
    let allow = AdmitFirstContactOutput {
        admission: AdmissionKind::Allow,
        blocked: false,
        retry_after_secs: None,
    };
    let j_allow = serde_json::to_value(&allow).unwrap();
    assert_eq!(j_allow, json!({ "admission": "allow", "blocked": false }));
    assert!(!j_allow.as_object().unwrap().contains_key("retry_after_secs"));

    let blocked = AdmitFirstContactOutput {
        admission: AdmissionKind::Blocked,
        blocked: true,
        retry_after_secs: None,
    };
    let j_blocked = serde_json::to_value(&blocked).unwrap();
    assert_eq!(j_blocked, json!({ "admission": "blocked", "blocked": true }));
    assert!(!j_blocked.as_object().unwrap().contains_key("retry_after_secs"));

    let rate_limited = AdmitFirstContactOutput {
        admission: AdmissionKind::RateLimited,
        blocked: false,
        retry_after_secs: Some(3600),
    };
    let j_rate = serde_json::to_value(&rate_limited).unwrap();
    assert_eq!(
        j_rate,
        json!({ "admission": "rate-limited", "blocked": false, "retry_after_secs": 3600 })
    );
}

#[test]
fn import_input_refuses_a_bare_bundle() {
    let bare_bundle = json!({
        "manifest": {
            "bundle_version": 1,
            "subject_did": "did:key:zTest",
            "sections": {}
        },
        "sections": {}
    });
    let err = parse_value::<ImportInput>(&bare_bundle).unwrap_err();
    assert!(err.message.contains("manifest") || err.message.contains("bundle"));

    let empty_err = parse_value::<ImportInput>(&json!({})).unwrap_err();
    assert_eq!(empty_err.field.as_deref(), Some("bundle"));

    let wrapped = json!({ "bundle": bare_bundle });
    let ok = parse_value::<ImportInput>(&wrapped);
    assert!(ok.is_ok());
}

#[tokio::test]
async fn signing_status_and_install_certificate_typed_handlers() {
    let host = TestHost::default();
    let master = Identity::generate().unwrap();
    let service_key = Identity::generate().unwrap();
    let master_did = derive_did_key(&master.public_key());
    let service_did = derive_did_key(&service_key.public_key());

    host.set_signing_id(SigningIdentity {
        signing_did: service_did.clone(),
        pubkey_hex: "00".to_string(),
        owner_did: Some(master_did.clone()),
    });

    let st = signing_status(&host, NoParams {}).await.unwrap();
    assert_eq!(st.signing_did, service_did);
    assert_eq!(st.owner_did, Some(master_did.clone()));
    assert_eq!(st.certificate, CertificateStatus::Missing);

    let cert = DelegationCertificate::issue(
        &master,
        service_key.public_key(),
        86400,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();

    let inst = install_signing_certificate(
        &host,
        InstallCertificateInput { certificate: cert.to_json().unwrap() },
    )
    .await
    .unwrap();
    assert_eq!(inst.master_did, master_did);

    let st_after = signing_status(&host, NoParams {}).await.unwrap();
    assert!(matches!(st_after.certificate, CertificateStatus::Installed { .. }));
}

#[tokio::test]
async fn call_sibling_with_test_host() {
    let host = TestHost::default();

    // 1. Transport error
    host.set_proxy_response(Err(ProxyError::Transport("connection reset".to_string())));
    let res: Result<ResolveAddressOutput, _> =
        call_sibling(&host, PROFILE, "contacts.resolve-address", &json!({})).await;
    assert!(matches!(res, Err(SiblingError::Transport(_))));

    // 2. Refused
    host.set_proxy_response(Ok(json!({
        "error": { "code": -32602, "message": "bad argument" }
    })
    .to_string()));
    let res2: Result<ResolveAddressOutput, _> =
        call_sibling(&host, PROFILE, "contacts.resolve-address", &json!({})).await;
    match res2 {
        Err(SiblingError::Refused(err)) => {
            assert_eq!(err.code, -32602);
            assert_eq!(err.message, "bad argument");
        }
        other => panic!("expected Refused, got {other:?}"),
    }

    // 3. Bad reply (invalid JSON)
    host.set_proxy_response(Ok("invalid json string".to_string()));
    let res3: Result<ResolveAddressOutput, _> =
        call_sibling(&host, PROFILE, "contacts.resolve-address", &json!({})).await;
    assert!(matches!(res3, Err(SiblingError::BadReply(_))));

    // 4. Bad reply (missing result and error)
    host.set_proxy_response(Ok(json!({}).to_string()));
    let res4: Result<ResolveAddressOutput, _> =
        call_sibling(&host, PROFILE, "contacts.resolve-address", &json!({})).await;
    assert!(matches!(res4, Err(SiblingError::BadReply(_))));

    // 5. Success
    host.set_proxy_response(Ok(json!({
        "result": { "conversation_address": "addr-123" }
    })
    .to_string()));
    let res5: Result<ResolveAddressOutput, _> =
        call_sibling(&host, PROFILE, "contacts.resolve-address", &json!({})).await;
    assert_eq!(
        res5.unwrap(),
        ResolveAddressOutput { conversation_address: "addr-123".to_string() }
    );
    let call = host.last_call().expect("last call recorded");
    assert!(matches!(call.target, CallTarget::Dependency(ref d) if d == "profile"));
    assert_eq!(call.interface, PROFILE.interface);
    assert_eq!(call.method, "invoke");
    assert!(call.options.is_none());
    let envelope: Vec<String> = serde_json::from_str(&call.params).unwrap();
    assert_eq!(envelope.len(), 1);
    let inner: Value = serde_json::from_str(&envelope[0]).unwrap();
    assert_eq!(inner["method"], "contacts.resolve-address");
    assert_eq!(inner["params"], json!({}));

    // 6. Null result read as Option<ProfileRecord>
    host.set_proxy_response(Ok(json!({ "result": null }).to_string()));
    let res6: Result<Option<ProfileRecord>, _> =
        call_sibling(&host, PROFILE, "profile.get", &json!({})).await;
    assert_eq!(res6.unwrap(), None);
}

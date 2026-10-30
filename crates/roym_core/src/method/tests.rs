use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::AppHost;

use super::{
    MethodInfo, NoParams, ParamError, Reply, check::assert_service_table, parse_value, typed,
};
use crate::{
    envelope::{Request, Response},
    services::PROFILE,
    signing::tests::TestHost,
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
    assert!(err.message.contains("missing field `title`"));

    let deep_missing = json!({
        "title": "ok",
        "count": 1,
        "mid": { "deep": { "val_num": 1 } }
    });
    let err2 = parse_value::<TopTest>(&deep_missing).unwrap_err();
    assert_eq!(err2.field.as_deref(), Some("mid.deep.val_str"));
    assert!(err2.message.contains("missing field `val_str`"));
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
    let long_str = "x".repeat(10_000);
    let val = json!({ "count": long_str });
    let err = parse_value::<TopTest>(&val).unwrap_err();
    assert!(err.message.len() <= 256);
    assert!(err.message.is_char_boundary(err.message.len()));
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
    assert_service_table(PROFILE, &rows, &[], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "does not route to service")]
fn check_fails_on_wrong_service_route() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("conversation.send")];
    assert_service_table(PROFILE, &rows, &[], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "contains Value")]
fn check_fails_on_value_in_types() {
    let rows = vec![MethodInfo::of::<Value, DummyOutput>("profile.ping")];
    assert_service_table(PROFILE, &rows, &[], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "is not in wire, but does not reject unknown field")]
fn check_fails_on_non_wire_allowing_unknown() {
    let rows = vec![MethodInfo::of::<WireOpenTest, DummyOutput>("profile.ping")];
    assert_service_table(PROFILE, &rows, &[], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "is in wire, but rejects unknown field")]
fn check_fails_on_wire_rejecting_unknown() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    assert_service_table(PROFILE, &rows, &["profile.ping"], Path::new("/nonexistent"));
}

#[test]
#[should_panic(expected = "wire method profile.missing is not in method table")]
fn check_fails_on_unlisted_wire_method() {
    let rows = vec![MethodInfo::of::<NoParams, DummyOutput>("profile.ping")];
    assert_service_table(PROFILE, &rows, &["profile.missing"], Path::new("/nonexistent"));
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

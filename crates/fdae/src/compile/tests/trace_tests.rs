use super::{plan_tests::remote_relation_policy, *};

#[test]
fn default_permission_not_covering_operation_is_denied() {
    // Regression for a privilege escalation: `default` used
    // to apply regardless of whether its own permission's `allows`
    // covered the requested operation, so a caller holding *only* a
    // write capability could ride a read-only (or ability-less)
    // default permission's paths straight through a write-mode check.
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "default": "fallback",
                    "permissions": {
                        "fallback": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
    let write_cap = Capability {
        with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![write_cap]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        sieve.where_clause, "0=1",
        "a write-mode check must not fall through a read-only default permission"
    );
}

#[test]
fn collection_lookup_is_case_insensitive_like_sqlite() {
    // Regression for a policy bypass: SQLite resolves
    // table names case-insensitively, so a case-sensitive
    // `find_definition` let a caller spell the collection differently
    // than the policy and fall through to the unfiltered "no
    // definition" path against the *same* physical table.
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "DOCUMENT",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert!(
        sieve.is_some(),
        "a differently-cased collection name must still resolve to the same definition, not fall \
         through to unfiltered"
    );
}

#[test]
fn caveat_fields_deny_with_a_dotted_path_fails_closed() {
    // Regression: a runtime capability caveat can't be rejected at
    // policy parse time the way a policy `fields.deny` entry is, but a
    // dotted entry would silently mask nothing the same way -- fail
    // the compile instead of returning an unenforced mask.
    let policy = single_hop_policy();
    let cap = Capability {
        with: resource("document"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"fields": {"deny": ["profile.ssn"]}})),
    };
    let alice = session("did:key:alice", vec![cap]);
    let err = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

// -- ADR-0017 §9 decision trace ---------------------------------------
//
// `compile_read` returns the same `DecisionTrace` it emits via `tracing`
// on `CompiledSieve::trace`, so these assert on the struct directly
// rather than capturing log output. `do_check_access`'s "rows not
// reached" trace (the fourth deny reason -- known only after Mode A
// actually executes the compiled predicate against a row) is covered in
// `data_db`, the layer that runs that query.

#[test]
fn decision_trace_records_operation_not_admitted() {
    // No capabilities at all: nothing grants the operation on this
    // resource, distinct from holding a grant but reaching no row.
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(sieve.trace.tier, 3);
    assert!(!sieve.trace.operation_admitted);
    assert!(sieve.trace.held.is_empty());
    assert!(
        sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("no held capability")),
        "path_failed was: {:?}",
        sieve.trace.path_failed
    );
}

#[test]
fn decision_trace_records_strict_unknown_collection() {
    let policy =
        parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let sieve = compile_read(
        &policy,
        "unrelated_collection",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(sieve.trace.tier, 3);
    assert!(!sieve.trace.operation_admitted);
    assert!(
        sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("strict")),
        "path_failed was: {:?}",
        sieve.trace.path_failed
    );
}

#[test]
fn decision_trace_records_claim_absent() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "user": {
                    "table": "users",
                    "principal_column": "did",
                    "permissions": {
                        "view_self": {
                            "allows": ["data-layer/read"],
                            "paths": [["caller"]],
                            "conditions": [{"column": "region", "claim": "region"}]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let alice = session("did:key:alice", vec![read_cap(Some("user"))]);
    let sieve = compile_read(
        &policy,
        "user",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(sieve.trace.operation_admitted);
    assert_eq!(sieve.trace.applicable_permissions, vec!["view_self".to_string()]);
    assert!(
        sieve.trace.path_failed.as_deref().is_some_and(|r| r.contains("claim absent")),
        "path_failed was: {:?}",
        sieve.trace.path_failed
    );
}

#[test]
fn decision_trace_records_allow_with_no_path_failed() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(sieve.trace.operation_admitted);
    assert_eq!(sieve.trace.applicable_permissions, vec!["view".to_string()]);
    assert!(sieve.trace.path_failed.is_none());
    assert_eq!(sieve.trace.compiled_predicate.as_deref(), Some(sieve.where_clause.as_str()));
    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

#[test]
fn decision_trace_claim_absent_across_multiple_permissions_is_detected() {
    // Regression: the deny used to be detected by string-matching the
    // *joined* predicate against the literal "(0=1)", which only holds
    // for a single applicable permission. Two permissions that both
    // fail claim resolution OR together as "(0=1 OR 0=1)" -- a
    // different string -- so the old check silently missed this and
    // logged the decision as an allow.
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view_a": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "conditions": [{"column": "region", "claim": "region"}]
                        },
                        "view_b": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "conditions": [{"column": "tier", "claim": "tier"}]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
    // A plain platform-ability read capability makes both `view_a` and
    // `view_b` applicable (each's `allows` covers `data-layer/read`);
    // neither `region` nor `tier` is in the caller's claims, so both
    // clauses fail closed independently.
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        sieve.where_clause, "(0=1 OR 0=1)",
        "sanity: both permissions' paths must actually be OR'd, not deduplicated"
    );
    assert!(sieve.trace.operation_admitted);
    assert_eq!(
        sieve.trace.applicable_permissions,
        vec!["view_a".to_string(), "view_b".to_string()]
    );
    let reason = sieve.trace.path_failed.expect("a fully claim-absent deny must be traced");
    assert!(reason.contains("view_a") && reason.contains("view_b"), "reason was: {reason}");
}

#[test]
fn decision_trace_records_default_not_covering_operation() {
    // The fifth deny reason (compile.rs's H1-hardened branch): the
    // caller holds a grant for the operation, but no permission's
    // `allows` covers it and the configured `default` doesn't either --
    // distinct from "operation not admitted" (no grant at all). Same
    // policy/capability shape as
    // `default_permission_not_covering_operation_is_denied`, which pins
    // the SQL; this test pins the trace.
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "default": "fallback",
                    "permissions": {
                        "fallback": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
    let write_cap = Capability {
        with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![write_cap]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();
    assert!(
        sieve.trace.operation_admitted,
        "the caller does hold a grant for the operation -- distinct from no grant at all"
    );
    assert!(sieve.trace.applicable_permissions.is_empty());
    assert!(
        sieve
            .trace
            .path_failed
            .as_deref()
            .is_some_and(|r| r.contains("no applicable default permission")),
        "path_failed was: {:?}",
        sieve.trace.path_failed
    );
}

#[test]
fn compile_read_emits_a_deny_via_tracing() {
    // Nothing else in this suite proves `compile_read` actually calls
    // `trace.emit()` -- every other decision-trace test asserts on the
    // `CompiledSieve::trace` field the function *returns*, which would
    // stay green even if the `emit()` calls inside `compile_read` were
    // deleted entirely. This test captures real `tracing` output around
    // a call, the same way `data_db::sqlite::tests::decision_trace_
    // records_rows_not_reached_after_check_access_executes` proves
    // `do_check_access`'s own `emit()`.
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use tracing_subscriber::prelude::*;

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let policy =
        parse_and_validate(r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    tracing::subscriber::with_default(subscriber, || {
        let _ = compile_read(
            &policy,
            "unrelated_collection",
            &alice,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
    });

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
    assert!(logs_content.contains("unrelated_collection"), "logs were: {logs_content}");
    assert!(logs_content.contains("did:key:alice"), "logs were: {logs_content}");
}

/// When `compile_read` rejects a plan needing a remote fetch,
/// the trace record must say so -- `plan_read`'s own trace (emitted
/// first, before `compile_read` sees the fetch count) necessarily
/// reads as an allow, since `plan_read` itself doesn't fail; without
/// this fix an operator's log would show only that allow-shaped
/// record for a call that actually denied.
#[test]
fn compile_read_emits_its_own_deny_trace_when_a_remote_fetch_is_needed() {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use tracing_subscriber::prelude::*;

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    tracing::subscriber::with_default(subscriber, || {
        let _ = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap_err();
    });

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
    assert!(
        logs_content.contains("remote relationship fetch"),
        "the deny reason must name the unresolved fetch: {logs_content}"
    );
}

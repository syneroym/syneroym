use super::*;

#[test]
fn single_hop_exists_prunes_to_the_creator() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_user(&conn, "u-bob", "did:key:bob", None);
    insert_document(&conn, "doc-1", "u-alice");
    insert_document(&conn, "doc-2", "u-bob");

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

    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

#[test]
fn single_hop_denies_a_stranger() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_policy();
    let mallory = session("did:key:mallory", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &mallory,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();

    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
}

fn single_hop_anchor_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "anchor"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// ADR-0015 A5 (amended): `anchor` filters by the original principal a
/// proxying caller acts for, not the presenting caller itself -- the
/// confused-deputy defense. A caller presenting `subject_did = svc_1`
/// but anchored to `did:key:alice` reaches alice's row.
#[test]
fn anchor_terminal_filters_by_the_original_principal_not_the_caller() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_anchor_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();

    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

/// A direct call carries no distinct anchor (`anchor_did == None`) --
/// the compiler falls back to `subject_did` (a direct caller *is* the
/// anchor) rather than denying the policy.
#[test]
fn anchor_terminal_falls_back_to_subject_did_when_anchor_is_absent() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_anchor_policy();
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

    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

/// The mirror of the filtering test: a caller who *does* own the row is
/// denied when acting for an anchor who doesn't. Using
/// `subject_did = alice` (the row's actual owner) is the discriminating
/// case -- if the sieve wrongly bound `caller` instead of `anchor`, this
/// row would leak; a stranger `subject_did` (as in an earlier version of
/// this test) can't tell the two apart, since neither identity would
/// match either way.
#[test]
fn anchor_terminal_denies_when_the_anchor_is_a_stranger() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_anchor_policy();
    let proxying_service =
        session_with_anchor("did:key:alice", "did:key:mallory", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();

    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
}

/// The decision trace must surface `session.anchor_did` (ADR-0015 A5,
/// amended) -- without it, an operator reading the log line for a
/// proxying caller cannot tell whether the decision was made for
/// `subject_did` or for a different principal it was acting on behalf
/// of, which is exactly what the anchor mechanism exists to make
/// auditable.
#[test]
fn decision_trace_records_the_anchor_did() {
    let policy = single_hop_anchor_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(sieve.trace.subject_did, "did:key:svc-1");
    assert_eq!(sieve.trace.anchor_did.as_deref(), Some("did:key:alice"));
}

/// `anchor` resolution must hold in `Mode::PointInTime` too -- a wrong
/// terminal there is a boolean allow/deny, not merely a missing row, so
/// this exercises a code path Mode B's tests don't reach.
#[test]
fn anchor_terminal_holds_in_point_in_time_mode() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_anchor_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();
    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);

    let proxying_service_wrong_anchor =
        session_with_anchor("did:key:svc-1", "did:key:mallory", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service_wrong_anchor,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();
    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
}

/// `anchor` resolution on a multi-hop, non-recursive chain --
/// `emit_chain` resolves the terminal on a separate code path from the
/// single-hop/zero-hop case.
#[test]
fn anchor_terminal_holds_across_a_multi_hop_chain() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "
        CREATE TABLE departments (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
        CREATE TABLE users (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
        CREATE TABLE documents (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
        ",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO departments (id, payload) VALUES ('dept-eng', ?1)",
        [json!({"owner_did": "did:key:carol"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO users (id, payload) VALUES ('u-alice', ?1)",
        [json!({"dept_id": "dept-eng"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"creator_uuid": "u-alice"}).to_string()],
    )
    .unwrap();

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "home_department", "anchor"]]
                        }
                    }
                },
                "user": {
                    "table": "users",
                    "relations": {"home_department": {"target": "department", "join_column": "dept_id"}}
                },
                "department": {"table": "departments", "principal_column": "owner_did"}
            }
        }"#,
    )
    .unwrap();

    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:carol", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-1"],
        "carol is the anchor, two joins away from the document -- not the caller (svc-1)"
    );
}

/// `anchor` resolution on a recursive relation -- `emit_fused_recursive`
/// resolves the terminal on a separate code path from the non-recursive
/// cases above.
#[test]
fn anchor_terminal_holds_on_a_recursive_relation() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-eve", "did:key:eve", Some("u-frank"));
    insert_user(&conn, "u-frank", "did:key:frank", Some("u-eve"));
    insert_user(&conn, "u-mallory", "did:key:mallory", None);
    insert_document(&conn, "doc-1", "u-eve");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "management_chain", "anchor"]]
                        }
                    }
                },
                "user": {
                    "table": "users",
                    "principal_column": "did",
                    "relations": {
                        "management_chain": {
                            "target": "user", "from_key": "id", "to_key": "manager_id",
                            "recursive": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();

    for (anchor_did, expect_visible) in
        [("did:key:eve", true), ("did:key:frank", true), ("did:key:mallory", false)]
    {
        let proxying_service =
            session_with_anchor("did:key:svc-1", anchor_did, vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &proxying_service,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        let visible = run_sieve(&conn, "documents", &sieve).contains(&"doc-1".to_string());
        assert_eq!(visible, expect_visible, "anchor {anchor_did}");
    }
}

#[test]
fn recursive_relation_terminates_on_a_cyclic_manager_graph() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    // eve -> frank -> eve: a deliberately cyclic manager graph.
    insert_user(&conn, "u-eve", "did:key:eve", Some("u-frank"));
    insert_user(&conn, "u-frank", "did:key:frank", Some("u-eve"));
    insert_user(&conn, "u-mallory", "did:key:mallory", None);
    insert_document(&conn, "doc-1", "u-eve");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "management_chain", "caller"]]
                        }
                    }
                },
                "user": {
                    "table": "users",
                    "principal_column": "did",
                    "relations": {
                        "management_chain": {
                            "target": "user", "from_key": "id", "to_key": "manager_id",
                            "recursive": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();

    for (did, expect_visible) in
        [("did:key:eve", true), ("did:key:frank", true), ("did:key:mallory", false)]
    {
        let s = session(did, vec![read_cap(Some("document"))]);
        let sieve = compile_read(
            &policy,
            "document",
            &s,
            SERVICE_ID,
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap()
        .unwrap();
        let visible = run_sieve(&conn, "documents", &sieve);
        assert_eq!(!visible.is_empty(), expect_visible, "did={did}");
    }
}

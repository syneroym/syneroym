use std::collections::BTreeSet;

use rusqlite::Connection;
use serde_json::{Map, json};
use syneroym_ucan::{Ability, Capability, ResourceUri, ResourceUri as Uri, SessionContext};

use super::*;
use crate::{
    policy::{Policy, PolicyError, parse_and_validate},
    trace::RemoteFetchTrace,
};

const SERVICE_ID: &str = "svc-a";

fn resource(collection: &str) -> Uri {
    Uri(format!("{}/collection/{collection}", Uri::service(SERVICE_ID, SERVICE_ID).0))
}

fn session(subject_did: &str, capabilities: Vec<Capability>) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: None,
        capabilities,
        claims: Map::new(),
        verified_at_secs: 0,
    }
}

fn session_with_anchor(
    subject_did: &str,
    anchor_did: &str,
    capabilities: Vec<Capability>,
) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: Some(anchor_did.to_string()),
        capabilities,
        claims: Map::new(),
        verified_at_secs: 0,
    }
}

fn read_cap(collection: Option<&str>) -> Capability {
    let with = match collection {
        Some(c) => resource(c),
        None => ResourceUri::service(SERVICE_ID, SERVICE_ID),
    };
    Capability { with, can: Ability(Ability::DATA_LAYER_READ.to_string()), caveats: None }
}

fn run_sieve(conn: &Connection, base_table: &str, sieve: &CompiledSieve) -> Vec<String> {
    let sql = format!(
        "SELECT {base_table}.id FROM {base_table} WHERE {} ORDER BY {base_table}.id",
        sieve.where_clause
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(rusqlite::params_from_iter(sieve.params.iter()), |row| row.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn seed_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE users (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );
        ",
    )
    .unwrap();
}

fn insert_user(conn: &Connection, id: &str, did: &str, manager_id: Option<&str>) {
    let payload = json!({"did": did, "manager_id": manager_id});
    conn.execute("INSERT INTO users (id, payload) VALUES (?1, ?2)", (id, payload.to_string()))
        .unwrap();
}

fn insert_document(conn: &Connection, id: &str, creator_uuid: &str) {
    let payload = json!({"creator_uuid": creator_uuid});
    conn.execute("INSERT INTO documents (id, payload) VALUES (?1, ?2)", (id, payload.to_string()))
        .unwrap();
}

fn single_hop_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

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

#[test]
fn exclusion_operator_and_condition_claim_bind_correctly() {
    // `view` = reachable as creator, EXCLUDING documents that
    // specifically embargo the caller, further ANDed with a
    // claims-bound region match. Both `creator` and `embargoed_from`
    // are ordinary single-hop (many-to-one) relations, so this
    // exercises the exclusion operator and the conditions/claims bind
    // through the real relation-walk machinery, not a hand-rolled
    // shape assertion.
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_user(&conn, "u-bob", "did:key:bob", None);
    let insert_embargo_doc = |id: &str, creator: &str, embargoed: &str, region: &str| {
        let payload =
            json!({"creator_uuid": creator, "embargoed_uuid": embargoed, "region": region});
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
            (id, payload.to_string()),
        )
        .unwrap();
    };
    // alice created all three; doc-1 embargoes alice herself (excluded
    // despite being creator); doc-2 embargoes bob instead (visible to
    // alice); doc-3 is like doc-2 but in a different region.
    insert_embargo_doc("doc-1", "u-alice", "u-alice", "EU");
    insert_embargo_doc("doc-2", "u-alice", "u-bob", "EU");
    insert_embargo_doc("doc-3", "u-alice", "u-bob", "US");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "creator": {"target": "user", "join_column": "creator_uuid"},
                        "embargoed_from": {"target": "user", "join_column": "embargoed_uuid"}
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "exclusion",
                            "paths": [["creator", "caller"], ["embargoed_from", "caller"]],
                            "conditions": [{"column": "region", "claim": "region"}]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();

    let alice_with_region = |region: &str| {
        let mut claims = Map::new();
        claims.insert("region".to_string(), json!(region));
        SessionContext {
            subject_did: "did:key:alice".to_string(),
            anchor_did: None,
            capabilities: vec![read_cap(Some("document"))],
            claims,
            verified_at_secs: 0,
        }
    };

    let alice_eu = alice_with_region("EU");
    let sieve = compile_read(
        &policy,
        "document",
        &alice_eu,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-2"],
        "doc-1 excluded (alice is embargoed from it); doc-3 excluded (region mismatch)"
    );

    let alice_us = alice_with_region("US");
    let sieve = compile_read(
        &policy,
        "document",
        &alice_us,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-3"],
        "region claim correctly switches the visible row"
    );
}

#[test]
fn condition_with_absent_claim_fails_closed() {
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
    assert_eq!(sieve.where_clause, "(0=1)");
}

#[test]
fn no_definition_and_not_strict_is_unfiltered() {
    let policy = parse_and_validate(r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let alice = session("did:key:alice", vec![]);
    let sieve = compile_read(
        &policy,
        "unrelated_collection",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert!(sieve.is_none());
}

#[test]
fn strict_mode_denies_an_undefined_collection() {
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
    assert_eq!(sieve.where_clause, "0=1");
}

#[test]
fn no_applicable_permission_and_no_default_denies() {
    let policy = single_hop_policy();
    // A capability for a *different* collection: covers the operation
    // but not this resource, so no permission becomes applicable and
    // there is no `default` -- default-deny.
    let bob = session("did:key:bob", vec![read_cap(Some("other_collection"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &bob,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(sieve.where_clause, "0=1");
}

#[test]
fn write_mode_check_ignores_a_read_only_permission() {
    // "view" only allows data-layer/read; a caller holding *only* a
    // read capability must not pass a write-mode point-in-time check
    // through it.
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
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
    assert_eq!(sieve.where_clause, "0=1");
}

#[test]
fn write_capable_permission_also_covers_a_read_check() {
    // "manage" allows both read and write; entailment means a
    // write-capable grant also satisfies a read-mode check.
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {
                            "allows": ["data-layer/read", "data-layer/write"],
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
        with: resource("document"),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![write_cap]);
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
    assert_ne!(sieve.where_clause, "0=1");
}

/// Isolates the `includes` closure from direct entailment: the caller
/// holds *only* an app-permission grant for "manage" (`app/document.
/// manage`, a flat, self-entailing-only ability string), never a
/// platform-ability capability -- so "view" can only ever become
/// applicable through `manage`'s `includes`, never through the direct
/// route. This is what makes the write-mode assertion below a real
/// regression test for a privilege escalation: the closure used to widen
/// unconditionally, so a write-mode check would previously have pulled in
/// "view" (read-only) anyway.
#[test]
fn includes_closure_is_gated_by_the_included_permissions_own_allows() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "creator": {"target": "user", "join_column": "creator_uuid"},
                        "parent_dept": {"target": "department", "join_column": "owner_dept_id"}
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]]
                        },
                        "manage": {
                            "allows": ["data-layer/write"],
                            "includes": ["view"],
                            "paths": [["parent_dept", "caller"]]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"},
                "department": {"table": "departments", "principal_column": "owner_did"}
            }
        }"#,
    )
    .unwrap();
    let app_cap = Capability {
        with: resource("document"),
        can: Ability("app/document.manage".to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![app_cap]);

    // Write mode: "view" (allows: read) does not cover write, so
    // closure must NOT pull its path in -- the predicate is exactly
    // manage's own path, no OR.
    let write_sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(
        !write_sieve.where_clause.contains(" OR "),
        "a write-mode check must not pull in a read-only included permission's path"
    );

    // Read mode: "manage" is still applicable (its own `allows: write`
    // entails read), and "view" (allows: read) *does* cover this
    // operation -- closure should widen to OR its path in too.
    let read_sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(
        read_sieve.where_clause.contains(" OR "),
        "a read-mode check should widen through includes when the included permission covers it"
    );
}

#[test]
fn collection_selector_grant_is_honored_and_scoped() {
    // Guards grant-intersection scoping: a capability scoped to
    // `.../collection/document` must be admitted for `document` and
    // denied for an unrelated collection under the *same* service.
    let policy = single_hop_policy();
    let scoped = Capability {
        with: resource("document"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![scoped]);
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
    assert_ne!(
        sieve.where_clause, "0=1",
        "the scoped grant must be admitted for its own collection"
    );
}

#[test]
fn app_permission_route_admits_a_named_grant() {
    let policy = single_hop_policy();
    let named = Capability {
        with: resource("document"),
        can: Ability("app/document.view".to_string()),
        caveats: None,
    };
    let alice = session("did:key:alice", vec![named]);
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
    assert_ne!(sieve.where_clause, "0=1");
}

fn remote_relation_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"owner": {
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "did:key:zHrSvc"
                    }},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// `compile_read` (the synchronous, local-only entry point) still fails
/// closed on a policy whose selected path needs a remote fetch -- it has
/// no way to perform one itself. `plan_read` is the two-phase entry point
/// that actually resolves it (see the tests below).
#[test]
fn compile_read_fails_closed_when_a_remote_fetch_is_needed() {
    let policy = remote_relation_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
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

/// `plan_read` splits a policy needing a remote relation into a
/// `RemoteFetch` (carrying the anchor as principal, per the
/// confused-deputy defense) plus a `PendingSieve`, instead of failing
/// closed -- the plan half of the two-phase compile.
#[test]
fn plan_read_collects_a_remote_fetch_instead_of_failing_closed() {
    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();

    assert!(plan.local.is_none());
    assert!(plan.pending.is_some());
    assert_eq!(plan.fetches.len(), 1);
    let fetch = &plan.fetches[0];
    assert_eq!(fetch.service, "hr-svc");
    assert_eq!(fetch.relation, "employee", "the wire relation is the remote object type");
    assert_eq!(
        fetch.principal_did, "did:key:alice",
        "the fetch's principal is the anchor, not the proxying caller"
    );
}

/// A fully-local policy plans identically to `compile_read` -- `local`
/// carries the finished sieve and `fetches`/`pending` are empty/`None`,
/// matching the fully-local shape. Zero behavior change for every
/// existing local-only policy.
#[test]
fn plan_read_of_a_fully_local_policy_has_no_fetches() {
    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert!(plan.fetches.is_empty());
    assert!(plan.pending.is_none());
    assert!(plan.local.is_some());
}

/// `abac_permissions` lists only the applicable permissions that opted
/// into the stage-4 after-step, not every applicable permission -- a
/// single capability entitles both `view` and `view_secret` here (the
/// grant∩policy intersection), but only `view_secret` set
/// `authorize_rows: true`.
#[test]
fn abac_permissions_lists_only_opted_in_applicable_permissions() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": []},
                        "view_secret": {
                            "allows": ["data-layer/read"], "paths": [],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
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
    assert_eq!(sieve.abac_permissions, vec!["view_secret".to_string()]);
}

/// A denied read (no entitling capability at all) never runs the
/// after-step -- `deny_all()`'s `abac_permissions` is always empty,
/// regardless of what the policy declares.
#[test]
fn deny_all_carries_no_abac_permissions() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "permissions": {
                        "view_secret": {
                            "allows": ["data-layer/read"], "paths": [],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let stranger = session("did:key:stranger", vec![]);
    let sieve = compile_read(
        &policy,
        "document",
        &stranger,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(sieve.abac_permissions.is_empty());
    assert_eq!(sieve.where_clause, "0=1");
}

/// `finalize` binds a fetched id-set into the pending sieve's `IN (...)`
/// predicate and runs correctly against real seeded rows: the local row
/// whose `owner_uuid` is in the fetched set is visible; one that isn't,
/// isn't.
#[test]
fn finalize_binds_the_fetched_id_set_and_runs_correctly() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    let insert = |id: &str, owner_uuid: &str| {
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
            (id, json!({"owner_uuid": owner_uuid}).to_string()),
        )
        .unwrap();
    };
    insert("doc-1", "emp-alice");
    insert("doc-2", "emp-bob");

    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();

    let fetch_trace = RemoteFetchTrace {
        service: "hr-svc".to_string(),
        relation: "employee".to_string(),
        principal_did: "did:key:alice".to_string(),
        asserter_did: "did:key:zHrSvc".to_string(),
        valid_until_secs: 1_000,
    };
    let results =
        vec![FetchResult { slot, ids: vec!["emp-alice".to_string()], trace: fetch_trace.clone() }];
    let sieve = finalize(pending, &results).unwrap();
    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
    assert_eq!(
        sieve.trace.remote_fetches,
        vec![fetch_trace],
        "a successful fetch must leave provenance in the DecisionTrace, not just the deny path"
    );
}

/// `abac_permissions` survives the two-phase compile: `plan_read`
/// computes it before any fetch is known, `PendingSieve` carries it
/// across the `await` a real fetch would sit behind, and `finalize`'s
/// exhaustive destructure/rebuild must not drop it.
#[test]
fn finalize_preserves_abac_permissions_through_a_remote_fetch() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"owner": {
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "did:key:zHrSvc"
                    }},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"], "paths": [["owner", "anchor"]],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();

    let results = vec![FetchResult {
        slot,
        ids: vec!["emp-alice".to_string()],
        trace: RemoteFetchTrace {
            service: "hr-svc".to_string(),
            relation: "employee".to_string(),
            principal_did: "did:key:alice".to_string(),
            asserter_did: "did:key:zHrSvc".to_string(),
            valid_until_secs: 1_000,
        },
    }];
    let sieve = finalize(pending, &results).unwrap();
    assert_eq!(sieve.abac_permissions, vec!["view".to_string()]);
}

/// Mirrors the above with an empty fetched id-set: `IN (SELECT 1 WHERE
/// 0)` is valid SQL, unambiguously `false` (never `NULL`, unlike
/// `IN (NULL)`), and never the invalid `IN ()`.
#[test]
fn finalize_binds_an_empty_id_set_as_a_false_empty_subquery_not_invalid_sql() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"owner_uuid": "emp-alice"}).to_string()],
    )
    .unwrap();

    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();
    let sieve = finalize(
        pending,
        &[FetchResult { slot, ids: Vec::new(), trace: RemoteFetchTrace::default() }],
    )
    .unwrap();
    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
}

/// An `exclusion`-operator permission with a remote
/// hop that legitimately resolves to nobody must exclude *nobody* (the
/// row stays visible) -- not deny every row, which `{col} IN (NULL)`'s
/// three-valued-logic inversion under `NOT` would have caused.
#[test]
fn finalize_exclusion_operator_with_an_empty_remote_fetch_excludes_nobody() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "creator": {"target": "user", "join_column": "creator_uuid"},
                        "embargoed_from": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "embargoed_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "exclusion",
                            "paths": [["creator", "caller"], ["embargoed_from", "anchor"]]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();

    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();
    // The remote legitimately knows of nobody embargoed -- an honest
    // empty answer, not a fetch failure.
    let sieve = finalize(
        pending,
        &[FetchResult { slot, ids: Vec::new(), trace: RemoteFetchTrace::default() }],
    )
    .unwrap();
    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-1"],
        "an empty embargo list must exclude nobody, not deny everyone"
    );
}

/// `finalize`'s `params_index + shift` insertion arithmetic under two
/// *distinct* remote relations (different targets, so they don't dedupe)
/// at different text/param positions: `intersection` requires
/// both `owner` and `department` to match, each bound from its own
/// fetched id-set, spliced at the correct offset into a single flat
/// `params` vector. The delicate part -- verified by the assertion
/// below, not just traced by hand -- is that the *second* marker's ids
/// land after the *first* marker's already-inserted ids, not at the
/// position `params.len()` had *before* any insertion.
#[test]
fn finalize_binds_two_distinct_remote_fetches_at_the_correct_offsets() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    let insert = |id: &str, owner_uuid: &str, department_uuid: &str| {
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
            (id, json!({"owner_uuid": owner_uuid, "department_uuid": department_uuid}).to_string()),
        )
        .unwrap();
    };
    // doc-1 matches both fetches; doc-2 matches only the owner fetch;
    // doc-3 matches only the department fetch -- only doc-1 should
    // survive the `intersection`.
    insert("doc-1", "emp-alice", "team-eng");
    insert("doc-2", "emp-alice", "team-sales");
    insert("doc-3", "emp-bob", "team-eng");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        },
                        "department": {
                            "target": "team", "service": "hr-svc",
                            "join_column": "department_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "intersection",
                            "paths": [["owner", "anchor"], ["department", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert_eq!(plan.fetches.len(), 2, "owner and department are different target types");
    let owner_slot = plan.fetches.iter().find(|f| f.relation == "employee").unwrap().slot;
    let dept_slot = plan.fetches.iter().find(|f| f.relation == "team").unwrap().slot;
    let pending = plan.pending.take().unwrap();

    let results = vec![
        FetchResult {
            slot: owner_slot,
            ids: vec!["emp-alice".to_string()],
            trace: RemoteFetchTrace::default(),
        },
        FetchResult {
            slot: dept_slot,
            ids: vec!["team-eng".to_string()],
            trace: RemoteFetchTrace::default(),
        },
    ];
    let sieve = finalize(pending, &results).unwrap();
    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

/// The same shape as
/// `finalize_binds_two_distinct_remote_fetches_at_the_correct_offsets`,
/// but with the fetch results supplied in the *opposite* order --
/// `finalize` sorts by `params_index` internally, so the caller's
/// `results` ordering must not matter.
#[test]
fn finalize_binds_two_distinct_remote_fetches_regardless_of_result_order() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"owner_uuid": "emp-alice", "department_uuid": "team-eng"}).to_string()],
    )
    .unwrap();

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        },
                        "department": {
                            "target": "team", "service": "hr-svc",
                            "join_column": "department_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "intersection",
                            "paths": [["owner", "anchor"], ["department", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let owner_slot = plan.fetches.iter().find(|f| f.relation == "employee").unwrap().slot;
    let dept_slot = plan.fetches.iter().find(|f| f.relation == "team").unwrap().slot;
    let pending = plan.pending.take().unwrap();

    // Reversed vs. the sibling test.
    let results = vec![
        FetchResult {
            slot: dept_slot,
            ids: vec!["team-eng".to_string()],
            trace: RemoteFetchTrace::default(),
        },
        FetchResult {
            slot: owner_slot,
            ids: vec!["emp-alice".to_string()],
            trace: RemoteFetchTrace::default(),
        },
    ];
    let sieve = finalize(pending, &results).unwrap();
    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

/// One fetch shape serves both Mode A and Mode B. Mode A
/// (point-in-time) over a remote relation was asserted in the plan but
/// never actually run -- this exercises it for real: the `id = ?`
/// predicate `Mode::PointInTime` ANDs on must survive alongside the
/// finalized remote predicate.
#[test]
fn finalize_holds_in_point_in_time_mode_over_a_remote_relation() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"owner_uuid": "emp-alice"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-2', ?1)",
        [json!({"owner_uuid": "emp-alice"}).to_string()],
    )
    .unwrap();

    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();
    let sieve = finalize(
        pending,
        &[FetchResult {
            slot,
            ids: vec!["emp-alice".to_string()],
            trace: RemoteFetchTrace::default(),
        }],
    )
    .unwrap();

    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-1"],
        "doc-1 is reachable and matches the point-in-time id"
    );

    // Both doc-1 and doc-2 are reachable via the fetch (same owner) --
    // asking about doc-2 specifically must return doc-2, not doc-1 or
    // both, proving the `id = ?` predicate still narrows correctly on
    // top of the finalized remote predicate.
    let mut plan2 = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-2".to_string() },
    )
    .unwrap();
    let slot2 = plan2.fetches[0].slot;
    let pending2 = plan2.pending.take().unwrap();
    let sieve2 = finalize(
        pending2,
        &[FetchResult {
            slot: slot2,
            ids: vec!["emp-alice".to_string()],
            trace: RemoteFetchTrace::default(),
        }],
    )
    .unwrap();
    assert_eq!(run_sieve(&conn, "documents", &sieve2), vec!["doc-2"]);
}

/// An id-set larger than `MAX_FETCH_IDS` is rejected rather than
/// silently truncated or spliced into an unbounded `IN (...)` list
/// (fan-out containment).
#[test]
fn finalize_rejects_an_oversized_id_set() {
    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();
    let oversized: Vec<String> = (0..MAX_FETCH_IDS + 1).map(|i| format!("id-{i}")).collect();
    let err = finalize(
        pending,
        &[FetchResult { slot, ids: oversized, trace: RemoteFetchTrace::default() }],
    )
    .unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// `finalize` fails closed, rather than panicking or silently leaving
/// the marker text in place, when `results` is missing a slot the
/// pending sieve actually needs.
#[test]
fn finalize_fails_closed_on_a_missing_fetch_result() {
    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .pending
    .unwrap();
    let err = finalize(plan, &[]).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// A `recursive: true` relation that is also remote
/// (`service` set) is rejected at parse time -- an iterative cross-node
/// transitive closure is not supported. Confirms the guard the
/// policy schema already enforces (`policy::` tests pin the schema
/// layer directly); this is the fdae-crate-level confirmation that a
/// `Policy` value with that combination can never reach `plan_read` at
/// all, since `Policy` is only ever constructed via `parse_and_validate`.
#[test]
fn remote_and_recursive_on_the_same_relation_cannot_reach_plan_read() {
    let err = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "user": {
                    "table": "users",
                    "principal_column": "did",
                    "relations": {
                        "management_chain": {
                            "target": "user", "from_key": "id", "to_key": "manager_id",
                            "recursive": true, "service": "hr-svc"
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// Two OR'd permission paths that both reach the *same* remote relation
/// collapse into a single `RemoteFetch` (deduped by `(service,
/// relation)`) even though the marker text appears twice.
#[test]
fn plan_read_dedupes_repeated_fetches_to_the_same_remote_relation() {
    // Two *different* local relation names ("owner", "lead") both
    // pointing at the same remote (service, target) pair -- the dedupe
    // key is `(service, relation)` where `relation` is the remote
    // object type (`Relation.target`), not the local edge name,
    // so these collapse into one fetch even though `hop.name` differs.
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        },
                        "lead": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "lead_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["owner", "anchor"], ["lead", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert_eq!(
        plan.fetches.len(),
        1,
        "both paths resolve to the same (service, target) pair despite different local names"
    );
}

/// Two remote relations naming the *same* service but
/// *different* target types must **not** dedupe -- each needs its own
/// fetch and its own `IN (...)` predicate bound to its own id-set.
#[test]
fn plan_read_does_not_dedupe_fetches_to_different_remote_target_types() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        },
                        "department": {
                            "target": "team", "service": "hr-svc",
                            "join_column": "department_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["owner", "anchor"], ["department", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert_eq!(
        plan.fetches.len(),
        2,
        "same service, different target types ('employee' vs 'team') must stay distinct fetches"
    );
    let targets: BTreeSet<&str> = plan.fetches.iter().map(|f| f.relation.as_str()).collect();
    assert_eq!(targets, BTreeSet::from(["employee", "team"]));
}

/// `RemoteFetch.expected_asserter_did` is threaded from the policy's
/// `Relation.expected_asserter_did`, not left empty or derived.
#[test]
fn plan_read_carries_the_policys_expected_asserter_did_onto_the_fetch() {
    let policy = remote_relation_policy();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert_eq!(plan.fetches[0].expected_asserter_did, "did:key:zHrSvc");
}

/// Two hops naming the same remote (service, relation) but declaring
/// different `expected_asserter_did` values must fail closed rather than
/// silently keep whichever hop registered its fetch first -- a policy
/// author disagreeing with themselves about who is trusted to answer
/// for one remote type is a misconfiguration, not something to resolve
/// by picking one arbitrarily.
#[test]
fn plan_read_fails_closed_when_two_hops_disagree_on_expected_asserter_did() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvcOne"
                        },
                        "lead": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "lead_uuid",
                            "expected_asserter_did": "did:key:zHrSvcTwo"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["owner", "anchor"], ["lead", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let err = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// The same remote relation reached via two distinct local hop names
/// (`plan_read_dedupes_repeated_fetches_to_the_same_remote_relation`'s
/// shape) shares one fetch, so `finalize` must record its provenance
/// once, not once per occurrence.
#[test]
fn finalize_records_one_trace_entry_per_deduped_slot_not_per_occurrence() {
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "owner": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        },
                        "lead": {
                            "target": "employee", "service": "hr-svc",
                            "join_column": "lead_uuid",
                            "expected_asserter_did": "did:key:zHrSvc"
                        }
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["owner", "anchor"], ["lead", "anchor"]]
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_service =
        session_with_anchor("did:key:svc-1", "did:key:alice", vec![read_cap(Some("document"))]);
    let mut plan = plan_read(
        &policy,
        "document",
        &proxying_service,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert_eq!(plan.fetches.len(), 1, "owner/lead collapse to one deduped fetch");
    let slot = plan.fetches[0].slot;
    let pending = plan.pending.take().unwrap();
    let fetch_trace = RemoteFetchTrace {
        service: "hr-svc".to_string(),
        relation: "employee".to_string(),
        principal_did: "did:key:alice".to_string(),
        asserter_did: "did:key:zHrSvc".to_string(),
        valid_until_secs: 1_000,
    };
    let sieve = finalize(
        pending,
        &[FetchResult { slot, ids: vec!["emp-alice".to_string()], trace: fetch_trace.clone() }],
    )
    .unwrap();
    assert_eq!(sieve.trace.remote_fetches, vec![fetch_trace]);
}

fn resolvable_employee_policy(principal_col: &str) -> Policy {
    parse_and_validate(&format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "employee": {{
                    "table": "employees",
                    "principal_column": "{principal_col}",
                    "resolvable_without_capability": true
                }}
            }}
        }}"#
    ))
    .unwrap()
}

/// A definition opted into `resolvable_without_capability`
/// resolves via a bare `principal_column = ?` predicate, runnable
/// directly against the seeded table.
#[test]
fn resolve_structural_runs_correctly_against_a_json_payload_principal_column() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE employees (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO employees (id, payload) VALUES ('emp-1', ?1)",
        [json!({"did": "did:key:alice"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO employees (id, payload) VALUES ('emp-2', ?1)",
        [json!({"did": "did:key:bob"}).to_string()],
    )
    .unwrap();

    let policy = resolvable_employee_policy("did");
    let resolved = resolve_structural(&policy, "employee", "did:key:alice").unwrap().unwrap();
    assert_eq!(resolved.table, "employees");

    let sql = format!("SELECT id FROM {} WHERE {}", resolved.table, resolved.where_clause);
    let mut stmt = conn.prepare(&sql).unwrap();
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(resolved.params.iter()), |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(ids, vec!["emp-1"]);
}

/// A reserved `principal_column` (e.g. `creator_id`) resolves to the
/// physical column, not `json_extract(payload, '$.creator_id')` -- the
/// same reserved-column addressing every other predicate uses.
#[test]
fn resolve_structural_addresses_a_reserved_column_directly() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE employees (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute("INSERT INTO employees (id, creator_id) VALUES ('emp-1', 'did:key:alice')", [])
        .unwrap();

    let policy = resolvable_employee_policy("creator_id");
    let resolved = resolve_structural(&policy, "employee", "did:key:alice").unwrap().unwrap();
    assert_eq!(resolved.where_clause, "employees.creator_id = ?");

    let sql = format!("SELECT id FROM {} WHERE {}", resolved.table, resolved.where_clause);
    let mut stmt = conn.prepare(&sql).unwrap();
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(resolved.params.iter()), |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(ids, vec!["emp-1"]);
}

/// A definition that has *not* opted in resolves to `None`, not an
/// error -- the caller treats this identically to "not found," never
/// silently permitting structural resolution.
#[test]
fn resolve_structural_is_none_when_not_opted_in() {
    let policy = single_hop_policy();
    assert!(resolve_structural(&policy, "document", "did:key:alice").unwrap().is_none());
}

#[test]
fn resolve_structural_is_none_for_an_unknown_relation() {
    let policy = resolvable_employee_policy("did");
    assert!(resolve_structural(&policy, "nonexistent", "did:key:alice").unwrap().is_none());
}

/// `definition_table` resolves either the definition key or the
/// physical table name (case-insensitively) to the physical table -- the
/// native `resolve-relation` needs the *table*, since
/// `ServiceStore::query` addresses a collection literally, unlike
/// `compile_read`'s own permissive key-or-table matching.
#[test]
fn definition_table_resolves_by_key_or_table_case_insensitively() {
    let policy = resolvable_employee_policy("did");
    assert_eq!(definition_table(&policy, "employee"), Some("employees"));
    assert_eq!(definition_table(&policy, "EMPLOYEE"), Some("employees"));
    assert_eq!(definition_table(&policy, "employees"), Some("employees"));
    assert_eq!(definition_table(&policy, "nonexistent"), None);
}

#[test]
fn point_in_time_mode_ands_the_id_predicate_last() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");
    insert_document(&conn, "doc-2", "u-alice");

    let policy = single_hop_policy();
    let alice = session("did:key:alice", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &alice,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();

    assert_eq!(run_sieve(&conn, "documents", &sieve), vec!["doc-1"]);
}

#[test]
fn adversarial_subject_did_is_bound_not_interpolated() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_document(&conn, "doc-1", "u-alice");

    let policy = single_hop_policy();
    let attacker = session("attacker' OR '1'='1", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &attacker,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();

    // If this were string-interpolated, `OR '1'='1'` would make every
    // row visible. Bound as `?`, it is treated as an inert literal.
    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
    assert!(
        sieve.where_clause.contains('?'),
        "the DID must be a bound placeholder, not inlined text"
    );
}

#[test]
fn cls_masked_fields_union_policy_and_capability_deny_lists() {
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
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let cap = Capability {
        with: resource("user"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"fields": {"deny": ["salary"]}})),
    };
    let alice = session("did:key:alice", vec![cap]);
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
    assert_eq!(sieve.masked_fields, vec!["salary".to_string(), "ssn".to_string()]);
}

#[test]
fn where_caveats_are_collected_from_entitling_capabilities() {
    let policy = single_hop_policy();
    let cap = Capability {
        with: resource("document"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"where": {"region": "EU"}})),
    };
    let alice = session("did:key:alice", vec![cap]);
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
    assert_eq!(sieve.where_caveats, vec![json!({"region": "EU"})]);
}

#[test]
fn intersection_operator_requires_every_path_to_hold() {
    let conn = Connection::open_in_memory().unwrap();
    seed_schema(&conn);
    insert_user(&conn, "u-alice", "did:key:alice", None);
    insert_user(&conn, "u-bob", "did:key:bob", None);
    let insert_reviewed_doc = |id: &str, creator: &str, reviewer: &str| {
        let payload = json!({"creator_uuid": creator, "reviewer_uuid": reviewer});
        conn.execute(
            "INSERT INTO documents (id, payload) VALUES (?1, ?2)",
            (id, payload.to_string()),
        )
        .unwrap();
    };
    insert_reviewed_doc("doc-both", "u-alice", "u-alice");
    insert_reviewed_doc("doc-creator-only", "u-alice", "u-bob");
    insert_reviewed_doc("doc-reviewer-only", "u-bob", "u-alice");

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {
                        "creator": {"target": "user", "join_column": "creator_uuid"},
                        "reviewer": {"target": "user", "join_column": "reviewer_uuid"}
                    },
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "operator": "intersection",
                            "paths": [["creator", "caller"], ["reviewer", "caller"]]
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
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
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-both"],
        "intersection requires alice to be both creator and reviewer"
    );
}

#[test]
fn plain_two_hop_chain_prunes_through_both_joins() {
    // document -creator-> user -home_department-> department, with the
    // *department's owner* (not the creator) as the terminal -- a
    // non-recursive, non-fused 2-hop chain, distinct from the
    // recursive-fused case the other multi-hop test covers.
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
                            "paths": [["creator", "home_department", "caller"]]
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

    let carol = session("did:key:carol", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &carol,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        run_sieve(&conn, "documents", &sieve),
        vec!["doc-1"],
        "carol owns alice's home department, two joins away from the document"
    );

    let dave = session("did:key:dave", vec![read_cap(Some("document"))]);
    let sieve = compile_read(
        &policy,
        "document",
        &dave,
        SERVICE_ID,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap()
    .unwrap();
    assert!(run_sieve(&conn, "documents", &sieve).is_empty());
}

#[test]
fn default_fallback_still_carries_the_entitling_capabilitys_caveats() {
    // Regression test: when no permission is
    // directly/app-permission-applicable and access
    // comes only through `default`, the capability that satisfied
    // `holds_operation` must still contribute its caveats -- dropping
    // them would silently widen access beyond what the caveat allows.
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
    let caveat_cap = Capability {
        with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"where": {"region": "EU"}, "fields": {"deny": ["ssn"]}})),
    };
    let alice = session("did:key:alice", vec![caveat_cap]);
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
    assert_eq!(sieve.where_caveats, vec![json!({"region": "EU"})]);
    assert_eq!(sieve.masked_fields, vec!["ssn".to_string()]);
}

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

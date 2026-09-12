use super::*;

pub(super) fn remote_relation_policy() -> Policy {
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

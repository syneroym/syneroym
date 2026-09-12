use super::{plan_tests::remote_relation_policy, *};

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

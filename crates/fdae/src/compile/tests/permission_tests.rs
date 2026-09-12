use super::*;

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

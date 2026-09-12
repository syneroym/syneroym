use super::*;

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

use std::collections::BTreeMap;

use super::*;

fn minimal_doc(definitions: &str) -> String {
    format!(r#"{{"version": "fdae/v1", "definitions": {definitions}}}"#)
}

#[test]
fn parses_minimal_valid_policy() {
    let doc = minimal_doc(r#"{"user": {"table": "users", "principal_column": "did"}}"#);
    let policy = parse_and_validate(&doc).unwrap();
    assert_eq!(policy.version, "fdae/v1");
    assert!(!policy.strict);
    assert_eq!(policy.definitions.len(), 1);
    assert!(!policy.definitions["user"].resolvable_without_capability);
}

#[test]
fn parses_resolvable_without_capability_when_declared() {
    let doc = minimal_doc(
        r#"{"employee": {
            "table": "employees", "principal_column": "did",
            "resolvable_without_capability": true
        }}"#,
    );
    let policy = parse_and_validate(&doc).unwrap();
    assert!(policy.definitions["employee"].resolvable_without_capability);
}

#[test]
fn authorize_rows_defaults_to_false_when_absent() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "permissions": {
            "view": {"allows": ["data-layer/read"], "paths": []}
        }}}"#,
    );
    let policy = parse_and_validate(&doc).unwrap();
    assert!(!policy.definitions["document"].permissions["view"].authorize_rows);
}

#[test]
fn accepts_authorize_rows_when_declared() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "permissions": {
            "view": {"allows": ["data-layer/read"], "paths": [], "authorize_rows": true}
        }}}"#,
    );
    let policy = parse_and_validate(&doc).unwrap();
    assert!(policy.definitions["document"].permissions["view"].authorize_rows);
}

#[test]
fn rejects_an_unknown_permission_key() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "permissions": {
            "view": {"allows": ["data-layer/read"], "paths": [], "authorise_rows": true}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_wrong_version_at_schema_stage() {
    let doc = minimal_doc(r#"{"user": {"table": "users"}}"#).replace("fdae/v1", "fdae/v2");
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_unknown_top_level_field_via_schema() {
    let doc = r#"{"version": "fdae/v1", "definitions": {}, "bogus": true}"#;
    let err = parse_and_validate(doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_relation_with_no_shape() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"creator": {"target": "user"}}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_remote_relation_with_join_column() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"creator": {
            "target": "user", "join_column": "creator_uuid", "service": "hr-svc",
            "expected_asserter_did": "did:key:zHrSvc"
        }}}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_remote_relation_missing_expected_asserter_did() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"owner": {
            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
        }}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_relation_with_join_and_recursive_shapes() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "relations": {
            "management_chain": {
                "target": "user", "join_column": "manager_id", "from_key": "id",
                "to_key": "manager_id", "recursive": true
            }
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_recursive_relation_that_is_also_remote() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "relations": {
            "management_chain": {
                "target": "user", "from_key": "id", "to_key": "manager_id",
                "recursive": true, "service": "hr-svc"
            }
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_recursive_shape_missing_recursive_flag() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "relations": {
            "management_chain": {"target": "user", "from_key": "id", "to_key": "manager_id"}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_relation_target_not_a_definition() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"creator": {
            "target": "nobody", "join_column": "creator_uuid"
        }}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_remote_relation_target_unresolved_locally() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"owner": {
            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
            "expected_asserter_did": "did:key:zHrSvc"
        }}}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_remote_relation_missing_join_column() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"owner": {
            "target": "employee", "service": "hr-svc"
        }}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// A `caller` terminal on a path whose last hop is remote is a
/// parse-time error, not a silent substitution of `anchor` --
/// `compile::emit_remote_terminal` unconditionally binds the anchor
/// regardless of the declared terminal word, so accepting `caller` here
/// would let a policy author write one thing and get another
/// (`anchor` is the *broader* principal in any proxied chain).
#[test]
fn rejects_a_caller_terminal_on_a_remote_relation_path() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"owner": {
            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
            "expected_asserter_did": "did:key:zHrSvc"
        }}, "permissions": {"view": {
            "allows": ["data-layer/read"], "paths": [["owner", "caller"]]
        }}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

/// The mirror: `anchor` on the same shape is accepted.
#[test]
fn accepts_an_anchor_terminal_on_a_remote_relation_path() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"owner": {
            "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
            "expected_asserter_did": "did:key:zHrSvc"
        }}, "permissions": {"view": {
            "allows": ["data-layer/read"], "paths": [["owner", "anchor"]]
        }}}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_path_terminal_target_missing_principal_column() {
    let doc = minimal_doc(
        r#"{
            "document": {"table": "documents", "relations": {"creator": {
                "target": "user", "join_column": "creator_uuid"
            }}, "permissions": {"view": {"paths": [["creator", "caller"]]}}},
            "user": {"table": "users"}
        }"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_zero_hop_path_on_a_self_principal_type() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view_self": {"paths": [["caller"]]}
        }}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_path_with_unknown_relation() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "permissions": {
            "view": {"paths": [["nonexistent", "caller"]]}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_path_with_unknown_terminal() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view": {"paths": [["nobody"]]}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_anchor_terminal_at_parse_time() {
    // `anchor` is a syntactically valid terminal at parse time
    // regardless of compile-time support -- compile.rs is where it
    // resolves to a bound value.
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view": {"paths": [["anchor"]]}
        }}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_includes_naming_unknown_permission() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view": {"paths": [["caller"]], "includes": ["ghost"]}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_cyclic_includes() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "a": {"paths": [["caller"]], "includes": ["b"]},
            "b": {"paths": [["caller"]], "includes": ["a"]}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_default_naming_unknown_permission() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "default": "ghost",
            "permissions": {"view": {"paths": [["caller"]]}}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_the_adr_worked_example_shape() {
    let doc = minimal_doc(
        r#"{
            "document": {
                "table": "documents",
                "relations": {
                    "creator": {"target": "user", "join_column": "creator_uuid"},
                    "parent_dept": {"target": "department", "join_column": "owner_dept_id"}
                },
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "operator": "union",
                        "paths": [["creator", "caller"], ["creator", "management_chain", "caller"]]
                    },
                    "manage": {
                        "allows": ["data-layer/read", "data-layer/write", "rpc/move"],
                        "includes": ["view"],
                        "paths": [["creator", "caller"]]
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
            },
            "department": {"table": "departments", "principal_column": "owner_did"}
        }"#,
    );
    let policy = parse_and_validate(&doc).unwrap();
    assert_eq!(policy.definitions.len(), 3);
}

#[test]
fn rejects_fields_allow_since_this_slice_does_not_enforce_it() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view_self": {"paths": [["caller"]], "fields": {"allow": ["name"]}}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_fields_deny_with_a_dotted_nested_path() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view_self": {"paths": [["caller"]], "fields": {"deny": ["profile.ssn"]}}
        }}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn accepts_fields_deny_without_allow() {
    let doc = minimal_doc(
        r#"{"user": {"table": "users", "principal_column": "did", "permissions": {
            "view_self": {"paths": [["caller"]], "fields": {"deny": ["ssn"]}}
        }}}"#,
    );
    parse_and_validate(&doc).unwrap();
}

#[test]
fn rejects_a_table_name_that_is_not_a_safe_sql_identifier() {
    let doc = minimal_doc(r#"{"user": {"table": "users'; DROP TABLE users; --"}}"#);
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_a_join_column_that_is_not_a_safe_sql_identifier() {
    let doc = minimal_doc(
        r#"{"document": {"table": "documents", "relations": {"creator": {
            "target": "user", "join_column": "creator_uuid') OR ('1'='1"
        }}}}"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_a_definitions_table_colliding_with_another_definitions_key() {
    let doc = minimal_doc(
        r#"{
            "orders": {"table": "orders_tbl"},
            "shipments": {"table": "orders"}
        }"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_a_definitions_table_colliding_case_insensitively() {
    let doc = minimal_doc(
        r#"{
            "orders": {"table": "orders_tbl"},
            "shipments": {"table": "ORDERS_TBL"}
        }"#,
    );
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

#[test]
fn rejects_a_path_exceeding_the_max_hop_count_via_schema() {
    let hops: Vec<String> = (0..40).map(|i| format!("\"hop{i}\"")).collect();
    let path = format!("[{}, \"caller\"]", hops.join(", "));
    let doc = minimal_doc(&format!(
        r#"{{"user": {{"table": "users", "principal_column": "did", "permissions": {{
            "view": {{"paths": [{path}]}}
        }}}}}}"#
    ));
    let err = parse_and_validate(&doc).unwrap_err();
    assert!(matches!(err, PolicyError::Schema(_)));
}

#[test]
fn rejects_a_path_exceeding_the_max_hop_count_at_the_semantic_layer_too() {
    // Defense in depth: even a `Policy` constructed directly --
    // bypassing `parse_and_validate`'s schema gate entirely, which
    // nothing stops a caller from doing since every field here is
    // `pub` -- must still be caught by `validate_semantics`'s own
    // hop-count check, not just the schema's `maxItems`.
    let mut path: Vec<String> = (0..=MAX_PATH_HOPS).map(|i| format!("hop{i}")).collect();
    path.push("caller".to_string());
    let mut permissions = BTreeMap::new();
    permissions.insert(
        "view".to_string(),
        Permission {
            allows: vec![],
            operator: Operator::default(),
            paths: vec![path],
            conditions: vec![],
            includes: vec![],
            fields: None,
            authorize_rows: false,
        },
    );
    let mut definitions = BTreeMap::new();
    definitions.insert(
        "user".to_string(),
        Definition {
            table: "users".to_string(),
            principal_column: Some("did".to_string()),
            relations: BTreeMap::new(),
            permissions,
            default: None,
            resolvable_without_capability: false,
        },
    );
    let policy = Policy { version: "fdae/v1".to_string(), strict: false, definitions };
    let err = validate_semantics(&policy).unwrap_err();
    assert!(matches!(err, PolicyError::Semantic(_)));
}

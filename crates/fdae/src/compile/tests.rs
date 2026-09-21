use std::collections::BTreeSet;

use rusqlite::Connection;
use serde_json::{Map, json};
use syneroym_ucan::{Ability, Capability, ResourceUri, ResourceUri as Uri, SessionContext};

use super::{types::MAX_FETCH_IDS, *};
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

mod emit_tests;
mod finalize_tests;
mod permission_tests;
mod plan_tests;
mod rebac_tests;
mod trace_tests;

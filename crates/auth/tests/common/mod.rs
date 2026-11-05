#![allow(dead_code, clippy::unwrap_used)]

use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use syneroym_app_host::types::http::HttpRequest;
use syneroym_auth::AuthService;
use syneroym_core::dht_registry::{MasterAnchorPayload, MasterAnchorResolver};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{AuthLevel, CallerContext};

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug)]
pub struct MockAnchorResolver {
    pub resolvable: bool,
    pub revoked: Vec<String>,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for MockAnchorResolver {
    async fn resolve_master_anchor(&self, master_id: &str) -> anyhow::Result<MasterAnchorPayload> {
        if !self.resolvable {
            return Err(anyhow::anyhow!("DHT anchor lookup failed for {master_id}"));
        }
        Ok(MasterAnchorPayload {
            schema: "master_anchor_v1".to_string(),
            revoked_keys: self.revoked.clone(),
            revoke_list_registry: None,
            timestamp: now_secs(),
        })
    }
}

/// An auth service whose `local` login reads `dir`, which holds `alice.key`.
/// Returns Alice's DID too.
pub fn service_with_identities(dir: &Path) -> (AuthService, String) {
    let alice = Identity::generate().unwrap();
    fs::write(dir.join("alice.key"), alice.to_bytes()).unwrap();
    let node_did = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let resolver = Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![] });
    let service = AuthService::new(
        Identity::generate().unwrap(),
        node_did,
        3600,
        60,
        Some(dir.to_path_buf()),
        resolver,
    );
    (service, substrate::derive_did_key(&alice.public_key()))
}

/// The caller the router builds for a stream from this node's own gateway.
pub fn node_caller(service: &AuthService) -> CallerContext {
    CallerContext::verified(service.node_did(), AuthLevel::Delegated, vec![])
}

/// A `POST /_syneroym/session/login` request.
pub fn login_request(headers: Vec<(String, String)>, body: &Value) -> HttpRequest {
    HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/login".to_string(),
        query: String::new(),
        route: "/_syneroym/session/login".to_string(),
        path_params: vec![],
        headers,
        body: serde_json::to_vec(body).unwrap(),
        caller: None,
    }
}

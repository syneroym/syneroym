use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::dht_registry::MasterAnchorPayload;

/// Answers from a fixed table of revoked keys; an issuer that is not in
/// the table fails to resolve. Counts the lookups.
#[derive(Debug, Default)]
struct TableResolver {
    revoked: HashMap<String, Vec<String>>,
    calls: AtomicUsize,
}

impl TableResolver {
    fn revoking(issuer: &str, audience: &str) -> Self {
        let mut revoked = HashMap::new();
        revoked.insert(issuer.to_string(), vec![audience.to_string()]);
        Self { revoked, calls: AtomicUsize::new(0) }
    }
}

#[async_trait::async_trait]
impl MasterAnchorResolver for TableResolver {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.revoked.get(master_id) {
            Some(keys) => {
                Ok(MasterAnchorPayload { revoked_keys: keys.clone(), ..Default::default() })
            }
            None => Err(anyhow::anyhow!("no anchor for {master_id}")),
        }
    }
}

#[tokio::test]
async fn a_revoked_first_edge_fails_the_chain() {
    let resolver = TableResolver::revoking("owner", "member");
    assert!(!chain_edges_not_revoked(&[("owner", "member"), ("root", "owner")], &resolver).await);
}

#[tokio::test]
async fn a_revoked_second_edge_fails_the_chain() {
    let resolver = TableResolver::revoking("root", "owner");
    assert!(!chain_edges_not_revoked(&[("owner", "member"), ("root", "owner")], &resolver).await);
}

#[tokio::test]
async fn an_unrevoked_chain_passes() {
    let resolver = TableResolver::revoking("owner", "someone-else");
    assert!(chain_edges_not_revoked(&[("owner", "member"), ("root", "owner")], &resolver).await);
}

#[tokio::test]
async fn a_repeated_edge_is_looked_up_once() {
    let resolver = TableResolver::default();
    let edges = [("owner", "member"), ("root", "owner"), ("owner", "member")];
    assert!(chain_edges_not_revoked(&edges, &resolver).await);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_failed_lookup_counts_as_not_revoked() {
    let resolver = TableResolver::default();
    assert!(chain_edges_not_revoked(&[("owner", "member")], &resolver).await);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

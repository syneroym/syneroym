//! The one `TopologyFetcher` implementation (ADR-0022 §3): Tier 1 then
//! Tier 2 over the real network. Verification is deliberately not done
//! here -- `register_verified` is the only place a document is trusted, so
//! a fetcher can never become the trust boundary.

use std::time::Duration;

use anyhow::{Context, Result};
use syneroym_app_orchestration::{
    AppDid, LogicalResolver, LogicalServiceName, SignedTopologyDocument, TopologyFetcher,
    TopologyKey, register_verified,
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::Identity;
use syneroym_rpc::CapabilityToken;

use crate::SyneroymClient;

/// Tier 1 → Tier 2 → a verified document, over the real network.
///
/// Holds a registry URL rather than a `RegistryClient` so each fetch is
/// independent; a supervisor connection is opened per fetch and dropped,
/// the same one-shot shape `LiveQueueConnector` uses.
#[derive(Debug)]
pub struct RegistryTopologyFetcher {
    registry_url: String,
    connect_timeout: Duration,
    /// Presented on the supervisor connection -- `resolve` is authorized
    /// (ADR-0022 §5), so a fetch without one only works for the node
    /// owner.
    caller_ucan: Option<CapabilityToken>,
    /// Raw key bytes, not an `Identity`: `Identity` deliberately does not
    /// implement `Clone` (see `SupervisorService::client_identity_bytes`'s
    /// own doc for why), so a fresh `Identity` is reconstructed per fetch
    /// from these bytes rather than held directly.
    identity_bytes: Option<[u8; 32]>,
}

impl RegistryTopologyFetcher {
    #[must_use]
    pub fn new(registry_url: String) -> Self {
        Self {
            registry_url,
            connect_timeout: Duration::from_secs(10),
            caller_ucan: None,
            identity_bytes: None,
        }
    }

    #[must_use]
    pub fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    #[must_use]
    pub fn with_ucan(mut self, caller_ucan: CapabilityToken) -> Self {
        self.caller_ucan = Some(caller_ucan);
        self
    }

    #[must_use]
    pub fn with_identity(mut self, identity: &Identity) -> Self {
        self.identity_bytes = Some(identity.to_bytes());
        self
    }
}

#[async_trait::async_trait]
impl TopologyFetcher for RegistryTopologyFetcher {
    async fn fetch(
        &self,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        // Tier 1: the app DID resolves to the substrate supervising it.
        // Self-signed under the app DID -- no other trust input.
        let registry = RegistryClient::new(false, Some(self.registry_url.clone()));
        let tier1 = registry
            .lookup(app_did.as_str(), false)
            .await
            .with_context(|| format!("Tier 1 lookup for app DID '{app_did}' failed"))?;
        tier1.verify().context("Tier 1 record failed to verify against its own app DID")?;
        let supervisor_did = tier1.info.substrate_id.clone();

        // Tier 3 for the supervisor itself: `SyneroymClient::connect` does
        // the second lookup and picks a mechanism.
        let identity = self.identity_bytes.map_or_else(
            || Identity::generate().context("generating an ephemeral identity"),
            |b| Ok(Identity::from_bytes(&b)),
        )?;
        let mut client =
            SyneroymClient::new_with_identity(supervisor_did, self.registry_url.clone(), identity)
                .with_connect_timeout(self.connect_timeout);
        if let Some(ucan) = &self.caller_ucan {
            client = client.with_ucan(ucan.clone());
        }
        client
            .wait_for_ready(self.connect_timeout)
            .await
            .context("connecting to the supervisor for a Tier-2 resolve")?;
        let resp = client
            .request(
                "supervisor",
                "resolve",
                serde_json::json!([app_did.as_str(), service_name.as_str()]),
            )
            .await
            .context("supervisor resolve call failed");
        let _ = client.shutdown().await;
        let resp = resp?;
        serde_json::from_value(resp.result).context("decoding the signed topology document")
    }
}

/// Verify, convert, and register a fetched document in one call -- the
/// whole client-side path ADR-0022 §3 describes. The suggested TTL travels
/// inside the signed document itself, so nothing has to be carried
/// alongside it.
pub async fn fetch_and_register(
    fetcher: &dyn TopologyFetcher,
    resolver: &LogicalResolver,
    app_did: &AppDid,
    service_name: &LogicalServiceName,
) -> Result<TopologyKey> {
    let signed = fetcher.fetch(app_did, service_name).await?;
    register_verified(resolver, &signed, app_did, None)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use syneroym_app_orchestration::{
        AppInstanceId, ServiceId, StaticInventory, TopologyDocument, TopologyEpoch, TopologyMode,
    };
    use syneroym_identity::substrate;

    use super::*;

    #[derive(Debug)]
    struct CountingFetcher {
        calls: AtomicUsize,
        signed: SignedTopologyDocument,
    }

    #[async_trait::async_trait]
    impl TopologyFetcher for CountingFetcher {
        async fn fetch(
            &self,
            _app_did: &AppDid,
            _service_name: &LogicalServiceName,
        ) -> Result<SignedTopologyDocument> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.signed.clone())
        }
    }

    /// The two performance budgets this exercises, measured directly
    /// rather than as timings. One `fetch_and_register`, then N `resolve`
    /// calls, asserts `fetch_calls
    /// == 1` (budget 1: "resolution after the first fetch -- no network
    /// call") -- `register_verified` (the only caller of `verify`) having
    /// run exactly once by construction here covers budget 3 ("verify
    /// once per fetch, not once per resolve").
    #[tokio::test]
    async fn one_fetch_and_register_serves_every_later_resolve() {
        let master = Identity::generate().unwrap();
        let app_did = AppDid::new(substrate::derive_did_key(&master.public_key()));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let doc = TopologyDocument {
            app_instance_id: AppInstanceId::new("inst-1"),
            app_did: app_did.clone(),
            service_name: LogicalServiceName::new("backend"),
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            generation: 0,
            issued_at: now,
            not_after: now + 3600,
            cache_ttl_ms: 60_000,
        };
        let signed = doc.sign(&master).unwrap();
        let fetcher = CountingFetcher { calls: AtomicUsize::new(0), signed };

        let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
        let key =
            fetch_and_register(&fetcher, &resolver, &app_did, &LogicalServiceName::new("backend"))
                .await
                .unwrap();

        for _ in 0..5 {
            assert!(resolver.resolve(&key, None).is_ok());
        }
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            1,
            "budget 1: no network call after the first fetch"
        );
    }
}

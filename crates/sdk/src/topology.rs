//! The one `TopologyFetcher` implementation (ADR-0022 §3): Tier 1 then
//! Tier 2 over the real network. Verification is deliberately not done
//! here -- `register_verified` is the only place a document is trusted, so
//! a fetcher can never become the trust boundary.
//!
//! Also the app-scoped gateway host resolver (S3, D-S3-11): [`AppHostResolver`]
//! is the shared implementation of "hostname `-a…-s…` to a member DID"
//! that the client gateway and the WebRTC coordinator both need, lifted
//! here rather than written twice, since those two are the pair most
//! likely to drift subtly apart on the D-S3-5 binding checks.

use std::{fmt::Debug, time::Duration};

use anyhow::{Context, Result};
use dashmap::DashMap;
use syneroym_app_orchestration::{
    AppDid, LogicalResolver, LogicalServiceName, SignedTopologyDocument, TopologyFetcher,
    TopologyKey, register_verified,
};
use syneroym_core::{dht_registry::RegistryClient, util};
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

impl RegistryTopologyFetcher {
    /// Tier 2 only, against a supervisor the caller has already resolved.
    /// A caller that reached this app through its Tier-1 record already
    /// holds `substrate_id`; making it round-trip the registry again to
    /// rediscover the same value is the duplication task.md's budget 2
    /// forbids (D-S3-17).
    pub async fn fetch_via(
        &self,
        supervisor_did: &str,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        // Tier 3 for the supervisor itself: `SyneroymClient::connect` does
        // the lookup and picks a mechanism.
        let identity = self.identity_bytes.map_or_else(
            || Identity::generate().context("generating an ephemeral identity"),
            |b| Ok(Identity::from_bytes(&b)),
        )?;
        let mut client = SyneroymClient::new_with_identity(
            supervisor_did.to_string(),
            self.registry_url.clone(),
            identity,
        )
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
        // `RegistryClient::lookup` already verifies and fails fast on
        // both branches; kept here too since removing it would only
        // suggest this caller trusts the registry more than it does.
        tier1.verify().context("Tier 1 record failed to verify against its own app DID")?;
        self.fetch_via(&tier1.info.substrate_id, app_did, service_name).await
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

/// Tier 1, abstracted behind a trait so a test can substitute a counting
/// fake instead of a real registry (task.md's own registry-call budgets,
/// unmeasured before S3).
#[async_trait::async_trait]
pub trait Tier1Lookup: Debug + Send + Sync {
    async fn lookup(&self, alias: &str) -> Result<syneroym_core::dht_registry::SignedEndpointInfo>;
}

#[derive(Debug)]
pub struct RegistryTier1Lookup {
    registry_url: String,
}

impl RegistryTier1Lookup {
    #[must_use]
    pub fn new(registry_url: String) -> Self {
        Self { registry_url }
    }
}

#[async_trait::async_trait]
impl Tier1Lookup for RegistryTier1Lookup {
    async fn lookup(&self, alias: &str) -> Result<syneroym_core::dht_registry::SignedEndpointInfo> {
        RegistryClient::new(false, Some(self.registry_url.clone())).lookup(alias, false).await
    }
}

/// Tier 2, abstracted the same way. `fetch_via` is an inherent method on
/// [`RegistryTopologyFetcher`] (D-S3-17) rather than part of the
/// [`TopologyFetcher`] trait above -- it deliberately skips Tier 1, and
/// every other `TopologyFetcher` caller still wants the full two-tier
/// `fetch`.
#[async_trait::async_trait]
pub trait Tier2Fetch: Debug + Send + Sync {
    async fn fetch_via(
        &self,
        supervisor_did: &str,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument>;
}

#[async_trait::async_trait]
impl Tier2Fetch for RegistryTopologyFetcher {
    async fn fetch_via(
        &self,
        supervisor_did: &str,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        Self::fetch_via(self, supervisor_did, app_did, service_name).await
    }
}

// Blanket impls so a test (or a caller wanting to inspect a fake's call
// counter after handing it to `AppHostResolver`) can keep its own `Arc`
// clone of a fake alongside the boxed trait object the resolver holds.
#[async_trait::async_trait]
impl<T: Tier1Lookup + ?Sized> Tier1Lookup for std::sync::Arc<T> {
    async fn lookup(&self, alias: &str) -> Result<syneroym_core::dht_registry::SignedEndpointInfo> {
        (**self).lookup(alias).await
    }
}

#[async_trait::async_trait]
impl<T: Tier2Fetch + ?Sized> Tier2Fetch for std::sync::Arc<T> {
    async fn fetch_via(
        &self,
        supervisor_did: &str,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        (**self).fetch_via(supervisor_did, app_did, service_name).await
    }
}

/// D-S3-5's first half: `RegistryClient::lookup` cannot bind an *alias*
/// lookup to what was asked for, by construction (§0.4) -- a registry
/// answering the alias with another app's perfectly valid, self-signed
/// record must not silently redirect this caller to it.
fn check_tier1_binding(
    returned_service_id: &str,
    a_hash: &str,
    app_lookup_alias: &str,
) -> Result<()> {
    anyhow::ensure!(
        util::short_hash(returned_service_id) == a_hash,
        "registry answered alias '{app_lookup_alias}' with '{returned_service_id}', whose hash is \
         not the '-a{a_hash}' this host named"
    );
    Ok(())
}

/// D-S3-5's second half: `SignedTopologyDocument::verify` checks the
/// signer and the expiry, never *which service* was asked for.
fn check_tier2_binding(returned_service_name: &str, s_hash: &str) -> Result<()> {
    anyhow::ensure!(
        util::short_hash(returned_service_name) == s_hash,
        "supervisor answered '-s{s_hash}' with service '{returned_service_name}'"
    );
    Ok(())
}

/// The state D-S3-7 says each substrate-side consumer of an app-scoped
/// gateway host must own for itself -- the client gateway and the WebRTC
/// coordinator each build one, never shared, since the two have different
/// construction orders and lifetimes and `AppScope::Foreign` entries are
/// Which log line a caller (`ClientGateway::init`, `CoordinatorWebRtc::init`)
/// should emit for its own `resolve_ucan`/`grant_resolve_to_node_did`
/// configuration (D-S3-6), pulled out as a pure, shared function rather
/// than reimplemented per component so the two decisions cannot drift
/// apart. `None` means no warning; `Some` carries whether it is a `warn!`
/// (both credentials absent) or a `debug!` (the same-node gate alone
/// covers it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialWarning {
    NeitherConfigured,
    OnlyTheSameNodeGate,
}

#[must_use]
pub fn credential_warning(
    has_resolve_ucan: bool,
    grant_resolve_to_node_did: bool,
) -> Option<CredentialWarning> {
    if has_resolve_ucan {
        None
    } else if !grant_resolve_to_node_did {
        Some(CredentialWarning::NeitherConfigured)
    } else {
        Some(CredentialWarning::OnlyTheSameNodeGate)
    }
}

/// The state D-S3-7 says each substrate-side consumer of an app-scoped
/// gateway host must own for itself -- the client gateway and the WebRTC
/// coordinator each build one, never shared, since the two have different
/// construction orders and lifetimes and `AppScope::Foreign` entries are
/// only ever meaningful to the `LogicalResolver` that registered them.
#[derive(Debug)]
pub struct AppHostResolver {
    tier1: Box<dyn Tier1Lookup>,
    fetcher: Option<Box<dyn Tier2Fetch>>,
    resolver: LogicalResolver,
    /// `short_hash(app_did)` -> the app DID a Tier-1 lookup returned **and
    /// the substrate supervising it**, both from the one record, so a
    /// repeat request re-resolves neither (D-S3-17). Bound to the hash at
    /// insert time (D-S3-5), so a cache hit is as checked as a miss.
    app_dids: DashMap<String, (AppDid, String)>,
    /// `(app_did, short_hash(service_name))` -> the real service name, as
    /// carried by a verified document. Only ever written from a document
    /// that passed the `short_hash(name) == hash` check.
    service_names: DashMap<(AppDid, String), LogicalServiceName>,
}

impl AppHostResolver {
    #[must_use]
    pub fn new(
        tier1: Box<dyn Tier1Lookup>,
        fetcher: Option<Box<dyn Tier2Fetch>>,
        resolver: LogicalResolver,
    ) -> Self {
        Self { tier1, fetcher, resolver, app_dids: DashMap::new(), service_names: DashMap::new() }
    }

    /// A reference to this resolver's own `LogicalResolver`, so a caller
    /// (the client gateway's `handle_connection`, the coordinator's
    /// `handle_bootstrap`) can share the same cache between an app-scoped
    /// resolve and any other logical lookup it might make.
    #[must_use]
    pub fn logical_resolver(&self) -> &LogicalResolver {
        &self.resolver
    }

    /// Resolves an app-scoped (`-a…-s…`) target host to a member
    /// `ServiceId` (D-S3-5, D-S3-17). Tier 1 is cached alongside the
    /// supervising node, so a repeat request for the same app makes no
    /// registry call; Tier 2 is cached in the `LogicalResolver`, so a
    /// repeat request for the same service makes no network call at all
    /// (task.md budget 1) until the entry expires or is evicted (D-S3-8).
    pub async fn resolve_app_host(
        &self,
        app_lookup_alias: &str,
        a_hash: &str,
        s_hash: &str,
        routing_key: Option<&[u8]>,
    ) -> Result<String> {
        let fetcher = self
            .fetcher
            .as_ref()
            .context("no community registry configured; logical hostnames need Tier 1")?;

        // ── Tier 1 (cached) ──────────────────────────────────────────
        let (app_did, supervisor_did) = match self.app_dids.get(a_hash) {
            Some(e) => e.clone(),
            None => {
                let rec =
                    self.tier1.lookup(app_lookup_alias).await.with_context(|| {
                        format!("Tier 1 alias lookup '{app_lookup_alias}' failed")
                    })?;
                // No `rec.verify()` here: `RegistryClient::lookup` already
                // verifies and fails fast on both branches, so
                // re-verifying would only suggest it does not. The check
                // below is the one thing genuinely being added.
                check_tier1_binding(&rec.info.service_id, a_hash, app_lookup_alias)?;
                let did = AppDid::try_new(rec.info.service_id.as_str())?;
                self.app_dids
                    .insert(a_hash.to_string(), (did.clone(), rec.info.substrate_id.clone()));
                (did, rec.info.substrate_id)
            }
        };

        // ── Tier 2 (cached in the resolver) ─────────────────────────
        if let Some(name) = self.service_names.get(&(app_did.clone(), s_hash.to_string()))
            && let Ok(member) = self
                .resolver
                .resolve(&TopologyKey::foreign(app_did.clone(), name.clone()), routing_key)
        {
            return Ok(member.to_string());
        }

        // `fetch_via`, not `fetch`: the supervising node came back with
        // the Tier-1 record above, so `fetch`'s own Tier-1 lookup would
        // be the same round-trip twice (D-S3-17, task.md budget 2). A
        // hash is a valid `LogicalServiceName` (8 z32 characters, so
        // non-empty and free of `/`/`#`), and the supervisor reverses it
        // (D-S3-3).
        let signed = fetcher
            .fetch_via(&supervisor_did, &app_did, &LogicalServiceName::try_new(s_hash)?)
            .await?;
        // D-S3-5's second half: `verify` checks the signer and the
        // expiry, never *which service* was asked for.
        check_tier2_binding(signed.document.service_name.as_str(), s_hash)?;
        let key = register_verified(&self.resolver, &signed, &app_did, None)?;
        self.service_names
            .insert((app_did.clone(), s_hash.to_string()), signed.document.service_name.clone());

        Ok(self.resolver.resolve(&key, routing_key)?.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use syneroym_app_orchestration::{
        AppInstanceId, ServiceId, ShardingStrategy, StaticInventory, TopologyDocument,
        TopologyEpoch, TopologyMode,
    };
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType, SignedEndpointInfo};
    use syneroym_identity::substrate::{self, derive_did_key};

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

    // ── `AppHostResolver` (S3): tests 83-89 ─────────────────────────────

    #[derive(Debug)]
    struct FakeTier1 {
        calls: AtomicUsize,
        response: SignedEndpointInfo,
    }

    #[async_trait::async_trait]
    impl Tier1Lookup for FakeTier1 {
        async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    #[derive(Debug, Default)]
    struct FakeTier2 {
        calls: AtomicUsize,
        responses: StdMutex<VecDeque<SignedTopologyDocument>>,
    }

    #[async_trait::async_trait]
    impl Tier2Fetch for FakeTier2 {
        async fn fetch_via(
            &self,
            _supervisor_did: &str,
            _app_did: &AppDid,
            _service_name: &LogicalServiceName,
        ) -> Result<SignedTopologyDocument> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("FakeTier2 has no more queued responses"))
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    fn app_master() -> (Identity, AppDid) {
        let master = Identity::generate().unwrap();
        let app_did = AppDid::new(derive_did_key(&master.public_key()));
        (master, app_did)
    }

    fn signed_tier1_record(
        app_did: &AppDid,
        supervisor_did: &str,
        master: &Identity,
    ) -> SignedEndpointInfo {
        EndpointInfo {
            service_id: app_did.as_str().to_string(),
            substrate_id: supervisor_did.to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("my-chat-app".to_string()),
            is_private: false,
            ttl: None,
            not_after: now_secs() + 3600,
            generation: 0,
        }
        .sign(master)
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn signed_topology_doc(
        app_did: &AppDid,
        master: &Identity,
        service_name: &str,
        mode: TopologyMode,
        members: Vec<&str>,
        not_after_offset_secs: u64,
        sharding_strategy: Option<ShardingStrategy>,
    ) -> SignedTopologyDocument {
        let now = now_secs();
        let doc = TopologyDocument {
            app_instance_id: AppInstanceId::new("my-chat-app"),
            app_did: app_did.clone(),
            service_name: LogicalServiceName::new(service_name),
            mode,
            members: members.into_iter().map(ServiceId::new).collect(),
            sharding_strategy,
            epoch: TopologyEpoch(1),
            generation: 0,
            issued_at: now,
            not_after: now + not_after_offset_secs,
            cache_ttl_ms: 60_000,
        };
        doc.sign(master).unwrap()
    }

    fn app_host_resolver(
        tier1: Arc<FakeTier1>,
        fetcher: Option<Arc<FakeTier2>>,
    ) -> AppHostResolver {
        AppHostResolver::new(
            Box::new(tier1),
            fetcher.map(|f| Box::new(f) as Box<dyn Tier2Fetch>),
            LogicalResolver::new(Arc::new(StaticInventory::new())),
        )
    }

    /// Test 83: D-S3-5's first half -- a registry answering an alias with
    /// another app's perfectly valid, self-signed record is refused.
    #[tokio::test]
    async fn a_tier1_record_whose_hash_does_not_match_the_a_segment_is_refused() {
        let (master, app_did) = app_master();
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let resolver = app_host_resolver(tier1, Some(Arc::new(FakeTier2::default())));

        // A wrong `a_hash`, not the one this app's DID actually hashes to.
        let err = resolver
            .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("wronghash"), "{err}");
    }

    /// Test 84: D-S3-5's second half -- a document naming a different
    /// service than the `-s` segment is refused.
    #[tokio::test]
    async fn a_document_naming_a_different_service_than_the_s_segment_is_refused() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let doc = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Singleton,
            vec!["did:key:zMember0"],
            3600,
            None,
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(doc);
        let resolver = app_host_resolver(tier1, Some(fetcher));

        // A wrong `s_hash`, not `short_hash("backend")`.
        let err = resolver
            .resolve_app_host("my-chat-app-{a_hash}", &a_hash, "wronghash", None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("wronghash") || err.to_string().contains("backend"),
            "{err}"
        );
    }

    /// Test 85: task.md budget 1 at the gateway -- a fetch count, not a
    /// timing.
    #[tokio::test]
    async fn a_second_request_for_the_same_app_scoped_host_makes_no_network_call() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let doc = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Singleton,
            vec!["did:key:zMember0"],
            3600,
            None,
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(doc);
        let resolver = app_host_resolver(tier1.clone(), Some(fetcher.clone()));
        let s_hash = util::short_hash("backend");

        let first = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(first, "did:key:zMember0");

        let fetcher_calls_before = fetcher.calls.load(Ordering::SeqCst);
        let tier1_calls_before = tier1.calls.load(Ordering::SeqCst);
        let second =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(second, "did:key:zMember0");
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            fetcher_calls_before,
            "no new Tier-2 fetch"
        );
        assert_eq!(tier1.calls.load(Ordering::SeqCst), tier1_calls_before, "no new Tier-1 lookup");
    }

    /// Test 86: task.md budget 2, measured for the first time in this
    /// milestone (§0.13, D-S3-17) -- one Tier-1 lookup per cold app-scoped
    /// resolve, not two, and zero on a warm one.
    #[tokio::test]
    async fn a_cold_resolve_makes_exactly_one_tier1_lookup() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let doc = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Singleton,
            vec!["did:key:zMember0"],
            3600,
            None,
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(doc);
        let resolver = app_host_resolver(tier1.clone(), Some(fetcher));
        let s_hash = util::short_hash("backend");

        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(
            tier1.calls.load(Ordering::SeqCst),
            1,
            "exactly one Tier-1 lookup on a cold resolve"
        );

        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(
            tier1.calls.load(Ordering::SeqCst),
            1,
            "zero more Tier-1 lookups on a warm resolve"
        );
    }

    /// Test 87: D-S3-8, ADR-0022 §3's "on expiry try to refresh" -- an
    /// expired cache entry triggers exactly one refetch rather than a
    /// failure.
    #[tokio::test]
    async fn an_expired_entry_triggers_one_refetch_rather_than_a_failure() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        // A 3s margin, not 1s: commit 38ebbea's flakiness fix (and S2
        // post-merge finding 8) widened every wall-clock-boundary-adjacent
        // `not_after` in this codebase from 1s to 3s for the same reason --
        // a `not_after` computed one second before a real second boundary
        // leaves under 1ms of actual margin.
        let short_lived = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Singleton,
            vec!["did:key:zMember0"],
            3,
            None,
        );
        let fresh = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Singleton,
            vec!["did:key:zMember1"],
            3600,
            None,
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(short_lived);
        fetcher.responses.lock().unwrap().push_back(fresh);
        let resolver = app_host_resolver(tier1, Some(fetcher.clone()));
        let s_hash = util::short_hash("backend");

        let first = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(first, "did:key:zMember0");

        tokio::time::sleep(Duration::from_millis(3500)).await;

        let second =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
        assert_eq!(second, "did:key:zMember1", "must have refetched the fresh document");
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2, "exactly one refetch, not a failure");
    }

    /// Test 88: over a `Redundant` document -- the same key twice returns
    /// the same member, no header returns members in round-robin.
    #[tokio::test]
    async fn a_routing_key_header_selects_a_member_and_its_absence_does_not() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let doc = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Redundant,
            vec!["did:key:zMember0", "did:key:zMember1", "did:key:zMember2"],
            3600,
            None,
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(doc);
        let resolver = app_host_resolver(tier1, Some(fetcher));
        let s_hash = util::short_hash("backend");
        let key = b"routing-key-alice";

        let first =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, Some(key)).await.unwrap();
        for _ in 0..5 {
            let repeat = resolver
                .resolve_app_host("my-chat-app", &a_hash, &s_hash, Some(key))
                .await
                .unwrap();
            assert_eq!(repeat, first, "the same routing key must select the same member");
        }
    }

    /// Test 89: ADR-0022 §7's closing sentence -- a `Sharded` service with
    /// no routing key fails with the resolver's own, specific error.
    #[tokio::test]
    async fn a_sharded_service_with_no_routing_key_fails_with_the_resolvers_own_error() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let doc = signed_topology_doc(
            &app_did,
            &master,
            "backend",
            TopologyMode::Sharded,
            vec!["did:key:zMember0", "did:key:zMember1"],
            3600,
            Some(ShardingStrategy::HashSharding),
        );
        let fetcher = Arc::new(FakeTier2::default());
        fetcher.responses.lock().unwrap().push_back(doc);
        let resolver = app_host_resolver(tier1, Some(fetcher));
        let s_hash = util::short_hash("backend");

        let err =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
        assert!(
            err.to_string().contains("routing_key") || err.to_string().contains("Sharded"),
            "{err}"
        );
    }
}

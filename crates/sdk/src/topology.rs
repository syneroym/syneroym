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

use std::{
    fmt::Debug,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use syneroym_app_orchestration::{
    AppDid, LogicalResolver, LogicalServiceName, SignedTopologyDocument, TopologyFetcher,
    TopologyKey, is_retryable_resolve_error, register_verified,
};
use syneroym_core::{dht_registry::RegistryClient, util};
use syneroym_identity::Identity;
use syneroym_rpc::CapabilityToken;
use tokio::sync::Mutex as AsyncMutex;

use crate::SyneroymClient;

/// How long a failed cold `AppHostResolver` resolve is remembered before
/// being retried (finding B1). Short, deliberately: this exists to blunt
/// a burst of duplicate requests against an unauthenticated, public
/// listener (the WebRTC bootstrap page resolves through the same
/// resolver, D-S3-11), not to hide a host that has genuinely started
/// resolving -- a longer window would mean the fix itself makes recovery
/// from a transient failure feel broken.
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(5);

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
    /// `app_lookup_alias` (the full `<nickname>-<a_hash>` alias, not the
    /// hash alone) -> the app DID a Tier-1 lookup returned **and the
    /// substrate supervising it**, both from the one record, so a repeat
    /// request re-resolves neither (D-S3-17). Keyed on the alias, not just
    /// `a_hash`, so a warm entry cannot answer a *different* nickname over
    /// the same hash without its own alias lookup -- `a_hash` alone would
    /// let the cache silently widen what the parser accepts (finding B3).
    /// Bound to the hash at insert time regardless (D-S3-5), so a cache hit
    /// is as checked as a miss.
    app_dids: DashMap<String, (AppDid, String)>,
    /// `(app_did, short_hash(service_name))` -> the real service name, as
    /// carried by a verified document. Only ever written from a document
    /// that passed the `short_hash(name) == hash` check.
    service_names: DashMap<(AppDid, String), LogicalServiceName>,
    /// One lock per `(app_lookup_alias, s_hash)` cold fetch currently in
    /// flight (finding B2): concurrent callers for the same not-yet-cached
    /// host share one Tier-1-then-Tier-2 round trip rather than each
    /// starting an independent one. Removed once that fetch completes --
    /// the *outcome* is what stays cached (`app_dids`/`service_names` on
    /// success, `negative_cache` on failure), never the lock itself.
    inflight: DashMap<(String, String), Arc<AsyncMutex<()>>>,
    /// A cold fetch's most recent failure, remembered for
    /// `NEGATIVE_CACHE_TTL` (finding B1).
    negative_cache: DashMap<(String, String), (String, Instant)>,
}

impl AppHostResolver {
    #[must_use]
    pub fn new(
        tier1: Box<dyn Tier1Lookup>,
        fetcher: Option<Box<dyn Tier2Fetch>>,
        resolver: LogicalResolver,
    ) -> Self {
        Self {
            tier1,
            fetcher,
            resolver,
            app_dids: DashMap::new(),
            service_names: DashMap::new(),
            inflight: DashMap::new(),
            negative_cache: DashMap::new(),
        }
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
    /// A cold resolve is single-flighted and a failure briefly remembered
    /// (findings B1, B2) -- see [`Self::ensure_populated`].
    pub async fn resolve_app_host(
        &self,
        app_lookup_alias: &str,
        a_hash: &str,
        s_hash: &str,
        routing_key: Option<&[u8]>,
    ) -> Result<String> {
        // The lock-free warm path: both tiers already cached, no network,
        // no `inflight`/`negative_cache` bookkeeping at all.
        if let Some(key) = self.cached_key(app_lookup_alias, s_hash) {
            match self.resolver.resolve(&key, routing_key) {
                Ok(member) => return Ok(member.to_string()),
                // A permanent selection failure -- `Sharded` called with no
                // routing key, an empty member set -- is not a cache miss:
                // the cached document is warm and correct, and re-fetching
                // the identical document changes nothing. Surfacing it
                // directly is what keeps budget 1 ("no network call after
                // the first fetch") true once a caller is stuck in one of
                // these permanent states, rather than refetching Tier 2 on
                // every single request (finding A3).
                Err(e) if !is_retryable_resolve_error(&e) => return Err(e),
                // Not registered, or past `not_after`: fall through and
                // refetch (D-S3-8).
                Err(_) => {}
            }
        }

        let key = self.ensure_populated(app_lookup_alias, a_hash, s_hash).await?;
        Ok(self.resolver.resolve(&key, routing_key)?.to_string())
    }

    /// Both tiers already cached, read with no lock and no network --
    /// `None` on either miss, never a partial answer.
    fn cached_key(&self, app_lookup_alias: &str, s_hash: &str) -> Option<TopologyKey> {
        let (app_did, _) = self.app_dids.get(app_lookup_alias)?.clone();
        let name = self.service_names.get(&(app_did.clone(), s_hash.to_string()))?.clone();
        Some(TopologyKey::foreign(app_did, name))
    }

    /// A fresh (within `NEGATIVE_CACHE_TTL`) remembered failure for
    /// `key`, if there is one.
    fn fresh_negative(&self, key: &(String, String)) -> Option<String> {
        let entry = self.negative_cache.get(key)?;
        let (message, at) = entry.value();
        (at.elapsed() < NEGATIVE_CACHE_TTL).then(|| message.clone())
    }

    /// Ensures Tier 1 and Tier 2 are populated for `(app_lookup_alias,
    /// s_hash)`, fetching over the network **at most once** across every
    /// concurrent caller for the same cold key (finding B2) -- a
    /// `tokio::sync::Mutex` per key, held across the fetch, is what makes
    /// a second caller that reaches this while the first is still
    /// in-flight simply wait rather than start its own redundant fetch.
    /// A recent identical failure short-circuits before either the lock
    /// or the network (finding B1): the WebRTC bootstrap listener that
    /// also calls this is public and unauthenticated (D-S3-11), so a
    /// caller repeating the same unresolvable host must not repeat a
    /// full Tier-1/Tier-2 round trip for every repeat.
    async fn ensure_populated(
        &self,
        app_lookup_alias: &str,
        a_hash: &str,
        s_hash: &str,
    ) -> Result<TopologyKey> {
        let coalesce_key = (app_lookup_alias.to_string(), s_hash.to_string());

        if let Some(message) = self.fresh_negative(&coalesce_key) {
            anyhow::bail!(message);
        }

        let lock = self
            .inflight
            .entry(coalesce_key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Another caller may have finished this exact fetch while this
        // one waited for the lock -- the single-flight property itself.
        // `resolve_all` (not plain cache presence) is the check: it runs
        // the same `not_after` validity check `resolve` does without
        // needing a `routing_key`, so an entry that expired while this
        // caller was waiting is treated as still cold, not returned as if
        // fresh (the exact regression an earlier revision of this method
        // had against `an_expired_entry_triggers_one_refetch_rather_than_
        // a_failure`).
        if let Some(key) = self.cached_key(app_lookup_alias, s_hash)
            && self.resolver.resolve_all(&key).is_ok()
        {
            self.inflight.remove(&coalesce_key);
            return Ok(key);
        }
        if let Some(message) = self.fresh_negative(&coalesce_key) {
            self.inflight.remove(&coalesce_key);
            anyhow::bail!(message);
        }

        let result = self.fetch_and_bind(app_lookup_alias, a_hash, s_hash).await;
        // Recorded *before* the in-flight lock is dropped below: a caller
        // that arrives in the window between the two would otherwise find
        // neither the lock (already gone) nor the outcome (not yet
        // cached) and start its own redundant fetch on the failure path
        // -- the success path is unaffected, since `fetch_and_bind`
        // already wrote `app_dids`/`service_names` before returning.
        match &result {
            Ok(_) => {
                self.negative_cache.remove(&coalesce_key);
            }
            Err(e) => {
                // Swept before inserting, not just on some other timer:
                // `negative_cache` otherwise only ever loses an entry to a
                // later *success* for that exact key, so on the public,
                // unauthenticated bootstrap listener (D-S3-11) it grows by
                // one entry per distinct bad `Host` header forever. This
                // bounds it to roughly one `NEGATIVE_CACHE_TTL` window's
                // worth of distinct failures.
                self.negative_cache.retain(|_, (_, at)| at.elapsed() < NEGATIVE_CACHE_TTL);
                self.negative_cache.insert(coalesce_key.clone(), (e.to_string(), Instant::now()));
            }
        }
        self.inflight.remove(&coalesce_key);
        result
    }

    /// The real Tier-1-then-Tier-2 network round trip. Never called
    /// directly by `resolve_app_host` -- `ensure_populated` above is what
    /// keeps at most one of these running per `(app_lookup_alias, s_hash)`
    /// at a time.
    async fn fetch_and_bind(
        &self,
        app_lookup_alias: &str,
        a_hash: &str,
        s_hash: &str,
    ) -> Result<TopologyKey> {
        let fetcher = self
            .fetcher
            .as_ref()
            .context("no community registry configured; logical hostnames need Tier 1")?;

        // ── Tier 1 (cached) ──────────────────────────────────────────
        let (app_did, supervisor_did) = match self.app_dids.get(app_lookup_alias) {
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
                self.app_dids.insert(
                    app_lookup_alias.to_string(),
                    (did.clone(), rec.info.substrate_id.clone()),
                );
                (did, rec.info.substrate_id)
            }
        };

        // Tier 2 may already be known for this `app_did`+`s_hash` through
        // a *different* alias that resolved the same app earlier -- the
        // Tier-1 lookup above is keyed on the alias (finding B3), but
        // Tier 2 is keyed on the app DID, which is now known either way.
        // `resolve_all`, not plain presence, so an entry that has since
        // expired still triggers the real fetch below rather than
        // returning a key `resolve_app_host`'s own caller would just
        // fail against a second time.
        if let Some(name) = self.service_names.get(&(app_did.clone(), s_hash.to_string())) {
            let key = TopologyKey::foreign(app_did.clone(), name.clone());
            if self.resolver.resolve_all(&key).is_ok() {
                return Ok(key);
            }
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
        Ok(key)
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

    /// Finding C7: the no-registry path (`fetcher: None`, an empty
    /// `registry_url`) is what `ClientGateway::init` builds when
    /// `[substrate].registry_url` is unset -- an app-scoped host must be
    /// refused with a message naming Tier 1, not left to the panic/hang a
    /// missing fetcher would otherwise produce.
    #[tokio::test]
    async fn an_app_scoped_host_is_refused_with_no_registry_configured() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let resolver = app_host_resolver(tier1, None);
        let s_hash = util::short_hash("backend");

        let err =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
        assert!(err.to_string().contains("no community registry configured"), "{err}");
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
            .resolve_app_host(&format!("my-chat-app-{a_hash}"), &a_hash, "wronghash", None)
            .await
            .unwrap_err();
        // Pins the Tier-2 check specifically (finding C5): a correct
        // `a_hash` means Tier 1 must succeed here, so an `OR` against
        // either segment's error text would just as readily pass on a
        // Tier-1 regression as on the Tier-2 binding check this test
        // exists to cover.
        assert!(
            err.to_string().contains("supervisor answered '-swronghash' with service 'backend'"),
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

    /// Finding B3: the Tier-1 cache is keyed on the full
    /// `app_lookup_alias`, not `a_hash` alone -- a second host carrying a
    /// *different* nickname over the same app hash must repeat its own
    /// Tier-1 alias lookup rather than silently reuse the first alias's
    /// warm entry, which would let the cache accept a nickname the
    /// registry was never actually asked to bind.
    #[tokio::test]
    async fn a_different_nickname_over_the_same_app_hash_repeats_the_tier1_lookup() {
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
        assert_eq!(tier1.calls.load(Ordering::SeqCst), 1);

        // Same `a_hash`, a different nickname -- a different alias.
        resolver
            .resolve_app_host("a-totally-different-nickname", &a_hash, &s_hash, None)
            .await
            .unwrap();
        assert_eq!(
            tier1.calls.load(Ordering::SeqCst),
            2,
            "a different alias over the same hash must not reuse the first alias's cache entry"
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

        // The other half of this test's own title (finding C2): with no
        // header at all, a `Redundant` topology round-robins rather than
        // pinning one member.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..6 {
            seen.insert(
                resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap(),
            );
        }
        assert!(seen.len() > 1, "an unkeyed resolve must spread across members, got {seen:?}");
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

    /// Finding A3: a permanent selection failure (here, the same
    /// `Sharded`-with-no-key case test 89 pins) must not be treated as a
    /// cache miss. `FakeTier2` is seeded with exactly one response, so a
    /// second refetch would surface as "no more queued responses" instead
    /// of the resolver's own error -- the discriminator this test relies
    /// on -- and `fetcher.calls` pins it directly. Before the fix, every
    /// repeat call refetched Tier 2, breaking task.md budget 1 for any
    /// caller stuck in this permanent state.
    #[tokio::test]
    async fn a_permanent_selection_failure_is_not_treated_as_a_cache_miss() {
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
        let resolver = app_host_resolver(tier1, Some(fetcher.clone()));
        let s_hash = util::short_hash("backend");

        for _ in 0..3 {
            let err =
                resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
            assert!(err.to_string().contains("routing_key"), "{err}");
        }
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            1,
            "a permanent selection error must not trigger a refetch"
        );
    }

    /// Finding B1: a failed cold resolve is remembered for
    /// `NEGATIVE_CACHE_TTL`, so a caller repeating the same bad host
    /// (the WebRTC bootstrap listener that also calls this is public and
    /// unauthenticated, D-S3-11) does not repeat a Tier-1 round trip for
    /// every repeat. Reuses test 83's shape (a wrong `a_hash`) for the
    /// failure itself; what this test pins is that the *second* identical
    /// failure costs no further lookup.
    #[tokio::test]
    async fn a_recent_failure_is_served_from_the_negative_cache_without_a_repeat_lookup() {
        let (master, app_did) = app_master();
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let resolver = app_host_resolver(tier1.clone(), Some(Arc::new(FakeTier2::default())));

        let first = resolver
            .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
            .await
            .unwrap_err();
        assert!(first.to_string().contains("wronghash"), "{first}");
        assert_eq!(tier1.calls.load(Ordering::SeqCst), 1);

        let second = resolver
            .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
            .await
            .unwrap_err();
        assert_eq!(
            second.to_string(),
            first.to_string(),
            "the remembered failure must be identical"
        );
        assert_eq!(
            tier1.calls.load(Ordering::SeqCst),
            1,
            "a fresh negative-cache hit must not repeat the Tier-1 lookup"
        );
    }

    /// Residual finding: the negative cache had no sweep -- an entry was
    /// only ever removed by a later *success* for that exact key, so on
    /// the public, unauthenticated bootstrap listener it grew by one
    /// entry per distinct bad `Host` header forever. A new failure now
    /// sweeps every expired entry on its way in.
    #[tokio::test]
    async fn a_new_failure_sweeps_every_expired_negative_cache_entry() {
        let (master, app_did) = app_master();
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
        let resolver = app_host_resolver(tier1, Some(Arc::new(FakeTier2::default())));

        let _ = resolver.resolve_app_host("host-one", "wronghash", "anyhash", None).await;
        let _ = resolver.resolve_app_host("host-two", "wronghash", "anyhash", None).await;
        assert_eq!(resolver.negative_cache.len(), 2);

        tokio::time::sleep(NEGATIVE_CACHE_TTL + Duration::from_millis(500)).await;

        // A third, distinct failure sweeps the first two (now stale) on
        // its way in, leaving only itself.
        let _ = resolver.resolve_app_host("host-three", "wronghash", "anyhash", None).await;
        assert_eq!(
            resolver.negative_cache.len(),
            1,
            "expired entries must be swept, not accumulate forever"
        );
    }

    /// A `Tier1Lookup` whose artificial delay is what makes every
    /// concurrent caller in `concurrent_cold_resolves_for_the_same_host_
    /// share_one_fetch` still be waiting when the first one starts its
    /// real lookup -- without it, the race the test exists to exercise
    /// would only happen by chance.
    #[derive(Debug)]
    struct SlowTier1 {
        calls: AtomicUsize,
        response: SignedEndpointInfo,
    }

    #[async_trait::async_trait]
    impl Tier1Lookup for SlowTier1 {
        async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(self.response.clone())
        }
    }

    /// Finding B2: N concurrent callers for the same not-yet-cached host
    /// share one Tier-1-then-Tier-2 round trip rather than each starting
    /// an independent one. `FakeTier2` is seeded with exactly one
    /// response, so this also fails loudly ("no more queued responses")
    /// if the single-flight lock lets more than one caller through to the
    /// real fetch.
    #[tokio::test]
    async fn concurrent_cold_resolves_for_the_same_host_share_one_fetch() {
        let (master, app_did) = app_master();
        let a_hash = util::short_hash(app_did.as_str());
        let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
        let tier1 = Arc::new(SlowTier1 { calls: AtomicUsize::new(0), response: record });
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
        let resolver = AppHostResolver::new(
            Box::new(tier1.clone()),
            Some(Box::new(fetcher.clone()) as Box<dyn Tier2Fetch>),
            LogicalResolver::new(Arc::new(StaticInventory::new())),
        );
        let s_hash = util::short_hash("backend");

        let (r1, r2, r3, r4) = tokio::join!(
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
        );
        for r in [&r1, &r2, &r3, &r4] {
            assert_eq!(r.as_ref().unwrap().as_str(), "did:key:zMember0", "{r:?}");
        }
        assert_eq!(
            tier1.calls.load(Ordering::SeqCst),
            1,
            "one Tier-1 lookup shared by every concurrent caller"
        );
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            1,
            "one Tier-2 fetch shared by every concurrent caller"
        );
    }
}

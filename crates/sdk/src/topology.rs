//! The one `TopologyFetcher` implementation (ADR-0022 §3): Tier 1 then
//! Tier 2 over the real network. Verification is deliberately not done
//! here -- `register_verified` is the only place a document is trusted, so
//! a fetcher can never become the trust boundary.
//!
//! Also the app-scoped gateway host resolver: [`AppHostResolver`]
//! is the shared implementation of "hostname `-a…-s…` to a member DID"
//! that the client gateway and the WebRTC coordinator both need, lifted
//! here rather than written twice, since those two are the pair most
//! likely to drift subtly apart on the alias/document binding checks.

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
/// being retried. Short, deliberately: this exists to blunt
/// a burst of duplicate requests against an unauthenticated, public
/// listener (the WebRTC bootstrap page resolves through the same
/// resolver), not to hide a host that has genuinely started
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
    /// rediscover the same value is a duplicate lookup this path avoids.
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
/// fake instead of a real registry, to assert how many registry calls a
/// resolve makes.
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
/// [`RegistryTopologyFetcher`] rather than part of the
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

/// The alias half of the binding check: `RegistryClient::lookup` cannot
/// bind an *alias* lookup to what was asked for, by construction -- a
/// registry answering the alias with another app's perfectly valid,
/// self-signed record must not silently redirect this caller to it.
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

/// The document half of the binding check: `SignedTopologyDocument::verify`
/// checks the signer and the expiry, never *which service* was asked for.
fn check_tier2_binding(returned_service_name: &str, s_hash: &str) -> Result<()> {
    anyhow::ensure!(
        util::short_hash(returned_service_name) == s_hash,
        "supervisor answered '-s{s_hash}' with service '{returned_service_name}'"
    );
    Ok(())
}

/// Which log line a caller (`ClientGateway::init`, `CoordinatorWebRtc::init`)
/// should emit for its own `resolve_ucan`/`grant_resolve_to_node_did`
/// configuration, pulled out as a pure, shared function rather
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

/// The state each substrate-side consumer of an app-scoped
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
    /// request re-resolves neither. Keyed on the alias, not just
    /// `a_hash`, so a warm entry cannot answer a *different* nickname over
    /// the same hash without its own alias lookup -- `a_hash` alone would
    /// let the cache silently widen what the parser accepts.
    /// Bound to the hash at insert time regardless, so a cache hit
    /// is as checked as a miss.
    app_dids: DashMap<String, (AppDid, String)>,
    /// `(app_did, short_hash(service_name))` -> the real service name, as
    /// carried by a verified document. Only ever written from a document
    /// that passed the `short_hash(name) == hash` check.
    service_names: DashMap<(AppDid, String), LogicalServiceName>,
    /// One lock per `(app_lookup_alias, s_hash)` cold fetch currently in
    /// flight: concurrent callers for the same not-yet-cached
    /// host share one Tier-1-then-Tier-2 round trip rather than each
    /// starting an independent one. Removed once that fetch completes --
    /// the *outcome* is what stays cached (`app_dids`/`service_names` on
    /// success, `negative_cache` on failure), never the lock itself.
    inflight: DashMap<(String, String), Arc<AsyncMutex<()>>>,
    /// A cold fetch's most recent failure, remembered for
    /// `NEGATIVE_CACHE_TTL`.
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
    /// `ServiceId`. Tier 1 is cached alongside the supervising node, so a
    /// repeat request for the same app makes no registry call; Tier 2 is
    /// cached in the `LogicalResolver`, so a repeat request for the same
    /// service makes no network call at all until the entry expires or is
    /// evicted. A cold resolve is single-flighted and a failure briefly
    /// remembered -- see [`Self::ensure_populated`].
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
                // directly keeps "no network call after the first fetch"
                // true once a caller is stuck in one of these permanent
                // states, rather than refetching Tier 2 on every single
                // request.
                Err(e) if !is_retryable_resolve_error(&e) => return Err(e),
                // Not registered, or past `not_after`: fall through and
                // refetch.
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
    /// concurrent caller for the same cold key -- a `tokio::sync::Mutex`
    /// per key, held across the fetch, is what makes a second caller that
    /// reaches this while the first is still in-flight simply wait rather
    /// than start its own redundant fetch. A recent identical failure
    /// short-circuits before either the lock or the network: the WebRTC
    /// bootstrap listener that also calls this is public and
    /// unauthenticated, so a caller repeating the same unresolvable host
    /// must not repeat a full Tier-1/Tier-2 round trip for every repeat.
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
                // unauthenticated bootstrap listener it grows by one entry
                // per distinct bad `Host` header forever. This
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
        // Tier-1 lookup above is keyed on the alias, but Tier 2 is keyed
        // on the app DID, which is now known either way.
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
        // be the same round-trip twice. A hash is a valid
        // `LogicalServiceName` (8 z32 characters, so non-empty and free
        // of `/`/`#`), and the supervisor reverses it.
        let signed = fetcher
            .fetch_via(&supervisor_did, &app_did, &LogicalServiceName::try_new(s_hash)?)
            .await?;
        // The document half of the binding check: `verify` checks the
        // signer and the expiry, never *which service* was asked for.
        check_tier2_binding(signed.document.service_name.as_str(), s_hash)?;
        let key = register_verified(&self.resolver, &signed, &app_did, None)?;
        self.service_names
            .insert((app_did.clone(), s_hash.to_string()), signed.document.service_name.clone());
        Ok(key)
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;

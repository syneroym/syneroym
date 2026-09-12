//! Addressing and resolution overlay
//!
//! This module implements the logical resolver that sits *above* the physical
//! network router. The router continues to route by explicit [`ServiceId`]s;
//! this layer translates [`TopologyKey`]s into `ServiceId`s via an
//! [`AppRegistry`].
//!
//! # Architecture summary
//!
//! ```text
//! [Caller] → resolve(TopologyKey, routing_key?) → ServiceId
//!               ↓
//!          AppRegistry (topology state)
//!               ↓
//!          TopologyCache (keyed by TopologyKey = AppScope + LogicalServiceName)
//!               ↓
//!          Selector (Singleton | Redundant | Sharded via BLAKE3)
//!               ↓
//!          ServiceId  →  physical router
//! ```
//!
//! # Topology modes
//!
//! - **Singleton** — one member; `routing_key` is ignored.
//! - **Redundant** — round-robin for unkeyed calls; rendezvous hashing for
//!   keyed calls.
//! - **Sharded** — deterministic rendezvous hashing (BLAKE3) over the
//!   `routing_key`; supports sub-strategies: `HashSharding` (full key) and
//!   `EntityTagSharding` (partition-key-only).
//!
//! # Cache invalidation
//!
//! The topology cache is keyed by `(AppInstanceId, LogicalServiceName)`.
//! An entry is invalidated when:
//!  * The entry's `cache_ttl` has elapsed.
//!  * A caller explicitly triggers invalidation via [`AppRegistry::invalidate`]
//!    or [`LogicalResolver::register`].
//!
//! A cache **hit** does *not* compare epochs against the registry -- there is
//! no live re-check on the hot path, only TTL and explicit eviction. A writer
//! that wants a change visible before the TTL elapses (the binding write
//! does, to meet the convergence budget) must call
//! [`LogicalResolver::register`], never write the registry directly.

use std::sync::{Arc, atomic::AtomicU64};

use anyhow::{Error, Result};

pub mod registry;
pub mod select;
pub mod types;

pub(crate) use registry::TopologyCache;
pub use registry::{AppRegistry, StaticInventory};
pub(crate) use select::select_member;
pub use select::{range_select, rendezvous_select};
pub use types::{
    AllMembers, AppScope, BindingWriteOutcome, DEFAULT_BINDING_CACHE_TTL_MS, RangeChunk,
    RangeRoutingTable, ResolvedTopology, ShardingStrategy, TopologyEntry, TopologyEpoch,
    TopologyKey, classify_binding_write, is_retryable_resolve_error,
};
pub(crate) use types::{RetryableResolveError, expired_error, unix_now};

use crate::models::ServiceId;

// ─────────────────────────────────────────────────────────────
// LogicalResolver
// ─────────────────────────────────────────────────────────────

/// Translates a [`TopologyKey`] into an explicit [`ServiceId`] via the
/// [`AppRegistry`], applying topology-aware selection.
///
/// The resolver maintains a local topology cache to avoid redundant registry
/// reads on the hot resolution path.  The cache is keyed by
/// [`TopologyKey`] and stores the [`ResolvedTopology`]
/// (i.e., the full eligible set + epoch), **not** the selected member.
/// Member selection happens after the cache look-up so different callers
/// with different `routing_key`s get correct results without separate cache
/// entries.
///
/// Cache entries are invalidated when:
/// - The cache TTL has elapsed.
/// - The caller explicitly calls [`LogicalResolver::invalidate`] or
///   [`LogicalResolver::register`].
///
/// A cache **hit** does *not* compare epochs against the registry -- see the
/// module-level "Cache invalidation" section above.
#[derive(Debug)]
pub struct LogicalResolver {
    registry: Arc<dyn AppRegistry>,
    cache: TopologyCache,
}

// Lock-poisoning from a panicking writer is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here.
#[allow(clippy::expect_used)]
impl LogicalResolver {
    /// Create a new resolver backed by the provided registry.
    pub fn new(registry: Arc<dyn AppRegistry>) -> Self {
        Self { registry, cache: TopologyCache::default() }
    }

    /// Resolve a [`TopologyKey`] to a single [`ServiceId`].
    ///
    /// # Arguments
    /// - `key` — the app scope and logical name to resolve.
    /// - `routing_key` — optional bytes used for keyed selection (rendezvous
    ///   hashing for `Redundant` / `Sharded`, ignored for `Singleton`).
    ///
    /// # Errors
    /// - The logical service is not registered.
    /// - The topology has no eligible members.
    /// - `Sharded` mode is requested with an empty `routing_key`.
    /// - The registered entry is past its `not_after`.
    pub fn resolve(&self, key: &TopologyKey, routing_key: Option<&[u8]>) -> Result<ServiceId> {
        let topology = self.get_topology(key)?;
        select_member(&topology, routing_key, key)
    }

    /// Return the entire eligible member set for `key` as an
    /// epoch-consistent snapshot.  Use this for scatter-gather patterns.
    pub fn resolve_all(&self, key: &TopologyKey) -> Result<AllMembers> {
        let topology = self.get_topology(key)?;
        Ok(AllMembers { topology_epoch: topology.epoch, members: topology.members.clone() })
    }

    /// Explicitly evict the cache entry for `key`.
    pub fn invalidate(&self, key: &TopologyKey) {
        self.cache.evict(key);
        self.registry.invalidate(key);
    }

    /// Register `entry` and drop any cached copy in one step -- the write
    /// path's only entry point, so a binding write can never leave a stale
    /// cached topology behind. `AppRegistry::register` alone would leave a
    /// live cache entry serving the old membership for up to `cache_ttl`,
    /// which is what would make a scale-out invisible for up to a minute --
    /// well past the 5s convergence budget.
    pub fn register(&self, key: TopologyKey, entry: TopologyEntry) {
        self.registry.register(key.clone(), entry);
        self.cache.evict(&key);
    }

    // ── Internal helpers ─────────────────────────────────────

    /// Retrieve the `ResolvedTopology` for `key`, using the cache when
    /// valid, or re-fetching from the registry and updating the cache.
    ///
    /// Checked on both paths -- a cache entry whose `cache_ttl` outlives its
    /// `not_after` must not keep answering (ADR-0022 §3): it fails, it is not
    /// "stale but usable".
    fn get_topology(&self, key: &TopologyKey) -> Result<Arc<ResolvedTopology>> {
        let now = unix_now();

        // 1. Check cache validity first (fast path).
        if let Some(resolved) = self.cache.get(key) {
            if let Some(not_after) = resolved.not_after
                && now >= not_after
            {
                self.cache.evict(key);
                return Err(expired_error(key, not_after, now));
            }
            return Ok(resolved);
        }

        // 2. Cache miss or stale → Probe registry for entry.
        let entry = self.registry.get(key).ok_or_else(|| {
            Error::new(RetryableResolveError::NotRegistered)
                .context(format!("No topology registered for logical service '{key}'"))
        })?;

        if let Some(not_after) = entry.not_after
            && now >= not_after
        {
            // Never cached: an already-expired entry must not become a
            // cache hit later.
            return Err(expired_error(key, not_after, now));
        }

        // 3. Build ResolvedTopology from the registry entry.
        let resolved = Arc::new(ResolvedTopology {
            mode: entry.mode,
            members: entry.members.clone(),
            sharding_strategy: entry.sharding_strategy,
            epoch: entry.epoch,
            rr_counter: Arc::new(AtomicU64::new(0)),
            not_after: entry.not_after,
        });

        // 4. Store in cache.
        self.cache.insert(key.clone(), resolved.clone(), entry.cache_ttl);

        Ok(resolved)
    }
}

/// A `LogicalResolver` over a fresh, empty `StaticInventory` -- every
/// non-production `AppSandboxEngine::init`/`ControlPlaneService::init` call
/// site needs one of these and nothing else, so this saves each from
/// repeating `Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())))`.
/// Hidden: not part of this crate's public API, just a shared test fixture.
#[doc(hidden)]
#[must_use]
pub fn empty_resolver() -> Arc<LogicalResolver> {
    Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())))
}

#[cfg(test)]
mod tests;

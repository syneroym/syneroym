use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use super::types::{AppScope, LogicalServiceName, ResolvedTopology, TopologyEntry, TopologyKey};

// ─────────────────────────────────────────────────────────────
// AppRegistry trait
// ─────────────────────────────────────────────────────────────

/// Registry abstraction that manages topology state for logical service names.
///
/// Lives outside the router; the router only ever sees
/// [`crate::models::ServiceId`]s.  The registry is responsible for
/// persisting and invalidating topology entries.
pub trait AppRegistry: Send + Sync + fmt::Debug {
    /// Register or update the topology for `key`.
    fn register(&self, key: TopologyKey, entry: TopologyEntry);

    /// Look up the topology entry for `key`.
    ///
    /// Returns `None` if the combination has never been registered.
    fn get(&self, key: &TopologyKey) -> Option<TopologyEntry>;

    /// Explicitly invalidate the cached copy for `key`.
    ///
    /// The *registry* entry itself is preserved; only in-process caches should
    /// be evicted.  The next resolution will re-read from the registry.
    fn invalidate(&self, key: &TopologyKey);

    /// List all registered logical services under an app scope.
    fn list(&self, app: &AppScope) -> Vec<LogicalServiceName>;
}

// ─────────────────────────────────────────────────────────────
// StaticInventory — standalone mode
// ─────────────────────────────────────────────────────────────

/// In-memory registry: resolved bindings are injected at deploy time
/// and never replicated to a live backend.
///
/// `StaticInventory` is the only registry mode implemented. Dynamic or
/// database-backed registries are future work.
#[derive(Debug, Clone)]
pub struct StaticInventory {
    inner: Arc<RwLock<StaticInventoryInner>>,
}

#[derive(Debug, Default)]
struct StaticInventoryInner {
    entries: BTreeMap<TopologyKey, TopologyEntry>,
}

impl StaticInventory {
    /// Create an empty `StaticInventory`.
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(StaticInventoryInner::default())) }
    }
}

impl Default for StaticInventory {
    fn default() -> Self {
        Self::new()
    }
}

// Lock-poisoning from a panicking writer is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here.
#[allow(clippy::expect_used)]
impl AppRegistry for StaticInventory {
    fn register(&self, key: TopologyKey, entry: TopologyEntry) {
        let mut inner = self.inner.write().expect("registry lock poisoned");
        inner.entries.insert(key, entry);
    }

    fn get(&self, key: &TopologyKey) -> Option<TopologyEntry> {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner.entries.get(key).cloned()
    }

    fn invalidate(&self, _key: &TopologyKey) {
        // For StaticInventory there is no separate cache tier; the in-memory
        // map IS the cache.  Invalidation is a no-op at this level; the
        // LogicalResolver's cache handles eviction separately.
    }

    fn list(&self, app: &AppScope) -> Vec<LogicalServiceName> {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner.entries.keys().filter(|k| k.app == *app).map(|k| k.service_name.clone()).collect()
    }
}

// ─────────────────────────────────────────────────────────────
// Topology cache
// ─────────────────────────────────────────────────────────────

/// A single entry in the resolver's local topology cache.
#[derive(Debug, Clone)]
pub(crate) struct CacheEntry {
    topology: Arc<ResolvedTopology>,
    /// When this cache entry was created or last refreshed.
    cached_at: Instant,
    /// TTL copied from the registry entry at cache time.
    ttl: Duration,
}

impl CacheEntry {
    fn is_valid(&self) -> bool {
        self.cached_at.elapsed() < self.ttl
    }
}

#[derive(Debug, Default)]
pub(crate) struct TopologyCache {
    entries: dashmap::DashMap<TopologyKey, CacheEntry>,
}

impl TopologyCache {
    pub(crate) fn get(&self, key: &TopologyKey) -> Option<Arc<ResolvedTopology>> {
        self.entries.get(key).filter(|e| e.is_valid()).map(|e| e.topology.clone())
    }

    pub(crate) fn insert(&self, key: TopologyKey, topology: Arc<ResolvedTopology>, ttl: Duration) {
        self.entries.insert(key, CacheEntry { topology, cached_at: Instant::now(), ttl });
    }

    pub(crate) fn evict(&self, key: &TopologyKey) {
        self.entries.remove(key);
    }
}

//! Slice 3 — Addressing & Resolution Overlay
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
//! that wants a change visible before the TTL elapses (A2's binding write
//! does, to meet the milestone's convergence budget) must call
//! [`LogicalResolver::register`], never write the registry directly.

use std::{
    cmp,
    collections::BTreeMap,
    error, fmt,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Error, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::models::{AppDid, AppInstanceId, LogicalServiceName, ServiceId, TopologyMode};

// ─────────────────────────────────────────────────────────────
// Domain types
// ─────────────────────────────────────────────────────────────

/// Default cache TTL for a binding written at deploy time (A2), matching
/// what this module's own tests already treat as ordinary
/// (`Duration::from_secs(60)`).
pub const DEFAULT_BINDING_CACHE_TTL_MS: u64 = 60_000;

/// Monotonically increasing counter that changes whenever the topology (member
/// set or mode) for a logical service changes.  Cache entries are invalidated
/// when the stored epoch no longer matches the registry epoch.
#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TopologyEpoch(pub u64);

impl TopologyEpoch {
    /// Return the next epoch value.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// A single contiguous range (chunk) in a range-sharded topology.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeChunk {
    /// Inclusive lower bound. `None` means -infinity (MinKey).
    pub start_key: Option<Vec<u8>>,
    /// Exclusive upper bound. `None` means +infinity (MaxKey).
    pub end_key: Option<Vec<u8>>,
    /// Target service member for keys in this range.
    pub target: ServiceId,
}

/// A complete chunk map routing table representing contiguous non-overlapping
/// ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeRoutingTable {
    pub chunks: Vec<RangeChunk>,
}

impl RangeRoutingTable {
    /// Validates that chunks are contiguous and cover the entire keyspace from
    /// -infinity to +infinity.
    pub fn validate(&self) -> Result<()> {
        if self.chunks.is_empty() {
            return Err(anyhow!("Range routing table must contain at least one chunk"));
        }

        // Verify first chunk starts at -infinity
        if self.chunks[0].start_key.is_some() {
            return Err(anyhow!("First range chunk must start at -infinity (None)"));
        }

        // Verify last chunk ends at +infinity
        if self.chunks.last().is_some_and(|last| last.end_key.is_some()) {
            return Err(anyhow!("Last range chunk must end at +infinity (None)"));
        }

        // Verify contiguity and sorting
        for i in 0..self.chunks.len() {
            let current = &self.chunks[i];

            if let (Some(start), Some(end)) = (&current.start_key, &current.end_key)
                && start >= end
            {
                return Err(anyhow!("Range chunk {} has start_key >= end_key", i));
            }

            if i < self.chunks.len() - 1 {
                let next = &self.chunks[i + 1];

                match (&current.end_key, &next.start_key) {
                    (Some(curr_end), Some(next_start)) => {
                        if curr_end != next_start {
                            return Err(anyhow!(
                                "Range chunks are not contiguous: chunk {} ends at {:?} but chunk \
                                 {} starts at {:?}",
                                i,
                                curr_end,
                                i + 1,
                                next_start
                            ));
                        }
                    }
                    _ => {
                        return Err(anyhow!(
                            "Invalid boundary logic between chunk {} and {}",
                            i,
                            i + 1
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Sub-strategy for [`TopologyMode::Sharded`] selections.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShardingStrategy {
    /// Rendezvous hash over the entire `routing_key`.
    HashSharding,
    /// Rendezvous hash over the first segment of the `routing_key` (treated as
    /// `partition_key`), ensuring entity-local data locality.
    EntityTagSharding,
    /// Range-based sharding mapping contiguous key ranges to specific members.
    RangeSharding(RangeRoutingTable),
}

/// The four outcomes ADR-0021 §3 requires a binding write to be
/// distinguishable between. Kept as data rather than a `Result` because
/// three of the four are successes: only the caller decides whether
/// `Stale` or `Conflict` is worth an alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingWriteOutcome {
    /// No entry held, or a strictly higher epoch. The caller applies.
    Applied,
    /// Same epoch, same membership. Success with no write -- the ordinary
    /// retry ADR-0021 §5 says to expect.
    NoOp,
    /// Same epoch, different membership. Two writers produced different
    /// answers at one epoch, which is the signal ADR-0021 §4 exists to
    /// catch.
    Conflict(TopologyEpoch),
    /// A lower epoch: a late-arriving retry. The mapping does not regress.
    Stale(TopologyEpoch),
}

/// Applies ADR-0021 §3's four-case rule. Pure: no storage, no resolver, so
/// the rule itself is unit-testable with no substrate.
///
/// `held` must come from the **per-dependent** persisted binding row, not
/// from the shared `StaticInventory` entry: that entry is keyed
/// `(app_instance_id, service_name)` and is one value per substrate, so
/// classifying against it would give every dependent on a node the same
/// answer and produce false conflicts the moment two dependents
/// legitimately differ.
///
/// "Content" is `(mode, members, sharding_strategy)` and deliberately
/// **not** `cache_ttl`: a TTL difference at one epoch is a policy
/// difference between two writers, not a disagreement about who is
/// serving the service, and reporting it as a two-writer conflict would
/// make the signal noisy exactly where it must be trustworthy. `not_after`
/// is excluded for the same reason: it is a policy value about when an
/// entry stops answering, not a claim about who is serving the service.
#[must_use]
pub fn classify_binding_write(
    held: Option<&TopologyEntry>,
    incoming: &TopologyEntry,
) -> BindingWriteOutcome {
    let Some(held) = held else { return BindingWriteOutcome::Applied };
    match incoming.epoch.cmp(&held.epoch) {
        cmp::Ordering::Greater => BindingWriteOutcome::Applied,
        cmp::Ordering::Less => BindingWriteOutcome::Stale(held.epoch),
        cmp::Ordering::Equal => {
            let same = held.mode == incoming.mode
                && held.members == incoming.members
                && held.sharding_strategy == incoming.sharding_strategy;
            if same { BindingWriteOutcome::NoOp } else { BindingWriteOutcome::Conflict(held.epoch) }
        }
    }
}

/// Full topology descriptor stored per logical service in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyEntry {
    /// How this logical service name maps to physical members.
    pub mode: TopologyMode,
    /// Ordered set of eligible member `ServiceId`s.
    pub members: Vec<ServiceId>,
    /// Sharding sub-strategy (only meaningful for `Sharded` mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
    /// Which counter this is depends on the entry's `AppScope`, and nothing
    /// in the type says so:
    ///
    /// - under `AppScope::Local`, the **per-dependent binding epoch** the
    ///   supervisor advances on every push to one dependent
    ///   (`SupervisorStore::advance_binding_epoch`), which is what
    ///   `classify_binding_write` compares;
    /// - under `AppScope::Foreign`, the **per-logical-service topology epoch**
    ///   a Tier-2 document carries (ADR-0022 §6), which changes when and only
    ///   when a member set or mode does.
    ///
    /// They are never compared with each other only because the two scopes
    /// are disjoint keys -- the separation is `AppScope`'s, not this
    /// field's. Anything that later reads this epoch without knowing the
    /// scope (shard rebalancing's data-path fence is the one on the map) has
    /// to establish the scope first.
    pub epoch: TopologyEpoch,
    /// Maximum age of a cached copy of this topology.
    #[serde(with = "duration_millis")]
    pub cache_ttl: Duration,
    /// Unix seconds after which this entry must stop resolving (ADR-0022 §3,
    /// failure-matrix row 6: past `not_after`, fail -- not "stale but
    /// usable"). `None` for an entry pushed by the intra-app binding path,
    /// which has no expiry and is refreshed by a later push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<u64>,
}

/// The resolved topology for a logical service — what the cache stores.
///
/// This intentionally stores the *full* eligible set, not a pre-selected
/// member.  The caller (selector) performs member selection so the cache stays
/// topology-epoch aligned, not request aligned.
#[derive(Debug, Clone)]
pub struct ResolvedTopology {
    pub mode: TopologyMode,
    pub members: Vec<ServiceId>,
    pub sharding_strategy: Option<ShardingStrategy>,
    pub epoch: TopologyEpoch,
    pub rr_counter: Arc<AtomicU64>,
    /// Copied from `TopologyEntry.not_after` at resolution time.
    pub not_after: Option<u64>,
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Marks a [`LogicalResolver::resolve`] (or [`LogicalResolver::resolve_all`])
/// failure a caller may fix by fetching a fresh Tier-2 document -- the entry
/// is missing, or has aged past its own `not_after`. Every *other* `resolve`
/// failure (an empty member set, a `Sharded` request with no routing key) is
/// a permanent property of the document itself, and re-fetching the
/// identical document changes nothing.
///
/// A caller that wants to retry on the first kind and surface the second
/// kind directly checks [`is_retryable`] rather than matching on the error
/// text -- `AppHostResolver::resolve_app_host` (S3) is the reason this
/// exists: treating every `resolve` error as a cache miss made a permanent
/// selection failure (e.g. a `Sharded` service called with no routing key)
/// refetch Tier 2 on every single request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryableResolveError {
    NotRegistered,
    Expired,
}

impl fmt::Display for RetryableResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegistered => write!(f, "not registered"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

impl error::Error for RetryableResolveError {}

/// True when `err` came from [`LogicalResolver::resolve`] (or
/// [`LogicalResolver::resolve_all`]) for a reason a fresh Tier-2 fetch can
/// fix. See [`RetryableResolveError`].
#[must_use]
pub fn is_retryable_resolve_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.downcast_ref::<RetryableResolveError>().is_some())
}

fn expired_error(key: &TopologyKey, not_after: u64, now: u64) -> anyhow::Error {
    Error::new(RetryableResolveError::Expired).context(format!(
        "topology for '{key}' expired at unix time {not_after} (now {now}); a Tier-2 document \
         must be re-fetched"
    ))
}

/// The result of a `resolve_all` call: an epoch-consistent snapshot of all
/// eligible members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllMembers {
    pub topology_epoch: TopologyEpoch,
    pub members: Vec<ServiceId>,
}

// ─────────────────────────────────────────────────────────────
// AppScope / TopologyKey
// ─────────────────────────────────────────────────────────────

/// Which app a topology entry belongs to (ADR-0022 §1, milestone plan §0.4).
///
/// `Local` is an app instance deployed through this node, keyed by the name
/// this node's own operator chose -- unique here by construction. `Foreign`
/// is another app's topology, learned from a verified Tier-2 document and
/// keyed by the app master DID, which is globally unique. Two unrelated apps
/// both called `chat` are two different keys, because they are two different
/// DIDs; keying both by the human name would silently re-point one at the
/// other's members.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AppScope {
    Local(AppInstanceId),
    Foreign(AppDid),
}

impl AppScope {
    /// The bytes this scope contributes to `rendezvous_select`'s domain
    /// separator.
    ///
    /// Deliberately *not* canonical across the two variants: an intra-app
    /// caller separates by the instance id and a foreign caller by the app
    /// DID, so the two disagree about which member a routing key selects.
    /// Unreachable today (`Sharded` is compiled by nothing, `Redundant`'s
    /// keyed path is load balancing, and no cross-app caller exists), and
    /// it becomes reachable when shard rebalancing enforces the epoch fence
    /// on the data path. Fixing it needs one canonical separator -- the app
    /// DID -- which needs the intra-app push path to carry the app DID on
    /// the wire. Recorded in the deferred backlog rather than built against
    /// a consumer that does not exist.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Local(id) => id.as_str(),
            Self::Foreign(did) => did.as_str(),
        }
    }
}

impl fmt::Display for AppScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The key of a topology entry: which app, and which logical service inside
/// it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TopologyKey {
    pub app: AppScope,
    pub service_name: LogicalServiceName,
}

impl TopologyKey {
    #[must_use]
    pub fn local(app_instance_id: AppInstanceId, service_name: LogicalServiceName) -> Self {
        Self { app: AppScope::Local(app_instance_id), service_name }
    }

    #[must_use]
    pub fn foreign(app_did: AppDid, service_name: LogicalServiceName) -> Self {
        Self { app: AppScope::Foreign(app_did), service_name }
    }
}

impl fmt::Display for TopologyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.app, self.service_name)
    }
}

// ─────────────────────────────────────────────────────────────
// AppRegistry trait
// ─────────────────────────────────────────────────────────────

/// Registry abstraction that manages topology state for logical service names.
///
/// Lives outside the router; the router only ever sees [`ServiceId`]s.  The
/// registry is responsible for persisting and invalidating topology entries.
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
// StaticInventory — Phase 0 standalone mode
// ─────────────────────────────────────────────────────────────

/// Phase 0 in-memory registry: resolved bindings are injected at deploy time
/// and never replicated to a live backend.
///
/// `StaticInventory` is the only registry mode required for M1.  Dynamic or
/// database-backed registries are deferred to M3/M5.
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
struct CacheEntry {
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
struct TopologyCache {
    entries: dashmap::DashMap<TopologyKey, CacheEntry>,
}

impl TopologyCache {
    fn get(&self, key: &TopologyKey) -> Option<Arc<ResolvedTopology>> {
        self.entries.get(key).filter(|e| e.is_valid()).map(|e| e.topology.clone())
    }

    fn insert(&self, key: TopologyKey, topology: Arc<ResolvedTopology>, ttl: Duration) {
        self.entries.insert(key, CacheEntry { topology, cached_at: Instant::now(), ttl });
    }

    fn evict(&self, key: &TopologyKey) {
        self.entries.remove(key);
    }
}

// ─────────────────────────────────────────────────────────────
// Rendezvous hashing
// ─────────────────────────────────────────────────────────────

/// Select one member from `members` via deterministic BLAKE3 rendezvous
/// hashing.
///
/// `app_instance_id` and `service_name` form the domain separator context.
/// `routing_key` is the caller-supplied bytes.
///
/// Returns `None` if `members` is empty.
///
/// Tie-breaking (hash collision): lexicographic comparison of the canonical
/// `ServiceId` byte representation (highest wins).
pub fn rendezvous_select<'a>(
    members: &'a [ServiceId],
    app_instance_id: &[u8],
    service_name: &[u8],
    routing_key: &[u8],
) -> Option<&'a ServiceId> {
    use blake3::Hasher;

    let mut prefix_hasher = Hasher::new();

    prefix_hasher.update(&(app_instance_id.len() as u64).to_be_bytes());
    prefix_hasher.update(app_instance_id);

    prefix_hasher.update(&(service_name.len() as u64).to_be_bytes());
    prefix_hasher.update(service_name);

    prefix_hasher.update(&(routing_key.len() as u64).to_be_bytes());
    prefix_hasher.update(routing_key);

    members
        .iter()
        .map(|m| {
            let mut hasher = prefix_hasher.clone();
            let service_id_bytes = m.as_str().as_bytes();
            hasher.update(&(service_id_bytes.len() as u64).to_be_bytes());
            hasher.update(service_id_bytes);
            let score = *hasher.finalize().as_bytes();
            (score, m)
        })
        .max_by(|a, b| {
            // Primary: unsigned lexicographic comparison of 32-byte digests (highest wins).
            // Tie-break: lexicographic comparison of ServiceId bytes (highest wins).
            a.0.cmp(&b.0).then_with(|| a.1.as_str().as_bytes().cmp(b.1.as_str().as_bytes()))
        })
        .map(|(_, m)| m)
}

/// Helper to select a ServiceId using Range Sharding.
///
/// Validates the routing table dynamically to ensure the boundaries are valid
/// before routing.
pub fn range_select(table: &RangeRoutingTable, key: &[u8]) -> Result<ServiceId> {
    table.validate()?;

    let chunk = table
        .chunks
        .iter()
        .find(|c| {
            let after_start = c.start_key.as_deref().is_none_or(|start| key >= start);
            let before_end = c.end_key.as_deref().is_none_or(|end| key < end);
            after_start && before_end
        })
        .ok_or_else(|| anyhow!("No routing chunk found for key"))?;

    Ok(chunk.target.clone())
}

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
    /// well past the milestone's 5s convergence budget.
    pub fn register(&self, key: TopologyKey, entry: TopologyEntry) {
        self.registry.register(key.clone(), entry);
        self.cache.evict(&key);
    }

    // ── Internal helpers ─────────────────────────────────────

    /// Retrieve the `ResolvedTopology` for `key`, using the cache when
    /// valid, or re-fetching from the registry and updating the cache.
    ///
    /// Checked on both paths -- a cache entry whose `cache_ttl` outlives its
    /// `not_after` must not keep answering (ADR-0022 §3, failure-matrix row
    /// 6: "fails. Not 'stale but usable'").
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

/// Select one member from `topology`, applying the correct strategy.
fn select_member(
    topology: &ResolvedTopology,
    routing_key: Option<&[u8]>,
    key: &TopologyKey,
) -> Result<ServiceId> {
    if topology.members.is_empty() {
        return Err(anyhow!("Topology has no eligible members"));
    }

    match topology.mode {
        TopologyMode::Singleton => {
            // Must have exactly one member by design; defensive guard.
            topology
                .members
                .first()
                .cloned()
                .ok_or_else(|| anyhow!("Singleton topology has no members"))
        }

        TopologyMode::Redundant => {
            if let Some(routing_key) = routing_key {
                // Keyed call: rendezvous hashing.
                rendezvous_select(
                    &topology.members,
                    key.app.as_str().as_bytes(),
                    key.service_name.as_str().as_bytes(),
                    routing_key,
                )
                .cloned()
                .ok_or_else(|| anyhow!("Redundant topology member selection failed"))
            } else {
                // Unkeyed call: round-robin.
                let idx = topology.rr_counter.fetch_add(1, Ordering::Relaxed) as usize
                    % topology.members.len();
                Ok(topology.members[idx].clone())
            }
        }

        TopologyMode::Sharded => {
            let routing_key = routing_key
                .ok_or_else(|| anyhow!("Sharded topology requires a routing_key for selection"))?;

            match &topology.sharding_strategy {
                Some(ShardingStrategy::RangeSharding(table)) => range_select(table, routing_key),
                Some(ShardingStrategy::HashSharding)
                | None
                | Some(ShardingStrategy::EntityTagSharding) => {
                    let effective_key = match &topology.sharding_strategy {
                        Some(ShardingStrategy::EntityTagSharding) => {
                            routing_key.split(|&b| b == 0).next().unwrap_or(routing_key)
                        }
                        _ => routing_key,
                    };

                    rendezvous_select(
                        &topology.members,
                        key.app.as_str().as_bytes(),
                        key.service_name.as_str().as_bytes(),
                        effective_key,
                    )
                    .cloned()
                    .ok_or_else(|| anyhow!("Sharded topology member selection failed"))
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Serde helpers
// ─────────────────────────────────────────────────────────────

mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc, time::Duration};

    use super::*;
    use crate::models::{AppDid, AppInstanceId, LogicalServiceName, TopologyMode};

    // ── Helper builders ──────────────────────────────────────

    fn inst(s: &str) -> AppInstanceId {
        AppInstanceId::new(s)
    }

    fn svc_name(s: &str) -> LogicalServiceName {
        LogicalServiceName::new(s)
    }

    fn svc_id(s: &str) -> ServiceId {
        ServiceId::new(format!("did:key:{s}"))
    }

    fn did(s: &str) -> AppDid {
        AppDid::new(format!("did:key:{s}"))
    }

    fn local_key(inst_id: &str, name: &str) -> TopologyKey {
        TopologyKey::local(inst(inst_id), svc_name(name))
    }

    fn foreign_key(app_did: &str, name: &str) -> TopologyKey {
        TopologyKey::foreign(did(app_did), svc_name(name))
    }

    fn make_entry(
        mode: TopologyMode,
        members: Vec<ServiceId>,
        strategy: Option<ShardingStrategy>,
    ) -> TopologyEntry {
        TopologyEntry {
            mode,
            members,
            sharding_strategy: strategy,
            epoch: TopologyEpoch::default(),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        }
    }

    fn registry_with(entries: Vec<(TopologyKey, TopologyEntry)>) -> Arc<StaticInventory> {
        let reg = Arc::new(StaticInventory::new());
        for (key, entry) in entries {
            reg.register(key, entry);
        }
        reg
    }

    // ── StaticInventory ──────────────────────────────────────

    #[test]
    fn test_static_inventory_register_and_get() {
        let inv = StaticInventory::new();
        let key = local_key("app-1", "auth");
        let entry = make_entry(TopologyMode::Singleton, vec![svc_id("abc")], None);

        inv.register(key.clone(), entry.clone());

        let got = inv.get(&key).expect("should be present");
        assert_eq!(got.mode, TopologyMode::Singleton);
        assert_eq!(got.members, vec![svc_id("abc")]);
    }

    #[test]
    fn test_static_inventory_list() {
        let inv = StaticInventory::new();
        let id = inst("app-1");
        inv.register(
            local_key("app-1", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("a")], None),
        );
        inv.register(
            local_key("app-1", "cache"),
            make_entry(TopologyMode::Redundant, vec![svc_id("b")], None),
        );
        // Different app — should not be listed.
        inv.register(
            local_key("other", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("c")], None),
        );

        let mut names = inv.list(&AppScope::Local(id));
        names.sort();
        assert_eq!(names, vec![svc_name("auth"), svc_name("cache")]);
    }

    #[test]
    fn test_static_inventory_update_replaces_entry() {
        let inv = StaticInventory::new();
        let key = local_key("app-1", "auth");

        inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("old")], None));
        inv.register(
            key.clone(),
            TopologyEntry {
                epoch: TopologyEpoch(1),
                ..make_entry(TopologyMode::Redundant, vec![svc_id("new1"), svc_id("new2")], None)
            },
        );

        let got = inv.get(&key).unwrap();
        assert_eq!(got.mode, TopologyMode::Redundant);
        assert_eq!(got.epoch, TopologyEpoch(1));
        assert_eq!(got.members.len(), 2);
    }

    #[test]
    fn test_static_inventory_get_missing() {
        let inv = StaticInventory::new();
        assert!(inv.get(&local_key("app-x", "nonexistent")).is_none());
    }

    // ── Rendezvous hashing ───────────────────────────────────

    #[test]
    fn test_rendezvous_select_deterministic() {
        let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
        let app_domain = b"app-instance-1";
        let svc_domain = b"svc-1";
        let key = b"user-42";

        let first = rendezvous_select(&members, app_domain, svc_domain, key);
        let second = rendezvous_select(&members, app_domain, svc_domain, key);
        assert_eq!(first, second, "rendezvous selection must be deterministic");
    }

    #[test]
    fn test_rendezvous_select_different_keys_can_differ() {
        let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
        let app_domain = b"app-instance-1";
        let svc_domain = b"svc-1";

        let results: Vec<_> = (0u64..20)
            .map(|i| rendezvous_select(&members, app_domain, svc_domain, &i.to_be_bytes()))
            .collect();

        let distinct: HashSet<_> =
            results.into_iter().flatten().map(|s| s.as_str().to_string()).collect();
        // With 20 keys and 3 members, expect at least 2 distinct selections.
        assert!(distinct.len() >= 2, "rendezvous should distribute across members");
    }

    #[test]
    fn test_rendezvous_select_single_member() {
        let members = vec![svc_id("only")];
        let result = rendezvous_select(&members, b"app", b"svc", b"any-key");
        assert_eq!(result, Some(&svc_id("only")));
    }

    #[test]
    fn test_rendezvous_select_empty() {
        let members: Vec<ServiceId> = vec![];
        assert!(rendezvous_select(&members, b"app", b"svc", b"key").is_none());
    }

    #[test]
    fn test_rendezvous_domain_separator_changes_result() {
        let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
        let key = b"same-routing-key";

        // Different domain separators (AppInstanceIds/LogicalServiceNames) must produce
        // independent hash spaces.  Collect multiple results and confirm they
        // are not all identical across different domain separators.
        let results_by_domain: Vec<Option<&ServiceId>> = [
            (b"app-a".as_ref(), b"svc-1".as_ref()),
            (b"app-b".as_ref(), b"svc-1".as_ref()),
            (b"app-c".as_ref(), b"svc-1".as_ref()),
            (b"app-a".as_ref(), b"svc-2".as_ref()),
        ]
        .iter()
        .map(|(app, svc)| rendezvous_select(&members, app, svc, key))
        .collect();

        let distinct: HashSet<_> =
            results_by_domain.into_iter().flatten().map(|s| s.as_str().to_string()).collect();
        // With 4 different domain separators and the same routing key, we
        // expect at least 2 distinct selected members.
        assert!(
            distinct.len() >= 2,
            "different domain separators should produce independent hash spaces"
        );
    }

    // ── LogicalResolver — Singleton ──────────────────────────

    #[test]
    fn test_resolve_singleton() {
        let reg = registry_with(vec![(
            local_key("app-1", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("sole-member")], None),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "auth");

        let id = resolver.resolve(&key, None).unwrap();
        assert_eq!(id, svc_id("sole-member"));
    }

    #[test]
    fn test_resolve_unregistered_returns_error() {
        let reg = Arc::new(StaticInventory::new());
        let resolver = LogicalResolver::new(reg);
        let err = resolver.resolve(&local_key("ghost-app", "missing"), None);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("No topology registered"));
    }

    #[test]
    fn test_resolve_empty_members_returns_error() {
        let reg = registry_with(vec![(
            local_key("app-1", "empty"),
            make_entry(TopologyMode::Singleton, vec![], None),
        )]);
        let resolver = LogicalResolver::new(reg);
        let err = resolver.resolve(&local_key("app-1", "empty"), None);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("no eligible members"));
    }

    // ── LogicalResolver — Redundant ──────────────────────────

    #[test]
    fn test_resolve_redundant_round_robin() {
        let members = vec![svc_id("r0"), svc_id("r1"), svc_id("r2")];
        let reg = registry_with(vec![(
            local_key("app-1", "cache"),
            make_entry(TopologyMode::Redundant, members.clone(), None),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "cache");

        // With no routing key, round-robin through members.
        let r0 = resolver.resolve(&key, None).unwrap();
        let r1 = resolver.resolve(&key, None).unwrap();
        let r2 = resolver.resolve(&key, None).unwrap();
        let r3 = resolver.resolve(&key, None).unwrap(); // wraps back

        assert_eq!(r0, members[0]);
        assert_eq!(r1, members[1]);
        assert_eq!(r2, members[2]);
        assert_eq!(r3, members[0]); // wrapped
    }

    #[test]
    fn test_resolve_redundant_keyed_is_deterministic() {
        let members = vec![svc_id("r0"), svc_id("r1"), svc_id("r2")];
        let reg = registry_with(vec![(
            local_key("app-1", "cache"),
            make_entry(TopologyMode::Redundant, members, None),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "cache");

        let a = resolver.resolve(&key, Some(b"key-abc")).unwrap();
        let b = resolver.resolve(&key, Some(b"key-abc")).unwrap();
        assert_eq!(a, b, "keyed redundant resolve must be deterministic");
    }

    // ── LogicalResolver — Sharded ────────────────────────────

    #[test]
    fn test_resolve_sharded_requires_routing_key() {
        let reg = registry_with(vec![(
            local_key("app-1", "store"),
            make_entry(
                TopologyMode::Sharded,
                vec![svc_id("s0"), svc_id("s1")],
                Some(ShardingStrategy::HashSharding),
            ),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "store");

        let err = resolver.resolve(&key, None);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("routing_key"));
    }

    #[test]
    fn test_resolve_sharded_hash_deterministic() {
        let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
        let reg = registry_with(vec![(
            local_key("app-1", "store"),
            make_entry(TopologyMode::Sharded, members, Some(ShardingStrategy::HashSharding)),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "store");

        let a = resolver.resolve(&key, Some(b"user:42")).unwrap();
        let b_res = resolver.resolve(&key, Some(b"user:42")).unwrap();
        assert_eq!(a, b_res);
    }

    #[test]
    fn test_resolve_sharded_entity_tag_uses_partition_key() {
        // EntityTagSharding: only the bytes before the first NUL matter.
        let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
        let reg = registry_with(vec![(
            local_key("app-1", "ts"),
            make_entry(TopologyMode::Sharded, members, Some(ShardingStrategy::EntityTagSharding)),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "ts");

        // Same partition key, different item keys → same shard.
        let mut key1 = b"tenant-99\0item-1".to_vec();
        let mut key2 = b"tenant-99\0item-2".to_vec();
        let _ = &mut key1; // suppress unused warning
        let _ = &mut key2;
        let r1 = resolver.resolve(&key, Some(&key1)).unwrap();
        let r2 = resolver.resolve(&key, Some(&key2)).unwrap();
        assert_eq!(r1, r2, "same partition key must map to same shard");
    }

    #[test]
    fn test_resolve_sharded_distribution() {
        let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
        let reg = registry_with(vec![(
            local_key("app-1", "store"),
            make_entry(
                TopologyMode::Sharded,
                members.clone(),
                Some(ShardingStrategy::HashSharding),
            ),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "store");

        let mut counts = BTreeMap::new();
        for i in 0u64..300 {
            let routing_key = i.to_be_bytes();
            let selected = resolver.resolve(&key, Some(&routing_key)).unwrap();
            *counts.entry(selected.to_string()).or_insert(0u64) += 1;
        }
        // All 3 members should be selected at least once with 300 distinct keys.
        assert_eq!(counts.len(), 3, "all shards should receive traffic");
    }

    // ── LogicalResolver — cache invalidation ─────────────────

    #[test]
    fn test_cache_hit_bypasses_registry() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Debug)]
        struct MockRegistry {
            call_count: AtomicUsize,
            entry: TopologyEntry,
        }

        impl AppRegistry for MockRegistry {
            fn register(&self, _: TopologyKey, _: TopologyEntry) {}
            fn get(&self, _: &TopologyKey) -> Option<TopologyEntry> {
                self.call_count.fetch_add(1, Ordering::Relaxed);
                Some(self.entry.clone())
            }
            fn invalidate(&self, _: &TopologyKey) {}
            fn list(&self, _: &AppScope) -> Vec<LogicalServiceName> {
                vec![]
            }
        }

        let mock = Arc::new(MockRegistry {
            call_count: AtomicUsize::new(0),
            entry: make_entry(TopologyMode::Singleton, vec![svc_id("sole")], None),
        });

        let resolver = LogicalResolver::new(mock.clone());
        let key = local_key("app-1", "auth");

        // First resolve -> miss -> calls get
        resolver.resolve(&key, None).unwrap();
        assert_eq!(mock.call_count.load(Ordering::Relaxed), 1);

        // Second resolve -> hit -> should NOT call get
        resolver.resolve(&key, None).unwrap();
        assert_eq!(mock.call_count.load(Ordering::Relaxed), 1, "Cache hit must bypass registry");
    }

    #[test]
    fn test_explicit_invalidate_clears_cache() {
        let inv = Arc::new(StaticInventory::new());
        let key = local_key("app-1", "auth");
        inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None));

        let resolver = LogicalResolver::new(inv.clone());

        // Populate cache.
        let _ = resolver.resolve(&key, None).unwrap();

        // Update registry (same epoch — TTL still valid, would not normally
        // refresh).  After explicit invalidate the new value should be seen.
        inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None));
        resolver.invalidate(&key);

        // Same epoch → cache was just evicted, re-fetch from registry.
        let got = resolver.resolve(&key, None).unwrap();
        assert_eq!(got, svc_id("v2"), "explicit invalidate should evict cache");
    }

    #[test]
    fn register_through_the_resolver_evicts_the_cached_topology() {
        let inv = Arc::new(StaticInventory::new());
        let key = local_key("app-1", "backend");
        inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None));

        let resolver = LogicalResolver::new(inv);

        // Populate the cache with a long TTL, so a plain TTL expiry could
        // never explain a refresh below.
        let got = resolver.resolve(&key, None).unwrap();
        assert_eq!(got, svc_id("v1"));

        // A scale-out: two members now, written through the resolver's own
        // `register`, not the registry directly.
        resolver.register(
            key.clone(),
            make_entry(TopologyMode::Redundant, vec![svc_id("v1"), svc_id("v2")], None),
        );

        // Visible immediately -- not after `cache_ttl` -- because `register`
        // evicted the stale cached copy in the same step.
        let all = resolver.resolve_all(&key).unwrap();
        assert_eq!(all.members, vec![svc_id("v1"), svc_id("v2")]);
    }

    #[test]
    fn test_ttl_expiry_triggers_refresh() {
        // Use a zero-TTL entry to simulate instant expiry.
        let inv = Arc::new(StaticInventory::new());
        let key = local_key("app-1", "auth");
        inv.register(
            key.clone(),
            TopologyEntry {
                cache_ttl: Duration::ZERO,
                ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
            },
        );

        let resolver = LogicalResolver::new(inv.clone());

        // Populate cache (with zero TTL it immediately expires).
        let _ = resolver.resolve(&key, None).unwrap();

        // Update registry.
        inv.register(
            key.clone(),
            TopologyEntry {
                cache_ttl: Duration::ZERO,
                ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
            },
        );

        // TTL is zero → expired → must re-fetch.
        let got = resolver.resolve(&key, None).unwrap();
        assert_eq!(got, svc_id("v2"), "expired TTL should trigger cache refresh");
    }

    // ── resolve_all ──────────────────────────────────────────

    #[test]
    fn test_resolve_all_returns_epoch_snapshot() {
        let members = vec![svc_id("m0"), svc_id("m1")];
        let reg = registry_with(vec![(
            local_key("app-1", "store"),
            TopologyEntry {
                epoch: TopologyEpoch(7),
                ..make_entry(
                    TopologyMode::Sharded,
                    members.clone(),
                    Some(ShardingStrategy::HashSharding),
                )
            },
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "store");

        let all = resolver.resolve_all(&key).unwrap();
        assert_eq!(all.topology_epoch, TopologyEpoch(7));
        assert_eq!(all.members, members);
    }

    #[test]
    fn test_resolve_all_unregistered_returns_error() {
        let reg = Arc::new(StaticInventory::new());
        let resolver = LogicalResolver::new(reg);
        let err = resolver.resolve_all(&local_key("ghost", "svc"));
        assert!(err.is_err());
    }

    // ── TopologyEntry serialization ──────────────────────────

    #[test]
    fn test_topology_entry_serialization_roundtrip() {
        let entry = TopologyEntry {
            mode: TopologyMode::Sharded,
            members: vec![svc_id("a"), svc_id("b")],
            sharding_strategy: Some(ShardingStrategy::EntityTagSharding),
            epoch: TopologyEpoch(42),
            cache_ttl: Duration::from_secs(120),
            not_after: Some(1_800_000_000),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let decoded: TopologyEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, decoded);
    }

    // ── AppScope / TopologyKey / not_after (S2) ──────────────

    /// Failure-matrix row 8: two unrelated apps both called `chat` must not
    /// collide -- each is keyed by its own app DID, a disjoint namespace
    /// from `AppScope::Local`.
    #[test]
    fn two_foreign_apps_with_the_same_instance_id_do_not_collide() {
        let reg = registry_with(vec![
            (
                foreign_key("appA", "chat"),
                make_entry(TopologyMode::Singleton, vec![svc_id("member-a")], None),
            ),
            (
                foreign_key("appB", "chat"),
                make_entry(TopologyMode::Singleton, vec![svc_id("member-b")], None),
            ),
        ]);
        let resolver = LogicalResolver::new(reg);

        assert_eq!(
            resolver.resolve(&foreign_key("appA", "chat"), None).unwrap(),
            svc_id("member-a")
        );
        assert_eq!(
            resolver.resolve(&foreign_key("appB", "chat"), None).unwrap(),
            svc_id("member-b")
        );
    }

    #[test]
    fn a_local_entry_and_a_foreign_entry_with_the_same_service_name_are_distinct() {
        let reg = registry_with(vec![
            (
                local_key("app-1", "auth"),
                make_entry(TopologyMode::Singleton, vec![svc_id("local-member")], None),
            ),
            (
                foreign_key("app-1", "auth"),
                make_entry(TopologyMode::Singleton, vec![svc_id("foreign-member")], None),
            ),
        ]);
        let resolver = LogicalResolver::new(reg);

        assert_eq!(
            resolver.resolve(&local_key("app-1", "auth"), None).unwrap(),
            svc_id("local-member"),
            "expected the local entry to resolve independently of the foreign one"
        );
    }

    /// Matrix row 6, checked on both the registry-read path and the
    /// cache-hit path -- an entry past `not_after` must fail, not keep
    /// answering from a warm cache.
    #[test]
    fn an_entry_past_its_not_after_stops_resolving() {
        // Registry path: an already-expired entry is never even cached.
        let past = unix_now().saturating_sub(3600);
        let inv = Arc::new(StaticInventory::new());
        let key = foreign_key("app-1", "svc");
        inv.register(
            key.clone(),
            TopologyEntry {
                not_after: Some(past),
                ..make_entry(TopologyMode::Singleton, vec![svc_id("m1")], None)
            },
        );
        let resolver = LogicalResolver::new(inv.clone());
        let err = resolver.resolve(&key, None).unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");

        // Cache-hit path: an entry valid when cached (a long `cache_ttl`,
        // a `not_after` a moment away) must stop answering once real time
        // carries it past `not_after`, with no registry re-read involved.
        // A 1s margin here raced `unix_now()`'s own second boundary under
        // load (the immediate "still valid" resolve could land exactly on
        // it); 3s leaves two full seconds of slack regardless of where in
        // its current second `register` happens to land.
        let key2 = foreign_key("app-1", "svc2");
        inv.register(
            key2.clone(),
            TopologyEntry {
                not_after: Some(unix_now() + 3),
                ..make_entry(TopologyMode::Singleton, vec![svc_id("m2")], None)
            },
        );
        assert!(resolver.resolve(&key2, None).is_ok(), "warms the cache while still valid");
        std::thread::sleep(Duration::from_millis(3200));
        let err2 = resolver.resolve(&key2, None).unwrap_err();
        assert!(err2.to_string().contains("expired"), "{err2}");
    }

    /// The absent-means-current-behavior property: every intra-app binding
    /// entry has `not_after: None` and must resolve exactly as it does
    /// today, with no expiry check ever tripping.
    #[test]
    fn an_entry_with_no_not_after_resolves_as_it_does_today() {
        let reg = registry_with(vec![(
            local_key("app-1", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None),
        )]);
        let resolver = LogicalResolver::new(reg);
        assert_eq!(resolver.resolve(&local_key("app-1", "auth"), None).unwrap(), svc_id("v1"));
    }

    /// `not_after` is excluded from `classify_binding_write`'s content
    /// comparison, the same way `cache_ttl` already is.
    #[test]
    fn a_not_after_difference_at_one_epoch_is_not_a_binding_conflict() {
        let held = TopologyEntry {
            not_after: Some(1_000),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        };
        let incoming = TopologyEntry {
            not_after: Some(2_000),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        };
        assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
    }

    // ── Performance: cache-hit latency budget ────────────────

    #[test]
    fn test_cache_hit_latency_under_100ns() {
        let members = vec![svc_id("only")];
        let reg = registry_with(vec![(
            local_key("app-perf", "svc"),
            make_entry(TopologyMode::Singleton, members, None),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key_ref = local_key("app-perf", "svc");
        let key = b"hot-routing-key";

        // Warm the cache.
        resolver.resolve(&key_ref, Some(key)).unwrap();

        // Measure 1000 cache-hit resolutions.
        let start = Instant::now();
        for _ in 0..1000 {
            resolver.resolve(&key_ref, Some(key)).unwrap();
        }
        let elapsed = start.elapsed();
        let per_call_ns = elapsed.as_nanos() / 1000;

        // The architecture budget of <100ns per cache-hit is a release-build
        // target.  Debug builds run unoptimized and cannot reliably meet it.
        // We enforce the strict budget only in release mode.
        #[cfg(not(debug_assertions))]
        assert!(
            per_call_ns < 100,
            "cache-hit resolution averaged {per_call_ns}ns, expected <100ns"
        );
        // In debug mode, assert a much more generous bound (10 µs) to at least
        // confirm the code path is exercised without excessive overhead.
        #[cfg(debug_assertions)]
        assert!(
            per_call_ns < 10_000,
            "cache-hit resolution averaged {per_call_ns}ns, expected <10µs in debug mode"
        );
    }

    #[test]
    fn test_independent_round_robin() {
        let members_a = vec![svc_id("a1"), svc_id("a2")];
        let members_b = vec![svc_id("b1"), svc_id("b2")];
        let reg = registry_with(vec![
            (
                local_key("app", "svc_a"),
                make_entry(TopologyMode::Redundant, members_a.clone(), None),
            ),
            (
                local_key("app", "svc_b"),
                make_entry(TopologyMode::Redundant, members_b.clone(), None),
            ),
        ]);
        let resolver = LogicalResolver::new(reg);

        let ref_a = local_key("app", "svc_a");
        let ref_b = local_key("app", "svc_b");

        // Resolving A should not affect B's counter
        assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[0]);
        assert_eq!(resolver.resolve(&ref_b, None).unwrap(), members_b[0]);
        assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[1]);
        assert_eq!(resolver.resolve(&ref_b, None).unwrap(), members_b[1]);
        assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[0]);
    }

    #[test]
    fn test_range_sharding_validation() {
        // Valid table
        let valid_table = RangeRoutingTable {
            chunks: vec![
                RangeChunk {
                    start_key: None,
                    end_key: Some(b"bar".to_vec()),
                    target: svc_id("shard-1"),
                },
                RangeChunk {
                    start_key: Some(b"bar".to_vec()),
                    end_key: None,
                    target: svc_id("shard-2"),
                },
            ],
        };
        assert!(valid_table.validate().is_ok());

        // Empty table is invalid
        let empty_table = RangeRoutingTable { chunks: vec![] };
        assert!(empty_table.validate().is_err());

        // First chunk doesn't start at -infinity
        let bad_first = RangeRoutingTable {
            chunks: vec![RangeChunk {
                start_key: Some(b"bar".to_vec()),
                end_key: None,
                target: svc_id("shard-1"),
            }],
        };
        assert!(bad_first.validate().is_err());

        // Last chunk doesn't end at +infinity
        let bad_last = RangeRoutingTable {
            chunks: vec![
                RangeChunk {
                    start_key: None,
                    end_key: Some(b"bar".to_vec()),
                    target: svc_id("shard-1"),
                },
                RangeChunk {
                    start_key: Some(b"bar".to_vec()),
                    end_key: Some(b"foo".to_vec()),
                    target: svc_id("shard-2"),
                },
            ],
        };
        assert!(bad_last.validate().is_err());

        // Gap / overlap in between chunks
        let gap_table = RangeRoutingTable {
            chunks: vec![
                RangeChunk {
                    start_key: None,
                    end_key: Some(b"bar".to_vec()),
                    target: svc_id("shard-1"),
                },
                RangeChunk {
                    start_key: Some(b"baz".to_vec()),
                    end_key: None,
                    target: svc_id("shard-2"),
                },
            ],
        };
        assert!(gap_table.validate().is_err());

        // Chunk with start_key >= end_key is invalid
        let bad_order = RangeRoutingTable {
            chunks: vec![
                RangeChunk {
                    start_key: None,
                    end_key: Some(b"foo".to_vec()),
                    target: svc_id("shard-1"),
                },
                RangeChunk {
                    start_key: Some(b"foo".to_vec()),
                    end_key: Some(b"bar".to_vec()), // "foo" > "bar"
                    target: svc_id("shard-2"),
                },
                RangeChunk {
                    start_key: Some(b"bar".to_vec()),
                    end_key: None,
                    target: svc_id("shard-3"),
                },
            ],
        };
        assert!(bad_order.validate().is_err());
    }

    // ── classify_binding_write (M05A A5a) ────────────────────

    #[test]
    fn a_higher_epoch_applies() {
        let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
        let incoming = TopologyEntry {
            epoch: TopologyEpoch(1),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
        };
        assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::Applied);
    }

    #[test]
    fn an_equal_epoch_with_identical_members_is_a_no_op() {
        let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
        let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
        assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
    }

    #[test]
    fn an_equal_epoch_with_different_members_is_a_conflict() {
        let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
        let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None);
        assert_eq!(
            classify_binding_write(Some(&held), &incoming),
            BindingWriteOutcome::Conflict(TopologyEpoch::default())
        );
    }

    #[test]
    fn a_lower_epoch_is_stale() {
        let held = TopologyEntry {
            epoch: TopologyEpoch(2),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
        };
        let incoming = TopologyEntry {
            epoch: TopologyEpoch(1),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        };
        assert_eq!(
            classify_binding_write(Some(&held), &incoming),
            BindingWriteOutcome::Stale(TopologyEpoch(2))
        );
    }

    #[test]
    fn an_absent_entry_applies() {
        let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
        assert_eq!(classify_binding_write(None, &incoming), BindingWriteOutcome::Applied);
    }

    #[test]
    fn a_cache_ttl_difference_at_one_epoch_is_not_a_conflict() {
        let held = TopologyEntry {
            cache_ttl: Duration::from_secs(60),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        };
        let incoming = TopologyEntry {
            cache_ttl: Duration::from_secs(120),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        };
        assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
    }

    #[test]
    fn test_range_sharding_routing() {
        let table = RangeRoutingTable {
            chunks: vec![
                RangeChunk {
                    start_key: None,
                    end_key: Some(b"bar".to_vec()),
                    target: svc_id("shard-1"),
                },
                RangeChunk {
                    start_key: Some(b"bar".to_vec()),
                    end_key: Some(b"foo".to_vec()),
                    target: svc_id("shard-2"),
                },
                RangeChunk {
                    start_key: Some(b"foo".to_vec()),
                    end_key: None,
                    target: svc_id("shard-3"),
                },
            ],
        };

        let members = vec![svc_id("shard-1"), svc_id("shard-2"), svc_id("shard-3")];
        let reg = registry_with(vec![(
            local_key("app-1", "range-service"),
            make_entry(
                TopologyMode::Sharded,
                members,
                Some(ShardingStrategy::RangeSharding(table)),
            ),
        )]);
        let resolver = LogicalResolver::new(reg);
        let key = local_key("app-1", "range-service");

        // "a" < "bar" -> shard-1
        assert_eq!(resolver.resolve(&key, Some(b"a")).unwrap(), svc_id("shard-1"));
        // "bar" -> shard-2 (start_key inclusive)
        assert_eq!(resolver.resolve(&key, Some(b"bar")).unwrap(), svc_id("shard-2"));
        // "baz" -> shard-2 ("bar" <= "baz" < "foo")
        assert_eq!(resolver.resolve(&key, Some(b"baz")).unwrap(), svc_id("shard-2"));
        // "foo" -> shard-3 (start_key inclusive)
        assert_eq!(resolver.resolve(&key, Some(b"foo")).unwrap(), svc_id("shard-3"));
        // "z" -> shard-3
        assert_eq!(resolver.resolve(&key, Some(b"z")).unwrap(), svc_id("shard-3"));
    }
}

use std::{
    cmp, error, fmt,
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Error, Result, anyhow};
use serde::{Deserialize, Serialize};

pub use crate::models::{AppDid, AppInstanceId, LogicalServiceName, ServiceId, TopologyMode};

// ─────────────────────────────────────────────────────────────
// Domain types
// ─────────────────────────────────────────────────────────────

/// Default cache TTL for a binding written at deploy time, matching
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
                return Err(anyhow!("Range chunk {i} has start_key >= end_key"));
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
    /// Unix seconds after which this entry must stop resolving (ADR-0022 §3):
    /// past `not_after`, fail -- not "stale but usable". `None` for an entry
    /// pushed by the intra-app binding path, which has no expiry and is
    /// refreshed by a later push.
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
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
/// text -- `AppHostResolver::resolve_app_host` is the reason this
/// exists: treating every `resolve` error as a cache miss made a permanent
/// selection failure (e.g. a `Sharded` service called with no routing key)
/// refetch Tier 2 on every single request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryableResolveError {
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

pub(crate) fn expired_error(key: &TopologyKey, not_after: u64, now: u64) -> anyhow::Error {
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

/// Which app a topology entry belongs to (ADR-0022 §1).
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

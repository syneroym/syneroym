use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::util;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoymRole {
    pub ui_bundle_path: Option<PathBuf>,
    pub owner_did: Option<String>,
}

fn default_podman_path() -> String {
    "podman".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PodmanSandboxRole {
    pub podman_path: String,
}

impl Default for PodmanSandboxRole {
    fn default() -> Self {
        Self { podman_path: default_podman_path() }
    }
}

const fn default_wasm_sandbox() -> bool {
    true
}
const fn default_cpu_limit() -> u32 {
    1
}
fn default_memory_limit() -> String {
    "1Gi".to_string()
}
const fn default_max_concurrent_instances() -> u32 {
    10
}
/// Deliberately generous: `100 * default_max_concurrent_instances() (10) ==
/// 1000`, i.e. this reproduces Wasmtime's own pre-existing pool-wide default
/// (`PoolingAllocationConfig`'s `total_core_instances`/`total_memories`/
/// `total_tables` all default to 1000) for a role that only sets
/// `max_concurrent_instances` and leaves these three at their defaults --
/// deliberately *not* narrowed to a workload-specific number here (a
/// deployed component's real shape isn't known at this layer), so an
/// existing config that never mentions these new fields keeps behaving
/// exactly as it did before they existed. It still adds real, if generous,
/// protection: an explicit, enforced per-component ceiling in place of
/// Wasmtime's own unbounded (`u32::MAX`) per-component default. Individual
/// deployments/tests that know their actual component shape (e.g.
/// `crates/router/tests/proxy_dispatch.rs`'s `test_substrate_config`, which
/// measured its real fixtures at 3 core modules each) should override these
/// with a tighter, validated number -- see
/// `AppSandboxEngine`'s `a_component_at_the_configured_per_component_
/// resource_max_still_instantiates` test for how to validate one.
const fn default_max_core_instances_per_component() -> u32 {
    100
}
const fn default_max_memories_per_component() -> u32 {
    100
}
const fn default_max_tables_per_component() -> u32 {
    100
}
const fn default_dispatch_epoch_timeout_secs() -> u64 {
    5
}
const fn default_lifecycle_hook_epoch_timeout_secs() -> u64 {
    30
}
const fn default_abac_max_instructions() -> u64 {
    50_000_000
}
const fn default_abac_epoch_timeout_secs() -> u64 {
    2
}
/// A browser page issues several parallel requests against one service, and
/// exhausting Wasmtime's pool is a hard error at instantiation, not a wait.
/// Bounded per-service queuing keeps that failure mode
/// local to the one overloaded service rather than starving every other
/// caller's share of the global pool.
const fn default_max_concurrent_guest_http_per_service() -> u32 {
    4
}

const fn default_max_concurrent_websockets_per_service() -> u32 {
    50
}

const fn default_max_sse_subscribers_per_service() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSandboxRole {
    /// Enables the WASM component sandbox.
    pub wasm_sandbox: bool,
    pub cpu_limit: u32,
    pub memory_limit: String,
    pub max_concurrent_instances: u32,
    /// Per-component ceiling on embedded core-module instances (e.g. the
    /// guest module plus a WASI adapter/fixup shim), Wasmtime linear
    /// memories, and tables, respectively. Wasmtime's pooling allocator
    /// leaves these unbounded per component by default, so the *global*
    /// pool totals (`max_concurrent_instances` times each of these) are the
    /// only thing standing between a component and the whole pool -- a
    /// component that transitively needs more of one of these than its
    /// declared max fails to instantiate with a clear Wasmtime error at
    /// deploy/dispatch time rather than starving its neighbors silently.
    pub max_core_instances_per_component: u32,
    pub max_memories_per_component: u32,
    pub max_tables_per_component: u32,
    pub default_max_instructions: Option<u64>,
    pub default_max_memory_bytes: Option<u64>,
    /// Wall-clock budget (Wasmtime epoch interruption) for an ordinary
    /// dispatch call -- RPC/proxy invocation, message delivery, or one
    /// streaming chunk. Tight by design: this is the hot path a stuck or
    /// hostile guest would otherwise hang forever.
    pub dispatch_epoch_timeout_secs: u64,
    /// Wall-clock budget for a component's `init()`/`migrate()` lifecycle
    /// hook (`AppSandboxEngine::invoke_lifecycle_hook`, called once per
    /// deploy). Deliberately larger than `dispatch_epoch_timeout_secs`: this
    /// hook does real one-time work (e.g. `store::create_collection`
    /// opening the service's SQLCipher DB), not a hot path repeatedly hit by
    /// a request, so a generous budget doesn't trade away the same
    /// protection the tighter dispatch budget buys.
    pub lifecycle_hook_epoch_timeout_secs: u64,
    /// Fuel ceiling for one stage-4 ABAC after-step invocation (ADR-0017 §7's
    /// "fuel-metered"). Deliberately a small fraction of
    /// `default_max_instructions`: the after-step runs once per read on the
    /// hot path, and the ADR's optional read-only lookups are the thing this
    /// bounds. Overrun denies the whole batch, never returns partially-
    /// checked rows. A starting point, to be re-tuned against a measured
    /// `criterion` bench.
    ///
    /// Deliberately **not** `Option<u64>`: the
    /// after-step always overrides the service's own fuel via
    /// `InstanceOptions::fuel_override`, which treats `None` as "keep
    /// whatever the caller already had" -- for every *other*
    /// `fuel_override` use that means "no override", but here it would
    /// silently fall through to the service's own `default_max_instructions`
    /// (10 billion by default, ~200x this field's own default), the exact
    /// opposite of what an operator clearing this field to disable a limit
    /// would expect. A plain `u64` makes that fallback unreachable.
    pub abac_max_instructions: u64,
    /// Wall-clock budget for one after-step. Tighter than
    pub abac_epoch_timeout_secs: u64,
    /// Concurrent guest HTTP requests one service may have in flight.
    /// Enforced per service by a semaphore, sized by this field;
    /// a request that outwaits `GUEST_HTTP_ADMISSION_TIMEOUT` for a permit
    /// gets a 503, never the pool's own hard instantiation error.
    pub max_concurrent_guest_http_per_service: u32,
    /// Concurrent guest websockets one service may have in flight.
    /// Enforced per service by a semaphore, sized by this field.
    pub max_concurrent_websockets_per_service: u32,
    /// Concurrent SSE subscribers one service may have in flight.
    /// Enforced per service by a semaphore, sized by this field;
    /// an immediate try_acquire failure yields a 503 with Retry-After: 1.
    pub max_sse_subscribers_per_service: u32,
    /// The guest proxy outbox worker's own tick. Recovery after an
    /// unreachable target returns is bounded by this, and nothing finer
    /// helps when the wait is for a peer to come back.
    pub queue_tick_secs: u64,
    /// The outbox's attempt budget before an item dead-letters. See
    /// [`default_proxy_queue_max_attempts`] for the ~10-hour window this
    /// and `queue_max_backoff_secs` together produce.
    pub queue_max_attempts: u8,
    /// The ceiling the outbox's backoff curve settles at. The early
    /// retries stay sub-second, so a peer that only blipped is served
    /// immediately.
    pub queue_max_backoff_secs: u64,
    /// How long a claimed outbox item stays invisible to a second claim
    /// before a crashed worker's hold on it is assumed gone.
    pub queue_visibility_timeout_secs: u64,
    /// Dead letters are pruned oldest-first past this row count, within
    /// one target: a permanently broken recipient must not be able to
    /// evict every other conversation's dead letters.
    pub queue_dlq_max_rows: u32,
    /// How many sagas one service may have open at once. Refuses `begin`
    /// above it: an open saga is work somebody expects to finish, so the
    /// bound refuses rather than evicts.
    pub saga_max_open: u32,
    /// How many steps one saga may record. Refuses `step` above it, for the
    /// same reason as `saga_max_open`.
    pub saga_max_steps: u32,
    /// Terminal (`compensated`/`failed`) saga rows are pruned oldest-first
    /// past this count, exactly as `queue_dlq_max_rows` prunes dead letters.
    pub saga_max_terminal_rows: u32,
    /// A `begin` with no explicit deadline takes this many seconds.
    pub saga_default_deadline_secs: u64,
    /// The ceiling a guest may request at `begin`. Above it, `begin` refuses
    /// rather than clamping -- a workflow must not silently run under a
    /// deadline it did not ask for.
    pub saga_max_deadline_secs: u64,
    /// The conversation delivery worker's own tick (beside the
    /// proxy outbox's `queue_tick_secs`).
    pub conversation_tick_secs: u64,
    /// Per-message body cap. A queued payload is stored plaintext under the
    /// service's own DEK and travels on every delivery attempt.
    pub conversation_max_body_bytes: u32,
    /// Ceiling on items waiting for delivery in one conversation. Exceeding
    /// it returns `quota-exceeded` for that conversation only.
    pub conversation_max_pending_per_conversation: u32,
    /// Ceiling on stored messages in one conversation.
    pub conversation_max_messages_per_conversation: u32,
    /// How long a message may wait for an unreachable peer before it moves
    /// from `pending` to `failed`. 30 days: long enough that an offline peer
    /// is not mistaken for a broken one, short enough that an outbox does
    /// not grow forever.
    pub conversation_max_pending_age_secs: u64,
    /// A message whose `sender_timestamp` is this far *ahead* of the
    /// receiver's own clock is refused on arrival — a far-future
    /// timestamp would otherwise pin a message to the top of every
    /// participant's history permanently. Asymmetric on purpose: a
    /// past timestamp is always accepted.
    pub conversation_max_clock_skew_secs: u64,
    /// One-time prekeys held per service, replenished lazily.
    pub conversation_prekey_pool_size: u32,
    /// Per-peer rate limit on `prekey-bundle` requests: each request is a
    /// store write and, once the one-time pool is empty, a keygen, so an
    /// unbounded caller could drain it at no cost to themselves.
    pub conversation_prekey_requests_per_peer_per_hour: u32,
    #[serde(default = "default_conversation_group_sync_secs")]
    pub conversation_group_sync_secs: u64,
    #[serde(default = "default_conversation_group_rekey_secs")]
    pub conversation_group_rekey_secs: u64,
    #[serde(default = "default_conversation_max_group_members")]
    pub conversation_max_group_members: u32,
    #[serde(default = "default_conversation_max_dag_entries_per_conversation")]
    pub conversation_max_dag_entries_per_conversation: u32,
    #[serde(default = "default_conversation_max_sync_entries_per_call")]
    pub conversation_max_sync_entries_per_call: u32,
    #[serde(default = "default_conversation_relay_fanout")]
    pub conversation_relay_fanout: u32,
    #[serde(default = "default_conversation_sync_now_budget_ms")]
    pub conversation_sync_now_budget_ms: u64,
    /// Total time budget for the background periodic sync pass, one
    /// conversation at a time — kept separate from
    /// `conversation_sync_now_budget_ms` because that budget is bounded by
    /// `dispatch_epoch_timeout_secs` for the guest-facing `sync-now` call,
    /// while the background pass has no guest waiting on it and needs
    /// enough time to reach every member of a large group, not just the
    /// first two.
    #[serde(default = "default_conversation_background_sync_budget_ms")]
    pub conversation_background_sync_budget_ms: u64,
}

/// The guest proxy outbox lives wherever a guest does, so its knobs live on
/// the sandbox role rather than on the supervisor's -- a substrate hosting
/// guests may run no supervisor at all.
const fn default_proxy_queue_tick_secs() -> u64 {
    5
}
/// 54 attempts with a 100 ms initial backoff, x2 multiplier and a 900 s
/// ceiling sum to roughly 10.2 hours of retrying. A message queued at 22:00
/// must still be deliverable at 07:00.
const fn default_proxy_queue_max_attempts() -> u8 {
    54
}
const fn default_proxy_queue_max_backoff_secs() -> u64 {
    900
}
/// Four times the proxy's own 30 s per-call budget, which bounds a single
/// delivery attempt. Too short re-delivers work still in flight; too long
/// strands a crashed worker's item for no reason.
const fn default_proxy_queue_visibility_timeout_secs() -> u64 {
    120
}
const fn default_proxy_queue_dlq_max_rows() -> u32 {
    1000
}
/// One workflow per open saga; a service with 64 in flight has a design
/// problem, not a capacity one.
const fn default_saga_max_open() -> u32 {
    64
}
const fn default_saga_max_steps() -> u32 {
    64
}
/// The same number `queue_dlq_max_rows` uses, for the same
/// operator-visibility reason.
const fn default_saga_max_terminal_rows() -> u32 {
    1000
}
/// An hour: long enough for a human-paced multi-provider workflow, short
/// enough that a crashed one compensates the same day.
const fn default_saga_default_deadline_secs() -> u64 {
    3600
}
/// A day. Both are honest first guesses, not measurements -- config fields,
/// so a deployment that finds them wrong changes them without a rebuild.
const fn default_saga_max_deadline_secs() -> u64 {
    86400
}

const fn default_conversation_tick_secs() -> u64 {
    5
}
const fn default_conversation_max_body_bytes() -> u32 {
    262_144
}
const fn default_conversation_max_pending_per_conversation() -> u32 {
    1_000
}
const fn default_conversation_max_messages_per_conversation() -> u32 {
    100_000
}
/// 30 days.
const fn default_conversation_max_pending_age_secs() -> u64 {
    2_592_000
}
/// 24 hours.
const fn default_conversation_max_clock_skew_secs() -> u64 {
    86_400
}
const fn default_conversation_prekey_pool_size() -> u32 {
    100
}
const fn default_conversation_prekey_requests_per_peer_per_hour() -> u32 {
    20
}
const fn default_conversation_group_sync_secs() -> u64 {
    60
}
const fn default_conversation_group_rekey_secs() -> u64 {
    604_800
}
const fn default_conversation_max_group_members() -> u32 {
    256
}
const fn default_conversation_max_dag_entries_per_conversation() -> u32 {
    100_000
}
const fn default_conversation_max_sync_entries_per_call() -> u32 {
    64
}
const fn default_conversation_relay_fanout() -> u32 {
    3
}
const fn default_conversation_sync_now_budget_ms() -> u64 {
    3_000
}
/// 10 seconds per peer times a generous 16-member allowance;
/// members past that still get reached on a later tick since the pass
/// rotates its starting member each time.
const fn default_conversation_background_sync_budget_ms() -> u64 {
    160_000
}

impl AppSandboxRole {
    #[must_use]
    pub fn memory_limit_bytes(&self) -> u64 {
        util::parse_size_string(&self.memory_limit, 128 * 1024 * 1024)
    }
}

impl Default for AppSandboxRole {
    fn default() -> Self {
        Self {
            wasm_sandbox: default_wasm_sandbox(),
            cpu_limit: default_cpu_limit(),
            memory_limit: default_memory_limit(),
            max_concurrent_instances: default_max_concurrent_instances(),
            max_core_instances_per_component: default_max_core_instances_per_component(),
            max_memories_per_component: default_max_memories_per_component(),
            max_tables_per_component: default_max_tables_per_component(),
            default_max_instructions: Some(10_000_000_000),
            default_max_memory_bytes: Some(256 * 1024 * 1024),
            dispatch_epoch_timeout_secs: default_dispatch_epoch_timeout_secs(),
            lifecycle_hook_epoch_timeout_secs: default_lifecycle_hook_epoch_timeout_secs(),
            abac_max_instructions: default_abac_max_instructions(),
            abac_epoch_timeout_secs: default_abac_epoch_timeout_secs(),
            max_concurrent_guest_http_per_service: default_max_concurrent_guest_http_per_service(),
            max_concurrent_websockets_per_service: default_max_concurrent_websockets_per_service(),
            max_sse_subscribers_per_service: default_max_sse_subscribers_per_service(),
            queue_tick_secs: default_proxy_queue_tick_secs(),
            queue_max_attempts: default_proxy_queue_max_attempts(),
            queue_max_backoff_secs: default_proxy_queue_max_backoff_secs(),
            queue_visibility_timeout_secs: default_proxy_queue_visibility_timeout_secs(),
            queue_dlq_max_rows: default_proxy_queue_dlq_max_rows(),
            saga_max_open: default_saga_max_open(),
            saga_max_steps: default_saga_max_steps(),
            saga_max_terminal_rows: default_saga_max_terminal_rows(),
            saga_default_deadline_secs: default_saga_default_deadline_secs(),
            saga_max_deadline_secs: default_saga_max_deadline_secs(),
            conversation_tick_secs: default_conversation_tick_secs(),
            conversation_max_body_bytes: default_conversation_max_body_bytes(),
            conversation_max_pending_per_conversation:
                default_conversation_max_pending_per_conversation(),
            conversation_max_messages_per_conversation:
                default_conversation_max_messages_per_conversation(),
            conversation_max_pending_age_secs: default_conversation_max_pending_age_secs(),
            conversation_max_clock_skew_secs: default_conversation_max_clock_skew_secs(),
            conversation_prekey_pool_size: default_conversation_prekey_pool_size(),
            conversation_prekey_requests_per_peer_per_hour:
                default_conversation_prekey_requests_per_peer_per_hour(),
            conversation_group_sync_secs: default_conversation_group_sync_secs(),
            conversation_group_rekey_secs: default_conversation_group_rekey_secs(),
            conversation_max_group_members: default_conversation_max_group_members(),
            conversation_max_dag_entries_per_conversation:
                default_conversation_max_dag_entries_per_conversation(),
            conversation_max_sync_entries_per_call: default_conversation_max_sync_entries_per_call(
            ),
            conversation_relay_fanout: default_conversation_relay_fanout(),
            conversation_sync_now_budget_ms: default_conversation_sync_now_budget_ms(),
            conversation_background_sync_budget_ms: default_conversation_background_sync_budget_ms(
            ),
        }
    }
}

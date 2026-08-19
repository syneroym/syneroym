//! The `supervisor` `NativeService`: dispatches every verb in
//! `supervisor.wit` (submit / adopt / release / pause / resume / retire /
//! force-reconcile / export-master / import-master / status / alerts).
//!
//! Every verb gates on `substrate/admin` on this supervisor's own node
//! (§11.2): submitting desired state hands the supervisor deploy authority
//! on N remote substrates and master keys, and there is no resource
//! narrower than the node that means anything here. `status`/`alerts` are a
//! coarse stand-in for a future monitoring-only credential -- recorded in
//! the deferred backlog, not built here.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use serde_json::Value;
use syneroym_app_orchestration::{
    ActionRecord, AlertKind, DeploymentState, MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES,
    ReconcileAction, Reconciler, ShardingStrategy, SignedTopologyDocument, TopologyDocument,
    TopologyEpoch, TopologyVisibility, has_occurrence_in,
    models::{
        AppDid, AppInstanceId, DeploymentPlan, LogicalServiceName, LogicalServiceRef, MAX_REPLICAS,
        MemberRef, PlannedService, RotationPolicy, ScheduleSpec, ServiceId, SubstrateAlias,
    },
    topology_fingerprint, validate_plan_visibility,
};
use syneroym_async_queue::{FailOutcome, QueueItem};
use syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;
use syneroym_core::dht_registry::DEFAULT_ENDPOINT_NOT_AFTER_SECS;
use syneroym_identity::{
    Identity,
    delegation::{is_expired_parts, is_near_expiry_parts},
};
use syneroym_mqtt_broker::{MqttBroker, namespace_topic_for_publish};
use syneroym_rpc::{
    Ability, CallerContext, NativeInvocation, NativeResponse, NativeService,
    PERMISSION_DENIED_CODE, ResourceUri, RpcError, RpcResult,
};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, SyneroymClient,
    deploy::{
        self, ApplyRequest, DeployTarget, SubstrateActor, WriteBindingsAttempt, WriteBindingsOutbox,
    },
    health::{self, ExpectedService, HealthTarget, Signal, StatusQuery},
    mapper::map_deployment_plan_to_wit,
};
use syneroym_wit_interfaces::supervisor::exports::syneroym::supervisor::supervisor::{
    AdoptResult, Alert, BindingConvergence, DeadLetter, InstanceStatus, ManagedService,
    ManagedState, MintedMaster as WitMintedMaster, OutboxItem, ScheduledTask, Submission,
    SubmitResult,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::{
    AnchorWriter, MasterVault, MintedMaster, Tier1Writer,
    inventory::{SupervisorInventory, SupervisorInventoryEntry},
    keys, outbox,
    outbox::{QueueKey, SupervisorOutbox},
    store::{DesiredState, RemediationState, ScheduleState, SupervisorStore},
    tier1, topology,
    topology::TopologyBuildError,
};

const SUPERVISOR_INTERFACE: &str = "supervisor";
/// Ceiling for connecting to one managed substrate -- generous relative to
/// `PREFLIGHT_TIMEOUT`'s 5s in `roymctl`, since a supervisor call may fan
/// out to several substrates concurrently rather than being a single
/// operator-watched command.
const MANAGED_SUBSTRATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The `substrate_did` D-A5c-10's "planned but never landed"
/// `InstanceNotRunning` alert is keyed under -- deliberately not the
/// empty string `record_report`'s own per-service loop uses for a
/// `Signal::NotDeployed` service, which would otherwise clear this exact
/// alert on every pass right before it gets re-raised (see the call
/// site's own comment).
const NEVER_LANDED_SUBSTRATE_DID: &str = "supervisor:never-landed";
/// The `substrate_did` a `ScheduledRunFailed` alert is keyed under.
/// A schedule belongs to a logical service, and its target
/// rotates across members that may be on different substrates -- keying the
/// alert under whichever one ran the failing tick would mean the next
/// tick's success clears a different row and the failure stands forever.
/// The substrate that actually ran the tick goes in the alert's `detail`,
/// where it is information rather than identity.
const SCHEDULE_SUBSTRATE_DID: &str = "supervisor:schedule";
/// A manifest's own `ScheduleSpec.timeout_ms` decides one run's budget;
/// this is the ceiling it cannot exceed. 30s rather
/// than something generous, because this call is awaited inline inside a
/// reconcile pass that holds the instance lock and runs instances one at a
/// time -- every second here is a second every *other* app instance waits.
/// A budget above the guest's own `dispatch_epoch_timeout_secs` (5s) buys
/// no extra work, only a longer wait on a substrate that is not answering.
///
/// `SynAppManifest::validate` refuses anything above this outright, so the
/// clamp at the call site only ever fires for a hand-edited plan that never
/// passed through validation -- which is why both read the same constant.
const SCHEDULED_RUN_CEILING: Duration = Duration::from_millis(MAX_SCHEDULE_TIMEOUT_MS as u64);
/// This supervisor's own `record_report` calls -- the resident loop's
/// pass and `handle_status`'s on-demand sweep alike -- are the sole
/// producer of `CertificateNearExpiry`/`CertificateExpired` for their own
/// instances (`raise_renewal_stalled`, `clear_settled_renewal_alerts`).
/// One named constant, not two independent `CertAlertPolicy::
/// ManagedElsewhere` literals: the two call sites drifting apart (one
/// left on `Reminder`) is exactly how the double-producer bug this
/// constant exists to prevent got in undetected the first time.
const SUPERVISOR_CERT_ALERT_POLICY: health::CertAlertPolicy =
    health::CertAlertPolicy::ManagedElsewhere;

/// One pass's write half, as arguments. A struct rather than nine
/// positional parameters because A5d adds a fourth work-list to a signature
/// that was already at the edge of readable.
struct WritePhase<'a> {
    instance_id: &'a AppInstanceId,
    app_instance_id: &'a str,
    plan: &'a DeploymentPlan,
    needs_work: &'a BTreeSet<String>,
    /// `(logical_ref, service_id, substrate_did)`, as `restart_candidates`
    /// produces them.
    restart_candidates: &'a [(String, String, String)],
    renewal_candidates: &'a [RenewalCandidate],
    pending_rotation_restarts: &'a BTreeSet<String>,
    /// Dependent members whose diff against the last active plan changed
    /// only `resolved_dependencies` (M05A A5e, D-A5e-7): a membership
    /// change in one of their dependencies, routed to `push_bindings`
    /// instead of a full redeploy. `(member's own planned service, the
    /// substrate DID it is already landed on)`.
    push_candidates: &'a [(PlannedService, String)],
    /// The fifth work-list: this pass's decision for every
    /// schedule the plan declares, computed once by the pure
    /// `schedule_decisions` and carried through unchanged -- see
    /// `ScheduleDecision`'s own doc.
    schedule_decisions: &'a [ScheduleDecision],
    did_to_alias: &'a BTreeMap<String, String>,
    clients: &'a BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    now: u64,
}

/// What this pass owes one schedule (ADR-0023 §6). Computed
/// once per pass by the pure [`SupervisorService::schedule_decisions`],
/// then acted on by [`SupervisorService::run_due_schedules`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum ScheduleDecision {
    /// Looked, nothing due (or nothing runnable: a bad cron, no healthy
    /// member, no known alias for the picked member's substrate). Advance
    /// the watermark only -- this is what makes a missed tick a skip
    /// rather than a backlog.
    Watermark { logical_ref: String },
    Run {
        logical_ref: String,
        service_id: String,
        substrate_did: String,
        member_index: u32,
        schedule: ScheduleSpec,
    },
}

/// One placed member due for certificate renewal this pass, resolved from
/// the pass's own health report.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RenewalCandidate {
    /// This member's own `MemberRef` display string (M05A A5e, D-A5e-2) --
    /// what every store/alert call this candidate feeds keys on.
    member_ref: String,
    service_name: String,
    /// The member master DID -- what the certificate names and what the
    /// substrate knows the service by.
    service_id: String,
    substrate_did: String,
    /// Carried so a failed renewal can tell "not yet expired" from "already
    /// expired" without re-reading the report.
    expires_at: u64,
    /// This member's ordinal (M05A A5e, D-A5e-5) -- what
    /// `keys::master_for_member` reads instead of hardcoding `0`, so
    /// member N's renewal signs with member N's own master.
    member_index: u32,
}

/// Why one member's renewal stopped. `VaultLocked` is carved out from the
/// generic per-step failure because it is one root cause with one operator
/// action, and reporting it under two different alert kinds depending on
/// which of the two checks caught it would defeat the point of raising it
/// at all.
#[derive(Debug)]
enum RenewalFailure {
    VaultLocked,
    Step {
        step: &'static str,
        error: String,
    },
    /// The mint and install both landed; only the `restart-on-rotation`
    /// restart itself failed. Kept distinct from `Step` because the
    /// certificate genuinely renewed -- the next health poll
    /// reports a fresh window, so this must not be reported as a stalled
    /// renewal, and must not be cleared by the renewal alert's own
    /// recomputed-every-pass clearing rule.
    RotationRestart {
        error: String,
    },
}

/// How the queue worker reconnects to a claimed item's target substrate
/// (M05B B1, D-B1-1). A trait rather than a direct call to
/// `SupervisorService::connected_client`, so `deliver_queued_item` is
/// testable against a fake instead of a live substrate -- the same
/// tradeoff `push_bindings`/`attempt_restart` already made by taking an
/// already-connected `&Arc<dyn SubstrateActor>` rather than connecting
/// themselves.
#[async_trait::async_trait]
trait QueueConnector: fmt::Debug + Send + Sync {
    async fn connect(
        &self,
        entry: &SupervisorInventoryEntry,
    ) -> anyhow::Result<Arc<dyn WriteBindingsAttempt>>;
}

/// The production `QueueConnector`: a fresh `SyneroymClient` per delivery,
/// closed by `Drop` rather than an explicit `shutdown()` call -- unlike
/// `connected_client`'s other callers, a queue delivery is one-shot and
/// short-lived, and `Arc<dyn WriteBindingsAttempt>` has no `shutdown` to
/// call even if it mattered here.
#[derive(Debug)]
struct LiveQueueConnector {
    client_identity_bytes: [u8; 32],
    /// Mirrors this node's own `enable_bep0044_dht` (see
    /// `SupervisorService::enable_registry_dht`'s doc): every delivery
    /// attempt builds a fresh `SyneroymClient`, so leaving this on would
    /// spin up a real mainline-DHT client on every queue-worker tick a
    /// target substrate stays unreachable.
    enable_registry_dht: bool,
}

#[async_trait::async_trait]
impl QueueConnector for LiveQueueConnector {
    async fn connect(
        &self,
        entry: &SupervisorInventoryEntry,
    ) -> anyhow::Result<Arc<dyn WriteBindingsAttempt>> {
        let identity = Identity::from_bytes(&self.client_identity_bytes);
        let mut client = SyneroymClient::new_with_identity(
            entry.did.clone(),
            entry.api_url.clone().unwrap_or_default(),
            identity,
        )
        .with_registry_dht(self.enable_registry_dht);
        if let Some(token) = &entry.ucan {
            client = client.with_ucan(token.clone());
        }
        client.wait_for_ready(MANAGED_SUBSTRATE_CONNECT_TIMEOUT).await?;
        Ok(Arc::new(client))
    }
}

/// What one call to [`SupervisorService::push_bindings`] actually did.
/// Not a bare `Vec<BindingWriteOutcome>`, because that is genuinely
/// ambiguous: a *real, attempted* write can legitimately carry zero
/// outcomes when it has zero bindings to send (every `depends_on` a
/// service declared was just removed from its manifest -- an ordinary
/// deploy, not an edge case), and an earlier version of this code used
/// exactly `Vec::new()` as its own sentinel for "not attempted, deferred
/// to an already-pending queue item" (M05B B1 review, two review passes
/// deep: the first found the epoch-skew this sentinel was meant to fix; a
/// second found the sentinel itself was ambiguous with a real, empty
/// success, permanently downgrading a service that had simply lost its
/// last dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PushOutcome {
    /// The write was actually attempted -- successfully or not, and
    /// possibly with zero outcomes if `svc` had zero bindings to send.
    Landed(Vec<BindingWriteOutcome>),
    /// Not attempted this call: an item for this exact key is already
    /// durably queued, so this call defers to it entirely.
    Deferred,
}

/// The signed document for one logical service, plus the epoch it was
/// signed at -- what `SupervisorService::signed_documents`'s in-process
/// cache holds.
#[derive(Debug, Clone)]
struct CachedDocument {
    signed: SignedTopologyDocument,
    epoch: u64,
}

pub struct SupervisorService {
    node_did: String,
    store: SupervisorStore,
    vault: MasterVault,
    /// The identity this supervisor presents when it connects, as a
    /// client, to the substrates it manages (ADR-0021 §8: "a client of
    /// substrates, not a server to services"). Stored as raw key bytes
    /// rather than an `Identity` because `Identity` deliberately does not
    /// implement `Clone` -- a fresh `Identity` is reconstructed per
    /// outbound connection.
    client_identity_bytes: [u8; 32],
    /// This node's own `SubstrateConfig.substrate.enable_bep0044_dht`
    /// (ADR-0021 §8: the supervisor is itself a client of the substrates
    /// it manages, so its outbound connections should honor the same DHT
    /// policy this node applies to its own registry publishing). Passed
    /// to every `SyneroymClient` this service builds -- `connected_client`
    /// and `LiveQueueConnector` alike -- so a node with DHT disabled
    /// doesn't spin one up on the client side instead.
    enable_registry_dht: bool,
    /// D-A5c-6 (§19.5): this node's shared broker, registered under
    /// `SUPERVISOR_DISPATCH_ID` (`runtime.rs`) so `record_report`'s caller
    /// can publish a newly-opened alert without a deployed service in the
    /// way.
    messaging_broker: Arc<MqttBroker>,
    /// `SupervisorRole.alert_topic` (default `supervisor/alerts`) --
    /// D-A5-13's prefix, joined with the app instance id at publish time.
    alert_topic: String,
    /// `SupervisorRole.poll_interval_secs` (default 30) -- the resident
    /// loop's `tokio::interval` period (M05A A5c §19.7/§19.14, D-A5c-7).
    poll_interval_secs: u64,
    /// `SupervisorRole.max_restart_attempts` (default 3) -- the bounded
    /// restart-in-place ceiling (§19.14, phase 6).
    max_restart_attempts: u32,
    /// `SupervisorRole.restart_backoff_secs` (default 30) -- minimum wait
    /// between two restart attempts for one service (§19.14, phase 6).
    restart_backoff_secs: u64,
    /// `SupervisorRole.renewed_cert_expires_hours` (default 4) -- the
    /// lifetime *every* instance certificate this supervisor mints carries,
    /// the first one at deploy and every renewal alike, so a managed member
    /// has one certificate lifetime for its whole life rather than a long
    /// first one followed by short renewals. Deliberately not `roymctl`'s
    /// own attended-posture default, which serves an operator with no
    /// renewal loop behind them.
    renewed_cert_expires_hours: u64,
    /// `SupervisorRole.max_renewals_per_pass` (default 5) -- how many
    /// members one pass may renew before deferring the rest to the next
    /// one. See the config field's own doc for why renewal, alone among
    /// the pass's work-lists, needs a cap.
    max_renewals_per_pass: u32,
    /// `SupervisorRole.master_anchor_refresh_interval_secs` (default 12h).
    master_anchor_refresh_interval_secs: u64,
    /// `SupervisorRole.queue_tick_secs` (default 5s, M05B B1) -- the
    /// durable outbox worker's own `tokio::interval` period, independent of
    /// `poll_interval_secs` (task.md's recovery budget: within one worker
    /// tick, not one poll interval).
    queue_tick_secs: u64,
    /// How the queue worker reconnects to a claimed item's target
    /// substrate (M05B B1). `LiveQueueConnector` in production; a test
    /// substitutes a fake, the same reason `push_bindings`/`attempt_restart`
    /// take an already-connected `&Arc<dyn SubstrateActor>` rather than
    /// connecting themselves.
    queue_connector: Arc<dyn QueueConnector>,
    /// Where master-anchor refreshes and revocations are published.
    /// `None` when this node has no registry configured: an anchor
    /// published nowhere would leave every consumer failing closed on a
    /// record it cannot distinguish from a revoked one, so the supervisor
    /// holds no writer rather than one that quietly does nothing.
    anchor_writer: Option<Arc<dyn AnchorWriter>>,
    /// Where each app instance's Tier-1 registry record is published
    /// (ADR-0022 §2). `None` for the same reason `anchor_writer` above is
    /// -- a single-node deployment with no registry configured does not
    /// use cross-app discovery, and must not be broken to enable it.
    tier1_writer: Option<Arc<dyn Tier1Writer>>,
    /// `SupervisorRole.topology_document_not_after_secs` (ADR-0022 §3).
    /// This is the window a caller with a cached document keeps routing
    /// while this supervisor is down -- the availability property the
    /// document form exists for.
    topology_document_not_after_secs: u64,
    /// `SupervisorRole.topology_document_cache_ttl_secs`, carried inside
    /// the signed document as `cache_ttl_ms` -- advice to the reader, not
    /// authority.
    topology_document_cache_ttl_secs: u64,
    /// Signed Tier-2 documents, keyed `(app_instance_id, service_name)`:
    /// one signature per epoch, not one per request. In process
    /// only -- after a restart the vault is locked anyway, so persisting
    /// these would buy one signature and a durability question. A cached
    /// copy is still served while the vault is locked, which is the
    /// availability property ADR-0022 §3 chose the document form for.
    signed_documents: DashMap<(String, String), CachedDocument>,
    /// A per-app-instance async mutex, held for the whole duration of a
    /// loop pass and for the whole duration of `submit`/`force-reconcile`/
    /// `adopt`/`release`/`retire` -- not `pause`/`resume` (single-column
    /// writes) or `status`/`alerts` (reads) (M05A A5c §19.7, D-A5c-7).
    /// Per-instance rather than global so one unreachable substrate cannot
    /// stall every other instance's loop pass.
    instance_locks: DashMap<String, Arc<AsyncMutex<()>>>,
    /// Unix-seconds timestamp of the last time the *resident loop*
    /// finished a reconcile pass for this instance, keyed by
    /// `app_instance_id` -- distinct from `status`'s own on-demand health
    /// sweep, which does not write here (review finding A-8: this field
    /// used to be hardcoded `None` under a comment claiming no loop
    /// existed yet to fill it). In-memory only, not persisted: a
    /// supervisor restart correctly reports "no pass since restart"
    /// rather than replaying a stale wall-clock time.
    last_reconciled: DashMap<String, i64>,
    /// Unix-seconds timestamp at which the resident loop *began* its
    /// previous sweep, or 0 before the second one starts. Written once per
    /// `run_pass`, after the sweep it timed, so every instance reconciled
    /// inside a sweep reads the start of the one before it -- which is how
    /// far back this supervisor can prove it was awake and looking.
    /// `schedule_grace_secs` is the only reader.
    ///
    /// Process-wide rather than per-instance on purpose. A per-instance gap
    /// (`last_reconciled`) is not stamped while an instance is paused, so it
    /// would report a three-day pause as a three-day gap and fire a
    /// catch-up tick the moment the instance resumed. The loop keeps
    /// sweeping throughout that pause, so the sweep-to-sweep gap stays one
    /// poll interval and a resume fires nothing.
    previous_pass_started_at: AtomicU64,
    /// Cancelled by `shutdown` -- the resident loop (`run`, spawned by
    /// `RuntimeServices::run_until_shutdown`, not pinned in its own
    /// `select!`) watches this to stop between passes rather than being
    /// dropped mid-pass (M05A A5c §19.8, D-A5c-8). The `JoinHandle`
    /// itself is held by `RuntimeServices`, which awaits it after calling
    /// `shutdown` -- cancelling alone does not wait for the pass in
    /// flight to actually finish closing its clients.
    cancellation_token: CancellationToken,
    /// Test-only: keeps a fixture-built service's backing directory alive
    /// for exactly this service's own lifetime (M05A A7 review finding
    /// 4). `Fixture::build_with_key_store` needs the directory to survive
    /// past its own return for the vault's encrypted-mint path to work at
    /// all (an earlier fix that instead called `.keep()` on the
    /// `TempDir`, unconditionally leaking it, is what finding 4 caught);
    /// tying its lifetime to the service it backs, rather than never
    /// dropping it, restores ordinary cleanup while keeping that fix.
    #[cfg(test)]
    _fixture_tempdir: Option<tempfile::TempDir>,
}

impl fmt::Debug for SupervisorService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorService")
            .field("node_did", &self.node_did)
            .finish_non_exhaustive()
    }
}

impl SupervisorService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_did: String,
        store: SupervisorStore,
        vault: MasterVault,
        client_identity: &Identity,
        enable_registry_dht: bool,
        messaging_broker: Arc<MqttBroker>,
        alert_topic: String,
        poll_interval_secs: u64,
        max_restart_attempts: u32,
        restart_backoff_secs: u64,
        renewed_cert_expires_hours: u64,
        max_renewals_per_pass: u32,
        master_anchor_refresh_interval_secs: u64,
        anchor_writer: Option<Arc<dyn AnchorWriter>>,
        tier1_writer: Option<Arc<dyn Tier1Writer>>,
        queue_tick_secs: u64,
        topology_document_not_after_secs: u64,
        topology_document_cache_ttl_secs: u64,
    ) -> Self {
        // `.take(0)` in `renewal_candidates` silently disables renewal for
        // the whole node, with no warning and no config validation to
        // catch it. The unit test on `SupervisorRole` pins the *default*
        // at 1, which does nothing for a configured 0 -- clamped here
        // instead, where every construction path (config-loaded or
        // test-built) goes through the same guard.
        let max_renewals_per_pass = if max_renewals_per_pass == 0 {
            tracing::warn!(
                "supervisor.max_renewals_per_pass was configured to 0, which would renew nothing, \
                 ever; clamped to 1"
            );
            1
        } else {
            max_renewals_per_pass
        };
        // A `0` here makes every signed topology document born expired
        // (`verify` rejects it on issue) while the supervisor keeps
        // signing and serving one afresh per request -- exactly the
        // per-request latency surface D-S2-6's re-sign-at-half-validity
        // cache exists to remove. Same clamp shape as
        // `max_renewals_per_pass` above.
        let topology_document_not_after_secs = if topology_document_not_after_secs == 0 {
            tracing::warn!(
                "supervisor.topology_document_not_after_secs was configured to 0, which would \
                 make every signed topology document expire the instant it is issued; clamped to \
                 3600"
            );
            3_600
        } else {
            topology_document_not_after_secs
        };
        // A `cache_ttl` at or above half of `not_after` breaks D-S2-6's "a
        // served copy always outlives the caller's own cache TTL" -- a
        // reader that re-asks exactly on the advised TTL could then read a
        // document that already expired.
        let topology_document_cache_ttl_secs = if topology_document_cache_ttl_secs.saturating_mul(2)
            >= topology_document_not_after_secs
        {
            // `.max(1)`: `not_after_secs` under 4 would otherwise clamp to
            // 0, and a reader taking that advice as its cache TTL gets
            // `Duration::ZERO`, which never registers a cache hit at all.
            let clamped = (topology_document_not_after_secs / 4).max(1);
            tracing::warn!(
                topology_document_cache_ttl_secs,
                topology_document_not_after_secs,
                "supervisor.topology_document_cache_ttl_secs is at least half of \
                 topology_document_not_after_secs, which breaks the property that a served copy \
                 always outlives the caller's own cache TTL; clamped to {clamped}"
            );
            clamped
        } else {
            topology_document_cache_ttl_secs
        };
        Self {
            node_did,
            store,
            vault,
            client_identity_bytes: client_identity.to_bytes(),
            enable_registry_dht,
            messaging_broker,
            alert_topic,
            poll_interval_secs,
            max_restart_attempts,
            restart_backoff_secs,
            renewed_cert_expires_hours,
            max_renewals_per_pass,
            master_anchor_refresh_interval_secs,
            anchor_writer,
            tier1_writer,
            topology_document_not_after_secs,
            topology_document_cache_ttl_secs,
            signed_documents: DashMap::new(),
            queue_tick_secs,
            queue_connector: Arc::new(LiveQueueConnector {
                client_identity_bytes: client_identity.to_bytes(),
                enable_registry_dht,
            }),
            instance_locks: DashMap::new(),
            last_reconciled: DashMap::new(),
            previous_pass_started_at: AtomicU64::new(0),
            cancellation_token: CancellationToken::new(),
            #[cfg(test)]
            _fixture_tempdir: None,
        }
    }

    pub fn store(&self) -> &SupervisorStore {
        &self.store
    }

    /// This app instance's own async mutex, created on first use and
    /// shared by every later caller that names the same instance (M05A
    /// A5c D-A5c-7). `DashMap::entry` takes its own internal shard lock
    /// only for the duration of the lookup/insert, not for the mutex's
    /// own hold time -- what the caller does with the returned `Arc`
    /// afterward is independent of it.
    fn instance_lock(&self, app_instance_id: &str) -> Arc<AsyncMutex<()>> {
        self.instance_locks
            .entry(app_instance_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// The resident reconcile loop (M05A A5c §19.7/§19.8, D-A5c-7/D-A5c-8).
    /// Spawned by `RuntimeServices::run_until_shutdown`, not pinned in its
    /// own `select!` -- see `cancellation_token`'s doc for why a bare
    /// token is not enough on its own and `shutdown` must also be awaited
    /// via the `JoinHandle` the spawn site keeps. `MissedTickBehavior::
    /// Skip` so a pass that outruns `poll_interval_secs` against a slow
    /// substrate drops the tick it overran instead of queueing a burst.
    pub async fn run(&self) -> anyhow::Result<()> {
        let mut interval = Self::build_pass_interval(self.poll_interval_secs);
        loop {
            tokio::select! {
                () = self.cancellation_token.cancelled() => return Ok(()),
                _ = interval.tick() => self.run_pass().await,
            }
        }
    }

    /// `MissedTickBehavior::Skip` (M05A A5c §19.7, D-A5c-7): a pass that
    /// outruns `poll_interval_secs` against a slow substrate drops the
    /// tick it overran instead of firing a queued burst once it finally
    /// returns. Its own function so this one configuration decision is
    /// directly testable under a paused clock, with no pass or network
    /// involved (§23 test 34).
    fn build_pass_interval(poll_interval_secs: u64) -> tokio::time::Interval {
        let mut interval = tokio::time::interval(Duration::from_secs(poll_interval_secs.max(1)));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval
    }

    /// How many items one worker tick claims before yielding back to the
    /// interval -- generous relative to any one supervisor's realistic
    /// outage-time backlog, and bounded so one tick cannot starve every
    /// later one if the backlog is somehow larger.
    const QUEUE_WORKER_CLAIM_LIMIT: u32 = 100;

    /// The durable outbox worker (M05B B1, D-B1-1): claims items due from
    /// this supervisor's own queue and replays each `write_bindings`
    /// against its target substrate. Spawned beside the resident loop
    /// (`RuntimeServices::run_until_shutdown`), on the same
    /// `queue_tick_secs` cadence and the same cancellation token -- and,
    /// like the resident loop, **not** drained on shutdown (D-B1-8): work
    /// in flight is abandoned, and the visibility timeout returns it to
    /// `Pending` on the next start.
    pub async fn run_queue_worker(&self) -> anyhow::Result<()> {
        let mut interval = Self::build_pass_interval(self.queue_tick_secs);
        loop {
            tokio::select! {
                () = self.cancellation_token.cancelled() => return Ok(()),
                _ = interval.tick() => self.queue_worker_tick().await,
            }
        }
    }

    async fn queue_worker_tick(&self) {
        let now = outbox::now_ms();
        let items = match self.store.queue.claim_due(now, Self::QUEUE_WORKER_CLAIM_LIMIT) {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!(error = %e, "failed to claim due items from the outbox this tick");
                return;
            }
        };
        let max_attempts = self.store.queue.max_attempts();
        for item in items {
            // D-B1-8/§0.7: shutdown abandons work in flight rather than
            // draining it -- checked between every item, not only between
            // ticks, and raced into the delivery itself below, so
            // cancelling mid-tick does not wait out the rest of a claim
            // batch (up to `QUEUE_WORKER_CLAIM_LIMIT` items, each up to
            // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT`) against exactly the
            // unreachable substrates this is meant never to wait on (M05B
            // B1 review finding 5). An item not yet started this tick is
            // left claimed; its own visibility timeout returns it to
            // `Pending` on the next start, same as a crashed worker.
            if self.cancellation_token.is_cancelled() {
                return;
            }
            // M05B B1 review finding 7: an item claimed `max_attempts`
            // times without ever reaching `fail`/`complete` (a worker
            // panic or crash on every delivery) would otherwise be handed
            // out forever -- `attempts` alone cannot bound it, since only
            // `fail` advances that counter. Dead-letter it through the
            // ordinary terminal path instead of attempting delivery again.
            if item.claim_count > u32::from(max_attempts) {
                self.dead_letter_poison_pill(item).await;
                continue;
            }
            tokio::select! {
                () = self.cancellation_token.cancelled() => return,
                () = self.deliver_queued_item(item) => {}
            }
        }
    }

    /// Parses a claimed item's queue key into what every path past this
    /// point needs, dead-lettering it (terminal) and returning `None` when
    /// it does not parse -- shared by `deliver_queued_item` and
    /// [`Self::dead_letter_poison_pill`].
    fn parse_or_dead_letter(
        &self,
        item: &QueueItem,
        now: i64,
    ) -> Option<(AppInstanceId, QueueKey)> {
        let Ok(key) = item.queue_key.parse::<QueueKey>() else {
            tracing::warn!(
                queue_key = %item.queue_key,
                "outbox item carries an unparseable queue key; dead-lettering"
            );
            let _ = self.store.queue.fail(item.id, now, "unparseable queue key", true);
            return None;
        };
        let Ok(instance_id) = AppInstanceId::try_new(key.app_instance_id.clone()) else {
            let _ =
                self.store.queue.fail(item.id, now, "queue key's app instance id is invalid", true);
            return None;
        };
        Some((instance_id, key))
    }

    /// An item whose claim count alone exhausted the attempt budget,
    /// without `fail` ever being called for it (M05B B1 review finding 7)
    /// -- dead-lettered through the same terminal path and alerting
    /// `fail_queued_item` already gives every other terminal reason.
    async fn dead_letter_poison_pill(&self, item: QueueItem) {
        let now = outbox::now_ms();
        let Some((instance_id, key)) = self.parse_or_dead_letter(&item, now) else { return };
        self.fail_queued_item(
            &instance_id,
            &key,
            item.id,
            now,
            "delivery attempt budget exhausted without a recorded outcome (repeated crash or \
             panic during delivery)",
            true,
        )
        .await;
    }

    /// Replays one claimed item: reconnects to its target substrate,
    /// attempts the write again, and applies the outcome mapping D-B1-15
    /// specifies -- `applied`/`no-op`/`stale` complete and clear any
    /// `BindingConflict` the original transport failure raised; `conflict`
    /// completes **and** raises that same alert, exactly as the
    /// synchronous path does; a transport error retries; a callee error
    /// (the substrate reached and refused -- e.g. its target no longer
    /// exists, failure-matrix row 9) is terminal.
    ///
    /// Takes the same `instance_lock` the resident loop's own pass holds
    /// (D-B1-14) for the whole delivery -- without it, a queued write and a
    /// live pass write for the same instance could interleave and race
    /// this supervisor into a spurious `BindingConflict`, indistinguishable
    /// from real split-brain.
    async fn deliver_queued_item(&self, item: QueueItem) {
        let now = outbox::now_ms();
        let Some((instance_id, key)) = self.parse_or_dead_letter(&item, now) else { return };
        let Ok(queued) = serde_json::from_slice::<outbox::QueuedBindingWrite>(&item.payload) else {
            tracing::warn!(
                queue_key = %item.queue_key,
                "outbox item carries an unparseable payload; dead-lettering"
            );
            let _ = self.store.queue.fail(item.id, now, "unparseable payload", true);
            return;
        };

        let lock = self.instance_lock(&key.app_instance_id);
        let _guard = lock.lock().await;

        let Ok(Some(state)) = self.store.get(&key.app_instance_id) else {
            let _ = self.store.queue.fail(item.id, now, "app instance no longer known", true);
            return;
        };
        if state.retired {
            // The operator retired this instance between enqueue and
            // delivery. Neither attempting the write (it would resurrect a
            // binding the operator just released) nor dead-lettering it
            // with a `DeliveryExhausted` alert (noise against an instance
            // nobody is going to act on) is right -- the item's own intent
            // is simply moot now, the same as `applied`/`no-op`/`stale`
            // (M05B B1 review finding 16).
            let _ = self.store.queue.complete(item.id);
            return;
        }
        let Ok(inventory) = serde_json::from_str::<SupervisorInventory>(&state.inventory_json)
        else {
            let _ =
                self.store.queue.fail(item.id, now, "stored inventory-json does not parse", true);
            return;
        };
        let Some(entry) = inventory.values().find(|e| e.did == queued.substrate_did) else {
            let _ = self.store.queue.fail(
                item.id,
                now,
                "target substrate is no longer in this instance's inventory",
                true,
            );
            return;
        };

        let client = match self.queue_connector.connect(entry).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(
                    queue_key = %item.queue_key,
                    attempt = item.attempts,
                    error = %e,
                    "queue worker delivery attempt failed to connect"
                );
                // Re-read the clock rather than reusing `now` from function
                // entry: `connect` can burn up to
                // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s), and the early
                // backoff waits this feeds are sub-second -- a stale `now`
                // can put `next_attempt_at` in the past, governing the wait
                // by `queue_tick_secs` instead of the configured curve
                // (M05B B1 review finding 8).
                let failed_at = outbox::now_ms();
                self.fail_queued_item(
                    &instance_id,
                    &key,
                    item.id,
                    failed_at,
                    &e.to_string(),
                    false,
                )
                .await;
                return;
            }
        };
        let outcome = client.attempt_write_bindings(queued.write.clone()).await;

        match outcome {
            Ok(outcomes) => {
                let _ = self.store.queue.complete(item.id);
                let conflict =
                    outcomes.iter().any(|o| matches!(o, BindingWriteOutcome::Conflict(_)));
                if conflict {
                    if let Ok(true) = self.store.alerts.raise(
                        &instance_id,
                        Some(&key.logical_ref),
                        None,
                        &queued.substrate_did,
                        AlertKind::BindingConflict,
                        &format!(
                            "a queued binding push for '{}' landed as a conflict on replay: \
                             {outcomes:?}",
                            key.logical_ref
                        ),
                    ) {
                        self.publish_opened_alerts(
                            &key.app_instance_id,
                            &[(AlertKind::BindingConflict, key.logical_ref.clone())],
                        )
                        .await;
                    }
                } else {
                    // The original transport failure's own alert (raised
                    // when this item was first enqueued) is stale now that
                    // delivery has actually converged.
                    let _ = self.store.alerts.clear(
                        &instance_id,
                        Some(&key.logical_ref),
                        &queued.substrate_did,
                        AlertKind::BindingConflict,
                    );
                }
            }
            Err(err) => {
                let failed_at = outbox::now_ms();
                // Only the narrower "the substrate answered that this
                // write's own target is gone" case is terminal here
                // (failure-matrix row 9) -- `deploy::is_callee_error`
                // alone would also dead-letter a transient,
                // reached-and-answered error (a locked database, a service
                // still starting) that the wire protocol cannot currently
                // distinguish from "gone" by error code, only by message
                // text (M05B B1 review finding 10). Treating every callee
                // error as terminal on this path -- the only path that can
                // reach an *already-durable* item, unlike the synchronous
                // path's "decline to enqueue" -- would prematurely give up
                // on work that survived a restart specifically to be
                // retried.
                let terminal = deploy::is_target_gone_error(&err);
                self.fail_queued_item(
                    &instance_id,
                    &key,
                    item.id,
                    failed_at,
                    &err.to_string(),
                    terminal,
                )
                .await;
            }
        }
    }

    /// Records a failed delivery attempt and, when the item's own budget is
    /// now exhausted, raises or refreshes the standing `DeliveryExhausted`
    /// alert with the current dead-letter count for this key (D-B1-6) --
    /// the DLQ's whole stated purpose fails unless something surfaces it.
    async fn fail_queued_item(
        &self,
        instance_id: &AppInstanceId,
        key: &QueueKey,
        item_id: i64,
        now: i64,
        error: &str,
        terminal: bool,
    ) {
        let Ok(outcome) = self.store.queue.fail(item_id, now, error, terminal) else { return };
        let FailOutcome::DeadLettered { pruned_keys } = outcome else { return };
        let count = self
            .store
            .queue
            .dead_letters()
            .map(|rows| rows.iter().filter(|d| d.queue_key == key.to_string()).count())
            .unwrap_or(0);
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&key.logical_ref),
            None,
            &key.substrate_did,
            AlertKind::DeliveryExhausted,
            &format!(
                "{count} binding write(s) for '{}' exhausted their delivery attempt budget",
                key.logical_ref
            ),
        ) {
            self.publish_opened_alerts(
                &key.app_instance_id,
                &[(AlertKind::DeliveryExhausted, key.logical_ref.clone())],
            )
            .await;
        }

        // The DLQ cap just pruned these other keys' oldest dead letters
        // (D-B1-9) -- if that was their *last* one, their own standing
        // alert must clear too, or a prune leaves a permanent red mark
        // nothing can ever clear (failure-matrix row 4a, M05B B1 review
        // finding 3). A pruned key's group is always this same instance
        // (group_key == app_instance_id, `outbox.rs`), but parsed
        // generically here rather than assumed, since this crate does not
        // enforce that pairing.
        for pruned_key in pruned_keys {
            let Ok(pruned) = pruned_key.parse::<QueueKey>() else { continue };
            let Ok(pruned_instance) = AppInstanceId::try_new(pruned.app_instance_id.clone()) else {
                continue;
            };
            self.clear_delivery_exhausted_if_empty(&pruned_instance, &pruned);
        }
    }

    /// Clears the standing `DeliveryExhausted` alert for `key` once none of
    /// its dead letters remain -- `replay`'s own clearing path (D-B1-6).
    fn clear_delivery_exhausted_if_empty(&self, instance_id: &AppInstanceId, key: &QueueKey) {
        let remaining = self
            .store
            .queue
            .dead_letters()
            .map(|rows| rows.iter().any(|d| d.queue_key == key.to_string()))
            .unwrap_or(true);
        if !remaining {
            let _ = self.store.alerts.clear(
                instance_id,
                Some(&key.logical_ref),
                &key.substrate_did,
                AlertKind::DeliveryExhausted,
            );
        }
    }

    /// Cancels the loop's token -- the spawn site (`RuntimeServices`) is
    /// the one that awaits the `JoinHandle` this unblocks, since that is
    /// the only place that holds it (M05A A5c D-A5c-8).
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.cancellation_token.cancel();
        Ok(())
    }

    /// One tick of the resident loop: every non-retired, non-paused
    /// instance, in `all_active`'s order, sequentially -- a slow instance
    /// delays later ones in this same pass, accepted for A5c since the
    /// per-instance lock (not a global one) is what keeps that a latency
    /// property rather than a correctness one (§19.14's `all_active`
    /// entry).
    async fn run_pass(&self) {
        let started =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let instances = match self.store.all_active() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read the supervisor's work list this pass");
                return;
            }
        };
        for state in instances {
            let lock = self.instance_lock(&state.app_instance_id);
            let _guard = lock.lock().await;
            self.reconcile_instance_pass(&state.app_instance_id).await;
        }
        // After the sweep, not before: every instance reconciled above must
        // read the *previous* sweep's start, which is the window
        // `schedule_grace_secs` sizes itself from.
        self.previous_pass_started_at.store(started, Ordering::Relaxed);
    }

    /// How far back a schedule may look for an occurrence it has not run
    /// yet (ADR-0023 §6). The floor is two poll intervals, which tolerates
    /// one dropped sweep; above that it is the real sweep-to-sweep gap,
    /// because a sweep that outruns its own interval is the ordinary case,
    /// not the exception -- every pass rebuilds an iroh client per
    /// substrate, so one unreachable substrate alone can push a sweep past
    /// two nominal intervals. Sizing the window from the *configured*
    /// interval instead would cut a hole between the last evaluation and
    /// the start of the window, and every occurrence landing in that hole
    /// would be dropped while the supervisor was awake the whole time --
    /// visible to an operator only as a watermark that keeps advancing
    /// while `last_run_at` never moves.
    ///
    /// A clamp is still needed above the honest watermark, since nothing
    /// runs while the process is down: without it, a supervisor started
    /// after a day off would fire one tick per schedule immediately. The
    /// gap this reads is measured inside one process and reset to zero by a
    /// restart, so downtime never widens the window.
    fn schedule_grace_secs(&self, now: u64) -> u64 {
        let floor = 2 * self.poll_interval_secs;
        match self.previous_pass_started_at.load(Ordering::Relaxed) {
            0 => floor,
            previous => floor.max(now.saturating_sub(previous)),
        }
    }

    /// One instance's share of a loop pass (M05A A5c §19.2/§19.9/§19.11,
    /// D-A5c-2/D-A5c-9/D-A5c-11): a health sweep (shared by the alert
    /// pass and, unless superseded, the reconcile below), then -- for a
    /// non-superseded instance -- a **filtered** redeploy of only the
    /// services `Reconciler::compute_diff` says changed since the last
    /// fully-landed plan, plus any service the current sweep finds with
    /// no completed placement at all (D-A5c-10's `missing_placement`,
    /// which a content-unchanged diff cannot see on its own). One client
    /// set for the whole pass (D-A5c-9), closed once at the end.
    async fn reconcile_instance_pass(&self, app_instance_id: &str) {
        // Review finding A-6: each of these four reads used to fail
        // silently -- no log, no alert -- which drops the instance out
        // of every future pass with nothing anywhere to say why. None of
        // the four can raise a *stored* alert (the failure is in reading
        // the store, or in parsing what it just returned, so there is no
        // instance state left to attach one to that is any more trustworthy
        // than the log line itself), but a `tracing::warn!` at least makes
        // the drop observable instead of indistinguishable from an
        // instance that was never submitted.
        let Ok(Some(state)) = self.store.get(app_instance_id) else {
            tracing::warn!(
                app_instance_id,
                "failed to read this instance's desired state; skipping it this pass"
            );
            return;
        };
        if state.paused || state.retired {
            return;
        }
        let Ok(plan) = DeploymentPlan::from_json(&state.plan_json) else {
            tracing::warn!(
                app_instance_id,
                "stored plan-json does not parse as a DeploymentPlan; skipping this instance \
                 until it is resubmitted"
            );
            return;
        };
        let Ok(inventory) = serde_json::from_str::<SupervisorInventory>(&state.inventory_json)
        else {
            tracing::warn!(
                app_instance_id,
                "stored inventory-json does not parse; skipping this instance until it is \
                 resubmitted"
            );
            return;
        };
        let Ok(instance_id) = AppInstanceId::try_new(app_instance_id.to_string()) else {
            tracing::warn!(
                app_instance_id,
                "the stored app_instance_id itself is not a valid AppInstanceId; skipping this \
                 instance"
            );
            return;
        };

        // Review finding A-7: nothing else ever moves a crashed-mid-apply
        // record out of `Applying` -- `apply_with_clients` only ever
        // updates one to `Active`/`Degraded` itself, from inside the same
        // call that appended it. The per-instance lock this pass holds
        // (D-A5c-7) proves that call is gone: a second apply for this
        // instance cannot be in flight while we hold the lock, so a
        // record still reading `Applying` here was abandoned by a process
        // that exited between appending it and updating it. `Degraded` is
        // the correct resting state for "we do not know whether this
        // landed" (D-A3-18) -- `handle_status` would otherwise report
        // `Applying` forever, past the point this pass's own diff (which
        // reads completed action rows, not this record's state) has
        // already re-derived and retried whatever was actually missing.
        if let Ok(Some(latest)) = self.store.journal.get_latest(&instance_id)
            && latest.state == DeploymentState::Applying
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to recover a deployment record stuck in Applying"
            );
        }

        let landed =
            self.store.journal.get_completed_actions_for_instance(&instance_id).unwrap_or_default();

        let mut expected = Vec::new();
        let mut missing_placement: BTreeSet<String> = BTreeSet::new();
        let mut did_to_alias: BTreeMap<String, String> = BTreeMap::new();
        for svc in &plan.services {
            match deploy::current_placement(&landed, &svc.member_ref().to_string()) {
                None => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                        member_index: svc.member_index,
                    });
                    missing_placement.insert(svc.member_ref().to_string());
                }
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
                        member_index: svc.member_index,
                    });
                    if let Some(alias) = &row.substrate_alias {
                        did_to_alias.insert(row.substrate_did.clone(), alias.clone());
                    }
                }
            }
        }

        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(&plan).unwrap_or_default().into_iter().collect();
        let connect_aliases = Self::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, &inventory).await;
        // Review finding A-6: these used to be discarded entirely. An
        // unreachable substrate is already visible another way (the
        // health sweep reports it as a fault for a service placed
        // there), but an alias with no inventory entry or no credential
        // is a configuration problem the health sweep cannot see at
        // all, since it never gets far enough to try connecting.
        for (alias, reason) in &failed {
            tracing::warn!(
                app_instance_id,
                alias,
                reason,
                "failed to connect to a substrate this pass needs"
            );
        }

        let mut targets: BTreeMap<String, HealthTarget> = BTreeMap::new();
        for (did, alias) in &did_to_alias {
            if !inventory.contains_key(alias) {
                continue;
            }
            let query: Arc<dyn StatusQuery> = match clients.get(&SubstrateAlias::new(alias.clone()))
            {
                Some(c) => c.clone() as Arc<dyn StatusQuery>,
                None => Arc::new(UnreachableQuery(format!(
                    "failed to connect to substrate alias '{alias}'"
                ))),
            };
            targets.insert(
                did.clone(),
                HealthTarget {
                    alias: Some(SubstrateAlias::new(alias.clone())),
                    substrate_did: did.clone(),
                    query,
                },
            );
        }

        let report = health::poll_once(&targets, &expected).await;
        drop(targets);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        // D-A5c-10, the same sentinel-keyed alert `handle_status` raises.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        // D-A5d-9: `SUPERVISOR_CERT_ALERT_POLICY`'s own doc explains why.
        let mut opened = match health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
            now,
            &extra_live_pairs,
            SUPERVISOR_CERT_ALERT_POLICY,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(app_instance_id, error = %e, "failed to record this pass's health report");
                Vec::new()
            }
        };
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if missing_placement.contains(&l_ref) {
                if let Ok(true) = self.store.alerts.raise(
                    &instance_id,
                    Some(&l_ref),
                    None,
                    NEVER_LANDED_SUBSTRATE_DID,
                    AlertKind::InstanceNotRunning,
                    "planned but never deployed; the supervisor holds no completed placement for \
                     this service",
                ) {
                    opened.push((AlertKind::InstanceNotRunning, l_ref));
                }
            } else {
                let _ = self.store.alerts.clear(
                    &instance_id,
                    Some(&l_ref),
                    NEVER_LANDED_SUBSTRATE_DID,
                    AlertKind::InstanceNotRunning,
                );
            }
        }
        let diff = Reconciler::new(&self.store.journal).compute_diff(&plan);
        // M05A A5e (D-A5e-7): a dependent member whose diff against the
        // last active plan changed *only* `resolved_dependencies` is a
        // membership change in one of its dependencies -- pushed via
        // `push_bindings`, not redeployed. Every other kind of change
        // (config, placement, ...) still takes the redeploy path.
        // A member whose diff changed *only* its
        // `schedule` is excluded the same way, but pushes nothing.
        // `classify_update_actions` is the same classifier an
        // operator-triggered apply uses (`apply_with_membership_pushes`),
        // so a loop pass and a `submit`/`force-reconcile` make the
        // identical redeploy-vs-push-vs-exclude call for the identical
        // diff.
        let (redeploy_exclusions, push_candidates) = diff
            .as_ref()
            .map(|d| Self::classify_update_actions(&landed, &d.actions))
            .unwrap_or_default();
        let needs_work = Self::redeploy_work_list(
            &missing_placement,
            diff.as_ref().map(|d| d.actions.as_slice()).unwrap_or_default(),
            &redeploy_exclusions,
        );
        // `Remove` is the one action the work list above ignores: a
        // plan-level removal is never undeployed here, only alerted on.
        for action in diff.iter().flat_map(|d| &d.actions) {
            let ReconcileAction::Remove(l_ref) = action else { continue };
            let l_ref_str = l_ref.to_string();
            if let Some(row) = deploy::current_placement(&landed, &l_ref_str)
                && let Ok(true) = self.store.alerts.raise(
                    &instance_id,
                    Some(&l_ref_str),
                    row.substrate_alias.as_deref(),
                    &row.substrate_did,
                    AlertKind::OrphanedService,
                    "dropped from the plan but still running on its substrate; not undeployed -- \
                     remove it by hand (`svc remove`) if that is intended",
                )
            {
                opened.push((AlertKind::OrphanedService, l_ref_str));
            }
        }
        // A member back in the current plan cannot be orphaned this
        // pass, regardless of what an older diff once said.
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if let Some(row) = deploy::current_placement(&landed, &l_ref) {
                let _ = self.store.alerts.clear(
                    &instance_id,
                    Some(&l_ref),
                    &row.substrate_did,
                    AlertKind::OrphanedService,
                );
            }
        }

        // M05A A5c phase 6 (§14 step 3, matrix row 13): landed services
        // the sweep just found `InstanceNotRunning` are restart
        // candidates -- distinct from `needs_work` above, which never-
        // landed or content-changed services feed into instead. A
        // healthy service's own remediation bookkeeping resets here too,
        // so the next fault starts counting from zero.
        let restart_candidates = Self::restart_candidates(&report);
        for svc in report.services.iter().filter(|s| s.signal == Signal::Healthy) {
            let _ = self.store.clear_remediation(app_instance_id, &svc.member_ref().to_string());
        }

        // M05A A5d: the fourth work-list. Its input is this pass's own
        // health poll -- `ServiceHealth` already carries the certificate's
        // issued/expires pair -- so renewal needs no poll and no cadence of
        // its own. Deduped against `needs_work` (a service about to go
        // through `apply_plan` gets a fresh certificate there, so renewing
        // it here would certify it twice in one pass) but deliberately
        // *not* against `restart_candidates`: a restart reloads the running
        // instance and touches no certificate, so a service under
        // remediation still needs its own renewal check.
        let revoked = self.store.revoked_placements(app_instance_id).unwrap_or_default();
        let renewal_candidates = Self::renewal_candidates(
            &report,
            &needs_work,
            &revoked,
            now,
            self.max_renewals_per_pass,
        );
        // The fifth work-list (ADR-0023 §6): every schedule
        // this instance's plan declares, evaluated against this pass's own
        // health report, over the grace window `schedule_grace_secs`
        // sizes from this supervisor's own sweep cadence.
        let declared_schedules: BTreeSet<String> =
            Self::declared_schedules(&plan).into_keys().collect();
        if let Err(e) = self.store.prune_schedule_states(app_instance_id, &declared_schedules) {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to drop the state of a schedule the plan no longer declares"
            );
        }
        let schedule_states = self.store.schedule_states(app_instance_id).unwrap_or_default();
        let schedule_decisions = Self::schedule_decisions(
            &plan,
            &schedule_states,
            &report,
            now,
            self.schedule_grace_secs(now),
        );
        // Members whose certificate renewed but whose
        // `restart-on-rotation` restart then failed. Independent of the
        // renewal work-list above -- these are no longer near-expiry, so
        // `renewal_candidates` will never see them again.
        let pending_rotation_restarts =
            self.store.pending_rotation_restarts(app_instance_id).unwrap_or_default();
        // D-A5d-9's clearing rule, the same recomputed-not-flagged shape
        // `Superseded` and `remediation.terminal` already use: a member the
        // substrate now reports with a healthy certificate window has no
        // stalled renewal, whatever an earlier pass raised.
        self.clear_settled_renewal_alerts(&instance_id, &report, now);
        self.publish_opened_alerts(app_instance_id, &opened).await;

        let held_max = Self::max_held_generation_from_clients(
            app_instance_id,
            &plan_aliases,
            &Self::actors_from_clients(&clients),
        )
        .await;
        let superseded = self
            .update_superseded_alert(&instance_id, app_instance_id, held_max, state.generation)
            .unwrap_or(false);

        // D-A5c-11: a superseded instance is skipped for every write this
        // pass (no deploy, no push, no restart) but was still polled for
        // health above.
        if superseded {
            self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
            Self::shutdown_clients(clients.into_values()).await;
            return;
        }

        // The anchor refresh is evaluated every pass against a persisted
        // fact rather than on a timer of its own, so it -- unlike the three
        // work-lists -- always has something to check.
        if !needs_work.is_empty()
            || !restart_candidates.is_empty()
            || !renewal_candidates.is_empty()
            || !pending_rotation_restarts.is_empty()
            || !push_candidates.is_empty()
            || !schedule_decisions.is_empty()
            || self.anchor_writer.is_some()
            || self.tier1_writer.is_some()
        {
            self.apply_write_phase(WritePhase {
                instance_id: &instance_id,
                app_instance_id,
                plan: &plan,
                needs_work: &needs_work,
                restart_candidates: &restart_candidates,
                renewal_candidates: &renewal_candidates,
                pending_rotation_restarts: &pending_rotation_restarts,
                push_candidates: &push_candidates,
                schedule_decisions: &schedule_decisions,
                did_to_alias: &did_to_alias,
                clients: &clients,
                now,
            })
            .await;
        }
        self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
        Self::shutdown_clients(clients.into_values()).await;
    }

    /// The write half of a loop pass (M05A A5c D-A5c-14/F6): mints,
    /// certifies, and applies only `needs_work`'s services, then attempts
    /// one bounded restart per `restart_candidates` entry. Extracted from
    /// `reconcile_instance_pass` so the re-read this opens with is
    /// directly testable against a `pause`/`retire` that lands between
    /// the health sweep and here -- neither takes the per-instance lock
    /// a pass otherwise holds for its whole duration, so this is the one
    /// window that flag can still land in, and this fresh read is what
    /// closes it (D-A5c-14 says a pause "takes effect at the next write
    /// phase", not mid-write; this is that write phase's own boundary).
    /// Also picks up a generation `adopt` may have bumped since the
    /// pass's own early read.
    async fn apply_write_phase(&self, phase: WritePhase<'_>) {
        let WritePhase {
            instance_id,
            app_instance_id,
            plan,
            needs_work,
            restart_candidates,
            renewal_candidates,
            pending_rotation_restarts,
            push_candidates,
            schedule_decisions,
            did_to_alias,
            clients,
            now,
        } = phase;
        let Ok(Some(fresh_state)) = self.store.get(app_instance_id) else { return };
        if fresh_state.paused || fresh_state.retired {
            return;
        }

        // Set only when `apply_with_clients` below is actually called this
        // pass -- the signal the finding-A downgrade further down needs to
        // tell "this pass's own record_plan might already carry a push
        // candidate's converged state" from "the last Active record is
        // stale and unrelated to this pass's push", which it must not
        // downgrade.
        let mut redeployed_this_pass = false;
        if !needs_work.is_empty() {
            let mut filtered_plan = plan.clone();
            // `resolve_targets` (deploy.rs) fails the *whole* `apply_plan`
            // call closed if even one service in the plan it is given has
            // no built target -- correct for `roymctl app deploy`'s own
            // all-or-nothing call, wrong here: a plan spanning two
            // substrates where only one is reachable this pass must not
            // block the service that *could* land (matrix row 12). Only
            // services whose alias this pass actually connected to are
            // included; an unreachable one stays in `needs_work` (nothing
            // landed for it) and is picked up again next pass.
            filtered_plan.services.retain(|s| {
                needs_work.contains(&s.member_ref().to_string())
                    && s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
            });
            if !filtered_plan.services.is_empty() {
                // Review finding A-1: what gets *applied* this pass is
                // deliberately narrowed to `filtered_plan`, but what gets
                // *journaled* as the new baseline must not be -- diffing
                // future passes against a snapshot that only ever holds
                // this pass's touched subset drops every untouched,
                // already-landed service out of the baseline, so the next
                // pass reads it as missing and redeploys it, which then
                // drops today's subset out in turn. The loop alternates
                // forever instead of converging. `record_plan` carries
                // every service this supervisor still believes landed
                // (everything outside `needs_work`) plus whatever this
                // pass is about to (re)land, and excludes only a
                // `needs_work` service still unreachable this pass, which
                // genuinely has not landed.
                let record_plan = Self::record_plan_for_pass(plan, needs_work, clients);
                match keys::mint_and_substitute(&mut filtered_plan, &self.vault).await {
                    Ok((minted, masters)) => {
                        // Set from the call's own result, not from having
                        // reached this arm -- mirrors `apply_result_is_ok`
                        // in `apply_with_membership_pushes` and for the
                        // same reason: `apply_with_clients` returning `Err`
                        // can mean nothing was journaled this pass at all
                        // (a certify failure before the journal write), in
                        // which case `redeployed_this_pass` must stay
                        // false, or `Degraded` was already journaled
                        // instead of `Active`, in which case the finding-A
                        // downgrade below is a harmless no-op either way.
                        redeployed_this_pass = self
                            .apply_with_clients(
                                &filtered_plan,
                                &record_plan,
                                &masters,
                                clients,
                                fresh_state.generation,
                                minted,
                            )
                            .await
                            .inspect_err(|e| {
                                tracing::warn!(
                                    app_instance_id,
                                    error = %e,
                                    "this pass's redeploy did not fully land"
                                );
                            })
                            .is_ok();
                    }
                    Err(e) => tracing::warn!(
                        app_instance_id,
                        error = %e,
                        "failed to mint members for this pass"
                    ),
                }
            }
        }

        let mut opened = Vec::new();

        // M05A A5e (D-A5e-7): every member whose only change is which DIDs
        // a dependency resolves to gets a binding push instead of the
        // redeploy above -- an unreachable member this pass simply retries
        // next pass, since `resolved_dependencies` still disagrees with
        // what was last pushed.
        let mut any_push_failed = false;
        for (svc, substrate_did) in push_candidates {
            // M05A A5e review (matrix row 11): a dependent this pass could
            // not even connect to used to be dropped here with no alert
            // and no `opened` entry, so `BindingConflict` was never set and
            // `Degraded` never derived from it -- indistinguishable from
            // "nothing to push". Raised through the same alert
            // `write_bindings_at_epoch` itself failing would raise, so the
            // operator sees the same row either way.
            //
            // M05B B1 review: raising the alert and moving on used to be
            // the whole story here, which left a substrate this pass could
            // not even reach with nothing durable behind it -- the DLQ's
            // try-then-queue only fires *inside* an attempted call
            // (`DurableActor::write_bindings`), and neither branch below
            // gets far enough to make one. `enqueue_unreachable_push`
            // queues the write directly so a substrate that is durably
            // offline, not merely flaky mid-call, still converges once it
            // returns.
            let Some(alias) = did_to_alias.get(substrate_did) else {
                self.enqueue_unreachable_push(
                    instance_id,
                    app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    fresh_state.generation,
                    "this pass has no known substrate alias for the member's landed DID",
                    &mut opened,
                )
                .await;
                any_push_failed = true;
                continue;
            };
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else {
                self.enqueue_unreachable_push(
                    instance_id,
                    app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    fresh_state.generation,
                    &format!("failed to connect to substrate alias '{alias}' this pass"),
                    &mut opened,
                )
                .await;
                any_push_failed = true;
                continue;
            };
            let actor = self.durable_actor(
                client.clone(),
                app_instance_id,
                &svc.member_ref().to_string(),
                substrate_did,
            );
            // `Deferred` means the push did not land this pass -- it must
            // count the same as an error here (M05B B1 review follow-on
            // 1), or a redeploy landing in the same pass would journal
            // this member's new baseline as converged while the queue
            // still holds stale content for it. Distinct from `Landed`
            // with zero outcomes (every dependency was just removed from
            // this member's manifest), which is a real, converged
            // success, not deferred (M05B B1 review follow-up: an earlier
            // version of this match used an empty `Vec` as the deferred
            // sentinel, which that case collided with).
            match self
                .push_bindings(
                    instance_id,
                    plan,
                    svc,
                    substrate_did,
                    &actor,
                    fresh_state.generation,
                    &mut opened,
                )
                .await
            {
                Ok(PushOutcome::Deferred) => any_push_failed = true,
                Ok(PushOutcome::Landed(_)) => {}
                Err(_) => any_push_failed = true,
            }
        }
        // Review round 2, finding A (same shape, narrower window here):
        // `record_plan_for_pass` above keeps every push candidate's *new*
        // `resolved_dependencies` in `record_plan` unconditionally (it is
        // not a `needs_work` member, so nothing filters it out) -- so a
        // needs_work redeploy this same pass journals that push candidate
        // as already converged, before this loop ever runs. If the push
        // then fails, the next pass's diff would read it as landed and
        // never retry. Gated on `redeployed_this_pass`: the ordinary case
        // (a push with no needs_work redeploy alongside it in the same
        // pass) journals nothing here at all, so the latest record is
        // whatever an earlier pass left -- unrelated to this push, and
        // must not be downgraded just because this pass's push failed.
        if any_push_failed
            && redeployed_this_pass
            && let Ok(Some(latest)) = self.store.journal.get_latest(instance_id)
            && latest.state == DeploymentState::Active
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to mark this pass's record Degraded after a binding push did not land"
            );
        }

        for (logical_ref, service_id, substrate_did) in restart_candidates {
            let Some(alias) = did_to_alias.get(substrate_did) else { continue };
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else { continue };
            let actor =
                self.durable_actor(client.clone(), app_instance_id, logical_ref, substrate_did);
            self.attempt_restart(
                instance_id,
                app_instance_id,
                logical_ref,
                service_id,
                substrate_did,
                &actor,
                fresh_state.generation,
                now,
                &mut opened,
            )
            .await;
        }

        self.renew_due_members(
            instance_id,
            app_instance_id,
            plan,
            renewal_candidates,
            did_to_alias,
            &Self::actors_from_clients(clients),
            fresh_state.generation,
            now,
            &mut opened,
        )
        .await;
        if !pending_rotation_restarts.is_empty() {
            self.retry_pending_rotation_restarts(
                instance_id,
                app_instance_id,
                plan,
                pending_rotation_restarts,
                did_to_alias,
                &Self::actors_from_clients(clients),
                fresh_state.generation,
                &mut opened,
            )
            .await;
        }
        self.refresh_due_master_anchors(plan, now).await;
        self.refresh_due_app_tier1_record(instance_id, &fresh_state, now, &mut opened).await;
        self.run_due_schedules(
            instance_id,
            app_instance_id,
            schedule_decisions,
            did_to_alias,
            &Self::actors_from_clients(clients),
            fresh_state.generation,
            now,
            &mut opened,
        )
        .await;
        self.publish_opened_alerts(app_instance_id, &opened).await;
    }

    /// The placed members whose installed certificate is inside its
    /// near-expiry window this pass, minus the two exclusions D-A5d-12
    /// names and capped at `max_renewals_per_pass`.
    ///
    /// A pure function of the pass's own health report, so the whole
    /// selection rule is testable with no vault, no client, and no store.
    /// The near-expiry decision itself is `is_near_expiry_parts` -- the
    /// same 25%-of-lifetime definition the substrate's own sweep uses, so
    /// the two cannot disagree about what "due" means.
    fn renewal_candidates(
        report: &health::HealthReport,
        needs_work: &BTreeSet<String>,
        revoked: &BTreeSet<String>,
        now: u64,
        cap: u32,
    ) -> Vec<RenewalCandidate> {
        let mut candidates: Vec<RenewalCandidate> = report
            .services
            .iter()
            .filter(|svc| {
                let l_ref = svc.member_ref().to_string();
                !needs_work.contains(&l_ref) && !revoked.contains(&l_ref)
            })
            .filter_map(|svc| {
                let issued = svc.instance_certificate_issued_at?;
                let expires = svc.instance_certificate_expires_at?;
                is_near_expiry_parts(issued, expires, now).then(|| RenewalCandidate {
                    member_ref: svc.member_ref().to_string(),
                    service_name: svc.logical_ref.service_name.to_string(),
                    service_id: svc.service_id.clone(),
                    substrate_did: svc.substrate_did.clone(),
                    expires_at: expires,
                    member_index: svc.member_index,
                })
            })
            .collect();
        // Report order (a `BTreeMap` over substrate DID,
        // then plan order) has no relation to urgency, so the cap used to
        // keep whichever members happened to sort first -- a member whose
        // renewal keeps failing stays near-expiry and occupies the same
        // slot every pass, starving everything past the cap. Sorted by
        // `expires_at` ascending first, the cap always keeps the most
        // urgent members.
        candidates.sort_by_key(|c| c.expires_at);
        candidates.truncate(cap as usize);
        candidates
    }

    /// Mint, install, and (if the plan says so) rotate, once per due
    /// member.
    ///
    /// The vault check comes first and covers the whole work-list:
    /// `kek_is_loaded` is a cheap, no-I/O read, and a locked vault means
    /// *every* mint below would fail identically. Skipping the list rather
    /// than the pass is deliberate -- health, remediation, and the anchor
    /// refresh all continue, since none of them opens the vault.
    #[allow(clippy::too_many_arguments)]
    async fn renew_due_members(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        candidates: &[RenewalCandidate],
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if candidates.is_empty() {
            return;
        }
        if !self.vault.kek_is_loaded() {
            for candidate in candidates {
                self.raise_vault_locked(instance_id, candidate, opened);
            }
            return;
        }

        for candidate in candidates {
            let Some(alias) = did_to_alias.get(&candidate.substrate_did) else { continue };
            let Some(actor) = actors.get(&SubstrateAlias::new(alias.clone())) else { continue };
            if let Err(failure) = self.renew_one_member(plan, candidate, actor, generation).await {
                match failure {
                    // D-A5d-17: one root cause, one alert kind. A vault
                    // locked between `kek_is_loaded` above and the mint
                    // below is the same condition, found later, and must
                    // not surface under a different name for it.
                    RenewalFailure::VaultLocked => {
                        self.raise_vault_locked(instance_id, candidate, opened);
                    }
                    RenewalFailure::Step { step, error } => {
                        self.raise_renewal_stalled(
                            instance_id,
                            candidate,
                            &format!(
                                "renewal {step} for '{}' failed: {error}",
                                candidate.member_ref
                            ),
                            now,
                            opened,
                        );
                    }
                    RenewalFailure::RotationRestart { error } => {
                        if let Err(e) = self.store.mark_rotation_restart_owed(
                            app_instance_id,
                            &candidate.member_ref,
                            now as i64,
                        ) {
                            tracing::warn!(
                                app_instance_id,
                                logical_ref = %candidate.member_ref,
                                error = %e,
                                "failed to persist an owed rotation restart"
                            );
                        }
                        self.raise_rotation_restart_pending(
                            instance_id,
                            candidate,
                            &format!(
                                "'{}' renewed its certificate but its restart-on-rotation restart \
                                 failed: {error}; retrying next pass",
                                candidate.member_ref
                            ),
                            opened,
                        );
                    }
                }
                tracing::warn!(
                    app_instance_id,
                    logical_ref = %candidate.member_ref,
                    "certificate renewal did not complete this pass; retrying next pass"
                );
            }
        }
    }

    /// One member's mint -> install -> rotate, in that order, stopping at
    /// the first failure. A restart is deliberately not attempted after a
    /// failed install: rotating a service whose new certificate never
    /// landed serves nothing and spends a lifecycle action for no gain.
    async fn renew_one_member(
        &self,
        plan: &DeploymentPlan,
        candidate: &RenewalCandidate,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
    ) -> Result<(), RenewalFailure> {
        let master = keys::master_for_member(
            &self.vault,
            &plan.app_instance_id.to_string(),
            &candidate.service_name,
            candidate.member_index,
        )
        .await
        .map_err(|e| match e {
            keys::VaultError::Locked => RenewalFailure::VaultLocked,
            other => RenewalFailure::Step { step: "master lookup", error: other.to_string() },
        })?;

        let cert = deploy::certify_instance_via_actor(
            actor,
            &master,
            &candidate.service_id,
            self.renewed_cert_expires_hours,
        )
        .await
        .map_err(|e| RenewalFailure::Step { step: "mint", error: e.to_string() })?;
        let cert_json = cert
            .to_json()
            .map_err(|e| RenewalFailure::Step { step: "mint", error: e.to_string() })?;

        actor
            .renew_cert(candidate.service_id.clone(), generation, cert_json)
            .await
            .map_err(|error| RenewalFailure::Step { step: "install", error })?;

        // The one place `RotationPolicy` is read. The substrate never sees
        // it: the supervisor holds the stored plan, so this is a local
        // decision made once the new certificate is known to be installed.
        let rotation = plan
            .services
            .iter()
            .find(|svc| svc.member_ref().to_string() == candidate.member_ref)
            .map(|svc| svc.config.rotation_policy);
        if rotation == Some(RotationPolicy::RestartOnRotation) {
            actor
                .restart(candidate.service_id.clone(), generation)
                .await
                .map_err(|error| RenewalFailure::RotationRestart { error })?;
        }
        Ok(())
    }

    fn raise_vault_locked(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            AlertKind::VaultLocked,
            &format!(
                "'{}' needs its instance certificate renewed, but this supervisor's vault is \
                 locked so its member master cannot be read; run: roymctl --substrate <this node> \
                 security inject-kek --kek-hex <...>",
                candidate.member_ref
            ),
        ) {
            opened.push((AlertKind::VaultLocked, candidate.member_ref.clone()));
        }
    }

    /// The certificate half of the renewal already landed,
    /// so this is deliberately not `raise_renewal_stalled` -- that pair
    /// clears the moment the health poll sees a fresh window, which this
    /// renewal already produced. Cleared only by
    /// `retry_pending_rotation_restarts` actually succeeding.
    fn raise_rotation_restart_pending(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        detail: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            AlertKind::RotationRestartPending,
            detail,
        ) {
            opened.push((AlertKind::RotationRestartPending, candidate.member_ref.clone()));
        }
    }

    /// One retry per pass, per member still owing a `restart-on-rotation`
    /// restart from an earlier renewal. Resolved against
    /// this pass's own plan and clients, the same shape `renew_due_members`
    /// uses -- an unreachable substrate simply leaves the marker in place
    /// for the next pass to retry.
    #[allow(clippy::too_many_arguments)]
    async fn retry_pending_rotation_restarts(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        pending: &BTreeSet<String>,
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        for l_ref in pending {
            let Some(svc) = plan.services.iter().find(|s| &s.member_ref().to_string() == l_ref)
            else {
                // A resubmit dropped this member from the plan (D-A5c-3:
                // not undeployed, just no longer named) -- this loop is
                // keyed off `plan.services`, so no future pass will ever
                // reach this logical ref here again. Unlike a member that
                // is merely unreachable this pass, there is no "retry
                // later" for a row nothing will ever revisit -- clearing
                // it, and whatever `RotationRestartPending` row it opened,
                // is the only way either one is not permanent.
                if let Err(e) = self.store.clear_rotation_restart_owed(app_instance_id, l_ref) {
                    tracing::warn!(
                        app_instance_id,
                        logical_ref = l_ref,
                        error = %e,
                        "failed to clear an owed rotation restart for a member dropped from the \
                         plan"
                    );
                }
                if let Ok(active) = self.store.alerts.active(instance_id) {
                    for row in active
                        .iter()
                        .filter(|r| r.kind == AlertKind::RotationRestartPending)
                        .filter(|r| r.logical_ref.as_deref() == Some(l_ref.as_str()))
                    {
                        let _ = self.store.alerts.clear(
                            instance_id,
                            Some(l_ref),
                            &row.substrate_did,
                            AlertKind::RotationRestartPending,
                        );
                    }
                }
                continue;
            };
            let Some(alias) = svc.substrate.as_ref() else { continue };
            // The plan only carries the alias; the alert row wants the
            // real DID (an alias in that column is a different bug this
            // must not repeat -- see `InstanceRevoked`'s own raise below),
            // so this reverses the same `did_to_alias` map every other
            // renewal path reads forwards.
            let Some(substrate_did) =
                did_to_alias.iter().find(|(_, a)| a.as_str() == alias.as_str()).map(|(did, _)| did)
            else {
                continue;
            };
            let Some(actor) = actors.get(alias) else { continue };
            match actor.restart(svc.service_id.to_string(), generation).await {
                Ok(()) => {
                    if let Err(e) = self.store.clear_rotation_restart_owed(app_instance_id, l_ref) {
                        tracing::warn!(
                            app_instance_id,
                            logical_ref = l_ref,
                            error = %e,
                            "failed to clear an owed rotation restart after it succeeded"
                        );
                    }
                    let _ = self.store.alerts.clear(
                        instance_id,
                        Some(l_ref),
                        substrate_did,
                        AlertKind::RotationRestartPending,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        app_instance_id,
                        logical_ref = l_ref,
                        error,
                        "rotation restart still owed; retrying next pass"
                    );
                    if let Ok(true) = self.store.alerts.raise(
                        instance_id,
                        Some(l_ref),
                        None,
                        substrate_did,
                        AlertKind::RotationRestartPending,
                        &format!(
                            "'{l_ref}' still owes a restart-on-rotation restart: {error}; \
                             retrying next pass"
                        ),
                    ) {
                        opened.push((AlertKind::RotationRestartPending, l_ref.clone()));
                    }
                }
            }
        }
    }

    /// A renewal that did not complete. `CertificateExpired` once the
    /// window has actually closed -- a current outage, not a reminder --
    /// and `CertificateNearExpiry` while there is still time (A4-04's own
    /// distinction, applied to the renewal path).
    fn raise_renewal_stalled(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        detail: &str,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let kind = if is_expired_parts(candidate.expires_at, now) {
            AlertKind::CertificateExpired
        } else {
            AlertKind::CertificateNearExpiry
        };
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            kind,
            detail,
        ) {
            opened.push((kind, candidate.member_ref.clone()));
        }
    }

    /// Clears every renewal-related alert for a member the substrate now
    /// reports with a certificate comfortably inside its window. Recomputed
    /// from the substrate's own answer each pass rather than tracked as a
    /// flag, so a renewal that succeeded out of band clears these just as a
    /// supervisor-driven one does.
    fn clear_settled_renewal_alerts(
        &self,
        instance_id: &AppInstanceId,
        report: &health::HealthReport,
        now: u64,
    ) {
        for svc in &report.services {
            let (Some(issued), Some(expires)) =
                (svc.instance_certificate_issued_at, svc.instance_certificate_expires_at)
            else {
                continue;
            };
            if is_near_expiry_parts(issued, expires, now) {
                continue;
            }
            let l_ref = svc.member_ref().to_string();
            for kind in [
                AlertKind::CertificateNearExpiry,
                AlertKind::CertificateExpired,
                AlertKind::VaultLocked,
            ] {
                let _ =
                    self.store.alerts.clear(instance_id, Some(&l_ref), &svc.substrate_did, kind);
            }
        }
    }

    /// Republishes each master this instance's plan names, but only once
    /// its `master_anchor_refresh_interval_secs` has elapsed since the last
    /// successful publication. Evaluated on the ordinary pass tick against
    /// a persisted fact rather than on a timer of its own -- the same shape
    /// the loop's other periodic decisions already use.
    ///
    /// Failures are logged, never alerted: an anchor that is still inside
    /// its 24-hour validity window is not yet a fault, and the interval
    /// leaves several passes of margin before it becomes one.
    async fn refresh_due_master_anchors(&self, plan: &DeploymentPlan, now: u64) {
        let Some(writer) = &self.anchor_writer else { return };
        // Logged rather than alerted. A locked vault is already alerted on,
        // per member, the moment a renewal is due -- and that fires on a
        // four-hour clock against this refresh's twelve-hour one, so a
        // supervisor whose vault stays shut long enough for an anchor to
        // matter has already raised `VaultLocked` several times over.
        // Raising a second kind here would be the same fact twice.
        if !self.vault.kek_is_loaded() {
            tracing::warn!(
                app_instance_id = %plan.app_instance_id,
                "vault locked; skipping this instance's master-anchor refresh check this pass"
            );
            return;
        }
        let now = now as i64;
        let interval = self.master_anchor_refresh_interval_secs as i64;
        let mut refreshed: BTreeSet<String> = BTreeSet::new();
        for svc in &plan.services {
            let master_did = svc.service_id.to_string();
            // Two services naming one master (not reachable from today's
            // compiler, but cheap to be right about) share one anchor and
            // must not each republish it in the same pass.
            if !refreshed.insert(master_did.clone()) {
                continue;
            }
            let last = self.store.last_master_anchor_refresh(&master_did).unwrap_or(None);
            // `at > now` (a backwards clock step, or a restored database)
            // must count as due immediately, not be suppressed until the
            // wall clock catches back up to it.
            if last.is_some_and(|at| at <= now && now.saturating_sub(at) < interval) {
                continue;
            }
            let master = match keys::master_for_member(
                &self.vault,
                &plan.app_instance_id.to_string(),
                svc.logical_ref.service_name.as_str(),
                svc.member_index,
            )
            .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(master_did, error = %e, "cannot read this master to refresh its anchor");
                    continue;
                }
            };
            match writer.refresh(&master).await {
                Ok(()) => {
                    if let Err(e) = self.store.record_master_anchor_refresh(&master_did, now) {
                        tracing::warn!(master_did, error = %e, "failed to stamp a master-anchor refresh");
                    }
                }
                Err(e) => tracing::warn!(
                    master_did,
                    error = %e,
                    "failed to refresh a master anchor; retrying on a later pass"
                ),
            }
        }
    }

    /// Publishes or refreshes this instance's Tier-1 registry record
    /// (ADR-0022 §2) -- "which supervisor holds this app" -- once
    /// `master_anchor_refresh_interval_secs` has elapsed since the last
    /// successful publish. Evaluated on the ordinary pass tick against a
    /// persisted fact, the same shape `refresh_due_master_anchors` uses,
    /// reusing its interval rather than a second config field.
    ///
    /// Keyed by `state.app_master_did`, not `app_instance_id`: the fact
    /// belongs to the DID being published, not to the human name, so a
    /// handover that changes which DID this instance publishes under
    /// (`import-master` under a new key, then `adopt`) starts that new
    /// DID's own refresh history at "never refreshed" rather than
    /// inheriting the old DID's recent stamp.
    ///
    /// Skipped, never minted, when this instance has no app master DID on
    /// its row yet: an instance adopted before that column existed gains
    /// one at its next `adopt`, and nowhere else -- minting here would
    /// create an app identity outside `adopt`, the one place that owns it.
    ///
    /// A locked vault raises `AlertKind::VaultLocked` (app-level,
    /// `logical_ref: None`) rather than only logging: the per-member raise
    /// from certificate renewal only fires for an instance with a member
    /// inside its near-expiry window, so an instance with none would
    /// otherwise get no signal at all while this record decays. Cleared
    /// once a refresh succeeds again.
    async fn refresh_due_app_tier1_record(
        &self,
        instance_id: &AppInstanceId,
        state: &DesiredState,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let Some(writer) = &self.tier1_writer else { return };
        if state.app_master_did.is_empty() {
            return;
        }
        let now_i = now as i64;
        let interval = self.master_anchor_refresh_interval_secs as i64;
        let last = self.store.last_tier1_refresh(&state.app_master_did).unwrap_or(None);
        // `at > now_i` (a backwards clock step, or a restored database)
        // must count as due immediately, not be suppressed until the wall
        // clock catches back up to it.
        if last.is_some_and(|at| at <= now_i && now_i.saturating_sub(at) < interval) {
            return;
        }
        let signed = match tier1::sign_tier1_record(
            &self.vault,
            &state.app_instance_id,
            &state.app_master_did,
            &self.node_did,
            state.generation,
            self.master_anchor_refresh_interval_secs,
        )
        .await
        {
            Ok(s) => s,
            // Reached by attempting the read, not by pre-checking
            // `kek_is_loaded()` first (A7's D-A7-1 shape, not A5d's): that
            // check reads the `KeyStore`, not whether the storage
            // provider's own encryption is even on, so on a node with
            // `storage.encryption = false` it always answers `false` even
            // though every vault read succeeds -- a pre-check here would
            // skip this instance's Tier-1 publish forever, silently, on
            // exactly that node, and raise a `VaultLocked` alert that is
            // never true. Reading `VaultError::Locked` off the real
            // attempt is correct on both an encrypted-and-locked vault and
            // an unencrypted one, and additionally catches the vault
            // locked *between* an early check and this call, which a
            // pre-check cannot.
            Err(tier1::Tier1SignError::Vault(keys::VaultError::Locked)) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    "vault locked; skipping this instance's Tier-1 record refresh this pass"
                );
                if let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::VaultLocked,
                    &format!(
                        "'{}' cannot refresh its Tier-1 registry record because this supervisor's \
                         vault is locked; callers outside the app will lose the ability to \
                         discover its supervisor once the currently-published record lapses. Run: \
                         roymctl --substrate {} security inject-kek --kek-hex <...>",
                        state.app_instance_id, self.node_did
                    ),
                ) {
                    opened.push((AlertKind::VaultLocked, state.app_instance_id.clone()));
                }
                return;
            }
            Err(e @ tier1::Tier1SignError::IdentityMismatch { .. }) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    error = %e,
                    "refusing to publish this instance's Tier-1 record under the wrong identity"
                );
                if let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::AppIdentityMismatch,
                    &e.to_string(),
                ) {
                    opened.push((AlertKind::AppIdentityMismatch, state.app_instance_id.clone()));
                }
                return;
            }
            Err(e) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    error = %e,
                    "cannot sign this instance's Tier-1 record this pass"
                );
                return;
            }
        };
        let _ = self.store.alerts.clear(instance_id, None, &self.node_did, AlertKind::VaultLocked);
        let _ = self.store.alerts.clear(
            instance_id,
            None,
            &self.node_did,
            AlertKind::AppIdentityMismatch,
        );
        match writer.publish(&signed).await {
            Ok(()) => {
                if let Err(e) = self.store.record_tier1_refresh(&state.app_master_did, now_i) {
                    tracing::warn!(
                        app_instance_id = %state.app_instance_id,
                        error = %e,
                        "failed to stamp a Tier-1 refresh"
                    );
                }
            }
            Err(e) => tracing::warn!(
                app_instance_id = %state.app_instance_id,
                error = %e,
                "failed to publish this instance's Tier-1 record; retrying on a later pass"
            ),
        }
    }

    /// The plan to journal as this pass's new baseline, as distinct from
    /// `filtered_plan`, the (possibly smaller) plan this pass actually
    /// deploys (M05A A5c review finding A-1). Recording only the touched
    /// subset as `Active` made `Reconciler::compute_diff` -- which reads
    /// the *last* `Active` record wholesale -- forget every already-
    /// landed service the current pass did not happen to touch, so the
    /// next pass read it as missing and redeployed it, dropping today's
    /// subset out of its own new snapshot in turn: two services on two
    /// substrates alternate being redeployed forever instead of the loop
    /// converging. Keeps every service already believed landed
    /// (anything outside `needs_work`) plus whatever this pass is about
    /// to (re)land; excludes only a `needs_work` service with nowhere
    /// reachable to send it this pass, which genuinely has not landed.
    fn record_plan_for_pass(
        plan: &DeploymentPlan,
        needs_work: &BTreeSet<String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> DeploymentPlan {
        let mut record_plan = plan.clone();
        record_plan.services.retain(|s| {
            !needs_work.contains(&s.member_ref().to_string())
                || s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
        });
        record_plan
    }

    /// Whether `old` and `new` (the same member, before and after a
    /// resubmit) differ only in which member DIDs a dependency resolves to
    /// -- a membership change in one of `new`'s dependencies, and nothing
    /// else about this member itself (M05A A5e, D-A5e-7). `logical_ref` is
    /// already guaranteed equal: `Reconciler::diff_plans` matches `old` and
    /// `new` by `MemberRef`, which includes it. Also requires `schedule` to
    /// be unchanged: a simultaneous schedule edit must not
    /// classify as membership-only, or the schedule change is silently
    /// dropped from the plan this pass records.
    fn only_resolved_dependencies_changed(old: &PlannedService, new: &PlannedService) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.schedule == new.schedule
            && old.resolved_dependencies != new.resolved_dependencies
    }

    /// Whether `old` and `new` differ only in `schedule`.
    /// A schedule-only edit must not redeploy the service -- the
    /// substrate has no use for the change at all (`ServiceSpec.schedule`'s
    /// own doc) -- and has nothing to push either, since it names no
    /// substrate-visible fact.
    fn only_schedule_changed(old: &PlannedService, new: &PlannedService) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.resolved_dependencies == new.resolved_dependencies
            && old.schedule != new.schedule
    }

    /// Splits a diff's `Update` actions into (a) members no caller should
    /// redeploy this pass and (b) the subset of those that need a binding
    /// push. The two are not the same set: a member
    /// whose only change is its schedule must not be redeployed (the
    /// substrate has no use for the change) and has nothing to push
    /// either -- so it joins the exclusion set but never the push list.
    ///
    /// The asymmetry in the landed-placement check below is deliberate: a
    /// membership push needs a substrate to push *to*, so a never-landed
    /// member falls through to the redeploy path. A schedule exclusion
    /// needs no substrate, so it applies whether or not the member has
    /// landed.
    ///
    /// Shared by the loop's write phase (`reconcile_instance_pass`) and an
    /// operator-triggered apply (`apply_with_membership_pushes`, under
    /// `handle_submit`/`deploy_submission`) so both make the identical
    /// redeploy-vs-push-vs-exclude call for the identical diff -- fixing
    /// findings §33.7/D-A5e-7 for one path and not the other is exactly the
    /// gap the M05A review round found.
    fn classify_update_actions(
        landed: &[ActionRecord],
        actions: &[ReconcileAction],
    ) -> (BTreeSet<String>, Vec<(PlannedService, String)>) {
        let mut redeploy_exclusions = BTreeSet::new();
        let mut push_candidates = Vec::new();
        for action in actions {
            if let ReconcileAction::Update { old, new } = action {
                let member_ref = new.member_ref().to_string();
                if Self::only_schedule_changed(old, new) {
                    redeploy_exclusions.insert(member_ref);
                    continue;
                }
                let landed_row = Self::only_resolved_dependencies_changed(old, new)
                    .then(|| deploy::current_placement(landed, &member_ref))
                    .flatten();
                if let Some(row) = landed_row {
                    redeploy_exclusions.insert(member_ref);
                    push_candidates.push(((**new).clone(), row.substrate_did.clone()));
                }
            }
        }
        (redeploy_exclusions, push_candidates)
    }

    /// The loop's redeploy work list (D-A5c-2/D-A5c-3/D-A5c-21): the diff's
    /// `Add` and `Update` actions -- a plan-level change -- plus
    /// `missing_placement`, a service the current sweep finds with no
    /// landed placement at all, which a content-unchanged diff against an
    /// older `Active` snapshot cannot see on its own (D-A5c-10's gap).
    /// `Remove` is not work: a plan-level removal is never undeployed here,
    /// only raised as `OrphanedService` by the caller.
    ///
    /// `redeploy_exclusions` comes from `classify_update_actions`, and this
    /// is the loop's half of applying it -- the half a test can reach
    /// without a substrate to deploy at. Its counterpart on the operator
    /// path is the `retain` in `apply_with_membership_pushes`.
    fn redeploy_work_list(
        missing_placement: &BTreeSet<String>,
        actions: &[ReconcileAction],
        redeploy_exclusions: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut needs_work = missing_placement.clone();
        for action in actions {
            let member_ref = match action {
                ReconcileAction::Add(svc) => svc.member_ref().to_string(),
                ReconcileAction::Update { new, .. } => new.member_ref().to_string(),
                ReconcileAction::Remove(_) => continue,
            };
            if !redeploy_exclusions.contains(&member_ref) {
                needs_work.insert(member_ref);
            }
        }
        needs_work
    }

    /// Every schedule a plan declares, keyed by logical ref. A schedule is
    /// identical across a logical service's members, so the first member
    /// carrying one decides it for the whole group; `BTreeMap` keeps both
    /// the pass and the `schedules` listing in a deterministic order.
    /// Shared by the two so a schedule the pass acts on and a schedule the
    /// operator is shown can never be different sets.
    fn declared_schedules(plan: &DeploymentPlan) -> BTreeMap<String, &ScheduleSpec> {
        let mut groups: BTreeMap<String, &ScheduleSpec> = BTreeMap::new();
        for svc in &plan.services {
            if let Some(sched) = &svc.schedule {
                groups.entry(svc.logical_ref.to_string()).or_insert(sched);
            }
        }
        groups
    }

    /// The selection rule for one schedule this pass, pure over the pass's
    /// own inputs -- testable with a fixed
    /// `now`, no vault, no client, no store, the same reason
    /// `renewal_candidates` is pure.
    ///
    /// No `landed` argument: `health::ServiceHealth` already carries
    /// `substrate_did` and `member_index` from this pass's own report, and
    /// a member with `Signal::Healthy` is by definition one the sweep
    /// reached on a real substrate. Reading the placement from the report
    /// keeps this a pure fold over one input rather than a join across two
    /// that could disagree.
    fn schedule_decisions(
        plan: &DeploymentPlan,
        states: &BTreeMap<String, ScheduleState>,
        report: &health::HealthReport,
        now: u64,
        grace_secs: u64,
    ) -> Vec<ScheduleDecision> {
        let groups = Self::declared_schedules(plan);
        let mut decisions = Vec::with_capacity(groups.len());
        for (l_ref, sched) in groups {
            let Ok(cron) = sched.parsed() else {
                // A plan that got past validation with a bad cron can only
                // come from a hand-edited submission. Watermark and move
                // on; the failure is reported by the alert the run would
                // have raised.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            };
            let Some(state) = states.get(&l_ref) else {
                // First sight: created with `evaluated_at = now` and no
                // run, so a schedule never fires for the past.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            };
            let window_start =
                u64::try_from(state.evaluated_at).unwrap_or(0).max(now.saturating_sub(grace_secs));
            // Anything other than a definite yes -- including an
            // evaluation error -- is treated exactly as the parse failure
            // above: watermark and move on.
            if !matches!(has_occurrence_in(&cron, window_start, now), Ok(true)) {
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            }

            // The report is the single source for "which members are
            // runnable and where they are": a `Healthy` signal carries the
            // substrate that answered.
            let mut healthy: Vec<&health::ServiceHealth> = report
                .services
                .iter()
                .filter(|h| h.logical_ref.to_string() == l_ref && h.signal == Signal::Healthy)
                .collect();
            healthy.sort_by_key(|h| h.member_index);
            if healthy.is_empty() {
                // A skipped tick, not a late one -- failure-matrix row 11.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            }

            // Round-robin: the first member strictly after the last one
            // used, wrapping. A member that has gone away simply drops out
            // of the ring. `None` -- never run -- sorts below every index,
            // so the first tick starts at the lowest healthy member rather
            // than skipping it.
            let pick = healthy
                .iter()
                .find(|h| Some(h.member_index) > state.last_member_index)
                .copied()
                .unwrap_or(healthy[0]);
            decisions.push(ScheduleDecision::Run {
                logical_ref: l_ref,
                service_id: pick.service_id.clone(),
                substrate_did: pick.substrate_did.clone(),
                member_index: pick.member_index,
                schedule: sched.clone(),
            });
        }
        decisions
    }

    /// Runs every due schedule and advances every watermark, in the order
    /// `schedule_decisions` produced. Never touches the
    /// outbox: a scheduled run is never queued (ADR-0023 §3) --
    /// the intent expires, and the next tick is a better retry than a
    /// delivery hours later. `actors` is built with `actors_from_clients`,
    /// not `durable_actor`, for exactly that reason -- the same call
    /// `renew_due_members` already makes.
    #[allow(clippy::too_many_arguments)]
    async fn run_due_schedules(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        decisions: &[ScheduleDecision],
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let now_i64 = now as i64;
        for decision in decisions {
            match decision {
                ScheduleDecision::Watermark { logical_ref } => {
                    if let Err(e) =
                        self.store.record_schedule_evaluated(app_instance_id, logical_ref, now_i64)
                    {
                        tracing::warn!(
                            app_instance_id,
                            logical_ref,
                            error = %e,
                            "failed to advance a schedule's watermark"
                        );
                    }
                }
                ScheduleDecision::Run {
                    logical_ref,
                    service_id,
                    substrate_did,
                    member_index,
                    schedule,
                } => {
                    // The two early `continue` branches deliberately
                    // advance the watermark rather than leaving it: the
                    // tick's window has passed and the target was not
                    // reachable, which is failure-matrix row 11's
                    // documented cost, not a delivery to retry.
                    let Some(alias) = did_to_alias.get(substrate_did) else {
                        let _ = self.store.record_schedule_evaluated(
                            app_instance_id,
                            logical_ref,
                            now_i64,
                        );
                        continue;
                    };
                    let Some(actor) = actors.get(&SubstrateAlias::new(alias.clone())) else {
                        let _ = self.store.record_schedule_evaluated(
                            app_instance_id,
                            logical_ref,
                            now_i64,
                        );
                        continue;
                    };

                    // Before the call, not after: a supervisor that dies
                    // inside the call must skip this tick on restart, not
                    // repeat it.
                    if let Err(e) = self.store.record_schedule_started(
                        app_instance_id,
                        logical_ref,
                        now_i64,
                        *member_index,
                    ) {
                        tracing::warn!(
                            app_instance_id,
                            logical_ref,
                            error = %e,
                            "failed to record a scheduled run's start; skipping this tick"
                        );
                        continue;
                    }

                    let budget = Duration::from_millis(u64::from(schedule.timeout_ms))
                        .min(SCHEDULED_RUN_CEILING);
                    let outcome = tokio::time::timeout(
                        budget,
                        actor.run_scheduled(
                            service_id.clone(),
                            generation,
                            schedule.interface.to_string(),
                            schedule.method.clone(),
                            schedule.params.clone(),
                        ),
                    )
                    .await;

                    match outcome {
                        Ok(Ok(())) => {
                            let _ = self.store.record_schedule_outcome(
                                app_instance_id,
                                logical_ref,
                                None,
                            );
                            // The sentinel, never `substrate_did` -- §0.9.
                            let _ = self.store.alerts.clear(
                                instance_id,
                                Some(logical_ref),
                                SCHEDULE_SUBSTRATE_DID,
                                AlertKind::ScheduledRunFailed,
                            );
                        }
                        Ok(Err(e)) => {
                            let detail = format!(
                                "scheduled run of '{}/{}' failed on substrate '{substrate_did}': \
                                 {e}",
                                schedule.interface, schedule.method
                            );
                            let _ = self.store.record_schedule_outcome(
                                app_instance_id,
                                logical_ref,
                                Some(&detail),
                            );
                            if let Ok(true) = self.store.alerts.raise(
                                instance_id,
                                Some(logical_ref),
                                None,
                                SCHEDULE_SUBSTRATE_DID,
                                AlertKind::ScheduledRunFailed,
                                &detail,
                            ) {
                                opened.push((AlertKind::ScheduledRunFailed, logical_ref.clone()));
                            }
                        }
                        Err(_elapsed) => {
                            let detail = format!(
                                "scheduled run of '{}/{}' on substrate '{substrate_did}' timed \
                                 out after {}ms",
                                schedule.interface,
                                schedule.method,
                                budget.as_millis()
                            );
                            let _ = self.store.record_schedule_outcome(
                                app_instance_id,
                                logical_ref,
                                Some(&detail),
                            );
                            if let Ok(true) = self.store.alerts.raise(
                                instance_id,
                                Some(logical_ref),
                                None,
                                SCHEDULE_SUBSTRATE_DID,
                                AlertKind::ScheduledRunFailed,
                                &detail,
                            ) {
                                opened.push((AlertKind::ScheduledRunFailed, logical_ref.clone()));
                            }
                        }
                    }
                }
            }
        }
    }

    /// One bounded restart attempt for a landed-but-`InstanceNotRunning`
    /// service (M05A A5c phase 6, §14 step 3, matrix row 13): refuses if
    /// this service's remediation is already terminal, or if it is still
    /// inside `restart_backoff_secs` of the last attempt; otherwise calls
    /// `SubstrateActor::restart`, records the attempt regardless of the
    /// call's own outcome (an attempt is an attempt -- the next sweep is
    /// what determines whether it worked), and raises
    /// `RemediationExhausted`, naming `force-reconcile` as the escape
    /// hatch (D-A5c-20), the moment `max_restart_attempts` is reached.
    /// Takes `Arc<dyn SubstrateActor>` so this is directly testable
    /// against a fake actor with no live substrate (§23 tests 35-38).
    #[allow(clippy::too_many_arguments)]
    async fn attempt_restart(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        logical_ref: &str,
        service_id: &str,
        substrate_did: &str,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let now = now as i64;
        let state = self.store.remediation_state(app_instance_id, logical_ref).unwrap_or(None);
        if state.is_some_and(|s| s.terminal) {
            return;
        }
        if let Some(RemediationState { last_attempt_at: Some(last), .. }) = state
            && now.saturating_sub(last) < self.restart_backoff_secs as i64
        {
            return;
        }

        if let Err(e) = actor.restart(service_id.to_string(), generation).await {
            tracing::warn!(app_instance_id, logical_ref, error = %e, "restart attempt failed");
        }

        let attempts = match self.store.record_restart_attempt(app_instance_id, logical_ref, now) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    logical_ref,
                    error = %e,
                    "failed to record this restart attempt"
                );
                return;
            }
        };
        if attempts < self.max_restart_attempts {
            return;
        }
        if let Err(e) = self.store.mark_remediation_terminal(app_instance_id, logical_ref) {
            tracing::warn!(
                app_instance_id,
                logical_ref,
                error = %e,
                "failed to mark remediation terminal"
            );
            return;
        }
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(logical_ref),
            None,
            substrate_did,
            AlertKind::RemediationExhausted,
            &format!(
                "bounded restart exhausted after {attempts} attempts with no confirmed recovery; \
                 run `supervisor force-reconcile` to try again"
            ),
        ) {
            opened.push((AlertKind::RemediationExhausted, logical_ref.to_string()));
        }
    }

    /// Publishes every alert this pass newly opened (D-A5-13, D-A5c-6):
    /// topic `<alert_topic>/<app_instance_id>`, namespaced under this
    /// node's `SUPERVISOR_RESERVED_SERVICE_ID` with the **publish-side**
    /// rule (`namespace_topic_for_publish`) -- the same rule the router's
    /// subscribe path for this one service id uses too (see `dispatch.rs`'s
    /// `handle_messaging_subscribe`), so the two strings match exactly.
    /// Messages are not retained (`MqttBroker::publish` is `try_publish`);
    /// `AlertStore` is the durable record and `alerts` the read surface.
    /// A publish failure is logged, never propagated -- the store write
    /// this reads from has already committed, so nothing here can lose an
    /// alert.
    async fn publish_opened_alerts(&self, app_instance_id: &str, opened: &[(AlertKind, String)]) {
        for (kind, label) in opened {
            let topic = namespace_topic_for_publish(
                SUPERVISOR_RESERVED_SERVICE_ID,
                &format!("{}/{app_instance_id}", self.alert_topic),
            );
            let payload = serde_json::json!({
                "app_instance_id": app_instance_id,
                "kind": kind.to_string(),
                "label": label,
            });
            let Ok(bytes) = serde_json::to_vec(&payload) else { continue };
            if let Err(e) = self.messaging_broker.publish(topic.clone(), bytes).await {
                tracing::warn!(
                    app_instance_id = %app_instance_id,
                    topic = %topic,
                    error = %e,
                    "failed to publish a newly-opened alert to MQTT; it is still stored and \
                     readable through `alerts`"
                );
            }
        }
    }

    fn require_admin(&self, caller: &CallerContext) -> RpcResult<()> {
        if caller.has_capability(
            &ResourceUri::substrate(&self.node_did),
            &Ability(Ability::SUBSTRATE_ADMIN.to_string()),
        ) {
            Ok(())
        } else {
            Err(RpcError::Custom(
                PERMISSION_DENIED_CODE,
                format!(
                    "caller {} holds no substrate/admin on this supervisor's node; the supervisor \
                     interface is node-owner only",
                    caller.caller_did
                ),
                None,
            ))
        }
    }

    async fn connected_client(
        &self,
        entry: &SupervisorInventoryEntry,
    ) -> anyhow::Result<SyneroymClient> {
        let identity = Identity::from_bytes(&self.client_identity_bytes);
        let mut client = SyneroymClient::new_with_identity(
            entry.did.clone(),
            entry.api_url.clone().unwrap_or_default(),
            identity,
        )
        .with_registry_dht(self.enable_registry_dht);
        if let Some(token) = &entry.ucan {
            client = client.with_ucan(token.clone());
        }
        client.wait_for_ready(MANAGED_SUBSTRATE_CONNECT_TIMEOUT).await?;
        Ok(client)
    }

    /// Every alias `handle_status` must connect to this pass, deduplicated
    /// (M05A A5c D-A5c-9/§19.9): the union of every alias the plan
    /// declares (needed for the generation read, which must reach a
    /// substrate even before anything has landed there) and every alias a
    /// landed placement names (needed for the health sweep). Pulled out
    /// as its own function so the dedup itself -- the whole point of the
    /// fix -- is directly unit-testable without a live substrate.
    fn connect_aliases_for_pass(
        plan_aliases: &BTreeSet<String>,
        did_to_alias: &BTreeMap<String, String>,
    ) -> Vec<String> {
        plan_aliases
            .iter()
            .cloned()
            .chain(did_to_alias.values().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Every alias the plan places a service on. Fails closed on a service
    /// with no placement at all: unlike `roymctl app deploy`, the
    /// supervisor has no operator present to supply a `--substrate`
    /// fallback, so an unplaced service can never be applied.
    fn placed_aliases(plan: &DeploymentPlan) -> Result<Vec<String>, String> {
        let mut aliases = BTreeSet::new();
        for svc in &plan.services {
            match &svc.substrate {
                Some(alias) => {
                    aliases.insert(alias.as_str().to_string());
                }
                None => {
                    return Err(format!(
                        "service '{}' has no substrate placement; the supervisor has no default \
                         substrate to fall back to",
                        svc.logical_ref
                    ));
                }
            }
        }
        Ok(aliases.into_iter().collect())
    }

    /// M05A A5c D-A5c-1 (§19.1): refuses a submission whose plan would move
    /// an already-landed service to a different substrate than the journal
    /// shows it running on. A5b shipped `submit` with no such refusal at
    /// all -- `roymctl`'s own `check_no_placement_change` is private to
    /// that binary and reads a local identity file the supervisor cannot
    /// see, so this is the supervisor's own check, not a reuse. Without
    /// it, a re-submit that changes an alias silently deploys a second
    /// live copy of the same member: the two-publisher state D-A3-12's
    /// refusal exists to prevent, reachable here because nothing on this
    /// path called it.
    ///
    /// Reads only what this supervisor's own journal has recorded landed
    /// -- never `roymctl`'s `--dir` -- so it is safe to call from both
    /// `submit` and `force-reconcile`.
    ///
    /// Review finding A-4: §21 q9 specifies that a refusal here raises
    /// `AlertKind::PlacementChangeRefused` -- the variant existed, tested
    /// only in its own `Display`/`FromStr` round trip, with nothing in
    /// either caller ever raising it. Raised (and published, same as
    /// every other alert this file opens) before the refusal is returned,
    /// so a refused submission is visible on `alerts` even though it is
    /// otherwise indistinguishable from a plain RPC error to whatever
    /// received it.
    /// D-A5e-14: `SynAppManifest::validate()` enforces `MAX_REPLICAS` at
    /// compile time, but `submit`/`force-reconcile` take an already-
    /// compiled `DeploymentPlan` straight as JSON -- nothing between the
    /// compiler and here re-checks it, so a submitted plan can carry an
    /// arbitrary member count for one logical service, each one a minted
    /// vault key, a certificate, a deploy call, and a journal row.
    /// Admin-gated, so this is not a privilege boundary, but the cap's own
    /// reason ("a bound set before the first measurement can never fail")
    /// does not hold if the interface that actually accepts the plan
    /// never enforces it.
    fn refuse_replicas_above_cap(plan: &DeploymentPlan) -> Result<(), String> {
        let mut counts: BTreeMap<&LogicalServiceRef, u32> = BTreeMap::new();
        for svc in &plan.services {
            *counts.entry(&svc.logical_ref).or_insert(0) += 1;
        }
        if let Some((l_ref, count)) = counts.into_iter().find(|(_, count)| *count > MAX_REPLICAS) {
            return Err(format!(
                "'{l_ref}' names {count} members in this plan, above the cap of {MAX_REPLICAS}"
            ));
        }
        Ok(())
    }

    /// `refuse_replicas_above_cap`'s sibling, same reason
    /// and same two call sites: `SynAppManifest::validate()` enforces both
    /// of these rules at compile time, but `submit`/`force-reconcile` take
    /// an already-compiled plan, which nothing between the compiler and
    /// here re-checks -- so a hand-edited plan reaches the supervisor with
    /// neither rule applied.
    ///
    /// The cap counts distinct `logical_ref`s carrying a schedule, not
    /// members -- a schedule belongs to the logical service, so a scaled
    /// service with one schedule counts once, not once per member.
    ///
    /// The budget is checked at both ends, and the zero end is the reason
    /// this rule is here rather than left to the runtime clamp: the clamp
    /// is a `min`, so a zero survives it, and
    /// `tokio::time::timeout(Duration::ZERO, ..)` elapses before the call
    /// starts -- after `record_schedule_started` has already written the
    /// watermark. The tick is consumed, an alert is raised, and every
    /// later tick repeats the cycle, forever.
    ///
    /// Refusing the whole submission is right for these two and wrong for
    /// an unparseable cron, which is why the cron is deliberately not
    /// re-validated here: a bad cron degrades to `schedule_decisions`'
    /// watermark branch, which skips that one schedule and leaves the rest
    /// of the instance reconciling. A budget no run can finish in has no
    /// such graceful form -- there is nothing to degrade to.
    fn refuse_unrunnable_schedules(plan: &DeploymentPlan) -> Result<(), String> {
        let scheduled: BTreeSet<&LogicalServiceRef> =
            plan.services.iter().filter(|s| s.schedule.is_some()).map(|s| &s.logical_ref).collect();
        if scheduled.len() > MAX_SCHEDULED_SERVICES {
            return Err(format!(
                "{} services declare a schedule in this plan, above the cap of \
                 {MAX_SCHEDULED_SERVICES}",
                scheduled.len()
            ));
        }
        for svc in &plan.services {
            let Some(sched) = &svc.schedule else { continue };
            if sched.timeout_ms == 0 || sched.timeout_ms > MAX_SCHEDULE_TIMEOUT_MS {
                return Err(format!(
                    "'{}' declares a schedule timeout of {}ms in this plan; it must be between 1 \
                     and {MAX_SCHEDULE_TIMEOUT_MS}ms",
                    svc.logical_ref, sched.timeout_ms
                ));
            }
        }
        Ok(())
    }

    /// The manifest's two `sharding_strategy` rules, re-applied to an
    /// already-compiled plan. `SynAppManifest::validate` enforces both at
    /// compile time, and nothing between the compiler and here re-checks
    /// them --
    /// the exact gap the two functions above exist to close, now with a
    /// sharper consequence: a strategy that reaches this supervisor goes
    /// into a *signed* Tier-2 document (ADR-0022 §3), where a reader acts
    /// on it against member ids the plan's author chose.
    fn refuse_unshardable_plan(plan: &DeploymentPlan) -> Result<(), String> {
        let mut counts: BTreeMap<&LogicalServiceRef, u32> = BTreeMap::new();
        for svc in &plan.services {
            *counts.entry(&svc.logical_ref).or_insert(0) += 1;
        }
        for svc in &plan.services {
            let Some(strategy) = &svc.sharding_strategy else { continue };
            if matches!(strategy, ShardingStrategy::RangeSharding(_)) {
                return Err(format!(
                    "'{}' declares a range_sharding strategy in this plan; range sharding names \
                     concrete members by ServiceId, which is reachable only once shard \
                     rebalancing assigns them",
                    svc.logical_ref
                ));
            }
            if counts.get(&svc.logical_ref).copied().unwrap_or(0) <= 1 {
                return Err(format!(
                    "'{}' declares a sharding_strategy with one member in this plan; a strategy \
                     over one member is not a selection",
                    svc.logical_ref
                ));
            }
        }
        Ok(())
    }

    async fn refuse_placement_change(
        &self,
        plan: &DeploymentPlan,
        inventory: &SupervisorInventory,
    ) -> Result<(), String> {
        let instance_id =
            AppInstanceId::try_new(plan.app_instance_id.to_string()).map_err(|e| e.to_string())?;
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&instance_id)
            .map_err(|e| e.to_string())?;
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            let Some(prev) = deploy::current_placement(&landed, &l_ref) else { continue };
            let Some(alias) = &svc.substrate else { continue };
            let Some(entry) = inventory.get(alias.as_str()) else { continue };
            if prev.substrate_did != entry.did {
                let detail = format!(
                    "service '{l_ref}' is already deployed on substrate {} and this submission \
                     would place it on {} ('{alias}'); the supervisor does not relocate a running \
                     member -- undeploy it on the old substrate and clear its placement record \
                     (`roymctl app forget`) before resubmitting",
                    prev.substrate_did, entry.did
                );
                if let Ok(true) = self.store.alerts.raise(
                    &instance_id,
                    Some(&l_ref),
                    prev.substrate_alias.as_deref(),
                    &prev.substrate_did,
                    AlertKind::PlacementChangeRefused,
                    &detail,
                ) {
                    self.publish_opened_alerts(
                        &plan.app_instance_id.to_string(),
                        &[(AlertKind::PlacementChangeRefused, l_ref)],
                    )
                    .await;
                }
                return Err(detail);
            }
        }
        Ok(())
    }

    /// Connects one client per placed alias, refusing an alias absent from
    /// the inventory or carrying no credential (§11.2).
    async fn build_clients(
        &self,
        aliases: &[String],
        inventory: &SupervisorInventory,
    ) -> Result<BTreeMap<SubstrateAlias, Arc<SyneroymClient>>, String> {
        let mut clients = BTreeMap::new();
        for alias in aliases {
            let entry = inventory
                .get(alias)
                .ok_or_else(|| format!("no inventory entry for substrate alias '{alias}'"))?;
            if entry.ucan.is_none() {
                return Err(format!(
                    "substrate alias '{alias}' carries no credential (ucan) in the submitted \
                     inventory; the supervisor cannot act on it"
                ));
            }
            let client = self
                .connected_client(entry)
                .await
                .map_err(|e| format!("failed to connect to substrate alias '{alias}': {e}"))?;
            clients.insert(SubstrateAlias::new(alias.clone()), Arc::new(client));
        }
        Ok(clients)
    }

    /// The highest generation any placed, reachable substrate reports
    /// holding for this instance -- best-effort, so one unreachable
    /// substrate cannot hide a real supersession another one reports.
    ///
    /// `aliases` comes from the plan's own declared placement
    /// (`Self::placed_aliases`), not from this supervisor's journal: a
    /// journal-derived set is empty until *this* supervisor has itself
    /// landed a placement, which would make a competing supervisor's
    /// `adopt` on an instance that never finished its first deploy here
    /// undetectable (B4, Slice A5b review).
    ///
    /// Returns `None`, not `Some(0)`, when not one placed substrate could
    /// be reached and queried -- every failure (no inventory entry,
    /// connect failure, RPC error) previously folded into the same "0" a
    /// substrate with a genuinely empty management row also produces, so
    /// a supervisor that had lost its own `orchestrator/status` grant
    /// reported "not superseded" indefinitely instead of "cannot tell".
    ///
    /// M05A A5c D-A5c-9: takes already-connected clients, keyed by alias,
    /// rather than connecting itself -- `handle_status` used to connect to
    /// every substrate twice per call (once for the health sweep, once
    /// here), and this is now the same client set the sweep used.
    ///
    /// Takes `Arc<dyn SubstrateActor>` rather than a concrete
    /// `SyneroymClient` (M05A A5c §23) -- callers upcast their real,
    /// connected clients into this shape, and a test substitutes a fake
    /// one instead, so the superseded/skip decision this drives is
    /// testable with no live substrate.
    async fn max_held_generation_from_clients(
        app_instance_id: &str,
        aliases: &BTreeSet<String>,
        clients: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
    ) -> Option<u64> {
        let mut held_max = 0u64;
        let mut reached_any = false;
        for alias in aliases {
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else { continue };
            let Ok(generation) = client.held_generation(app_instance_id).await else { continue };
            reached_any = true;
            held_max = held_max.max(generation.unwrap_or(0));
        }
        reached_any.then_some(held_max)
    }

    /// Upcasts a connected client set into the trait-object shape
    /// `max_held_generation_from_clients` takes. Deliberately the plain,
    /// undurable constructor (M05B B1): every actor this builds is used for
    /// exactly one read, `held_generation`, and never for `write_bindings`
    /// -- durability would add a queue key with nothing meaningful to bind
    /// it to and no call that could ever use it.
    fn actors_from_clients(
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
        clients.iter().map(|(alias, c)| (alias.clone(), deploy::build_actor(c.clone()))).collect()
    }

    /// The durable constructor every other app-supervisor call site with a
    /// real client builds through (M05B B1, D-B1-4/D-B1-5): `write_bindings`
    /// on the returned actor attempts synchronously first and enqueues onto
    /// this supervisor's own outbox only on a transport failure (D-B1-1).
    /// Every other action stays exactly as undurable as `build_actor` would
    /// make it (D-B1-3/D-B1-12) -- that declaration lives inside
    /// `DurableActor` itself, not in which call sites choose this over
    /// `build_actor`.
    fn durable_actor(
        &self,
        client: Arc<SyneroymClient>,
        app_instance_id: &str,
        logical_ref: &str,
        substrate_did: &str,
    ) -> Arc<dyn SubstrateActor> {
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_did: substrate_did.to_string(),
        };
        let outbox = Arc::new(SupervisorOutbox::new(self.store.queue.clone()));
        deploy::build_durable_actor(
            client,
            substrate_did.to_string(),
            queue_key.to_string(),
            outbox,
        )
    }

    /// Raises or clears `AlertKind::SupervisorSuperseded` from a
    /// `max_held_generation_from_clients` read, and returns whether this
    /// instance is currently superseded (ADR-0021 §4 / matrix row 9).
    /// Shared by `handle_status` and the loop's own pass (M05A A5c
    /// D-A5c-11) so the two cannot read "superseded" two different ways.
    /// `held_max == None` (nothing reachable) leaves whatever alert state
    /// already exists untouched and reports "not superseded" -- clearing
    /// here would silently un-alert a real supersession just because the
    /// network is flaky right now, and raising would false-alarm on a
    /// transient outage; neither is honest, so this is only logged.
    fn update_superseded_alert(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        held_max: Option<u64>,
        generation: u64,
    ) -> Result<bool, String> {
        let Some(held_max) = held_max else {
            tracing::warn!(
                app_instance_id = %app_instance_id,
                "could not reach any placed substrate to check for supersession (matrix row 9); \
                 status cannot confirm this supervisor is still the sole writer"
            );
            return Ok(false);
        };
        let superseded = held_max > generation;
        if superseded {
            self.store
                .alerts
                .raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::SupervisorSuperseded,
                    &format!(
                        "a managed substrate now holds generation {held_max}, higher than this \
                         supervisor's {generation}; another supervisor has adopted this instance"
                    ),
                )
                .map_err(|e| e.to_string())?;
        } else {
            self.store
                .alerts
                .clear(instance_id, None, &self.node_did, AlertKind::SupervisorSuperseded)
                .map_err(|e| e.to_string())?;
        }
        Ok(superseded)
    }

    /// Closes each client's iroh endpoint explicitly, rather than letting
    /// it drop -- a dropped-not-closed `SyneroymClient` is exactly what
    /// iroh logs as "Endpoint dropped without calling `Endpoint::close`.
    /// Aborting ungracefully", and every RPC verb that connects to a
    /// managed substrate used to leave every client it opened for iroh to
    /// clean up on drop (S6, Slice A5b review). Only closes a client this
    /// call holds the sole `Arc` to -- if something else still references
    /// it, leaving it open is correct, not a leak.
    async fn shutdown_clients(clients: impl IntoIterator<Item = Arc<SyneroymClient>>) {
        for mut client in clients {
            if let Some(c) = Arc::get_mut(&mut client) {
                let _ = c.shutdown().await;
            }
        }
    }

    /// The mint/substitute/certify/apply pipeline shared by `submit` and
    /// `force-reconcile`. Returns the plan with masters substituted in,
    /// not just the minted list -- `handle_submit` used to re-run
    /// `mint_and_substitute` a second time on its own copy to get this
    /// same plan for storing as desired state (H5, Slice A5b review): one
    /// vault open and one `reveal_secret` per service for a value this
    /// call had already computed.
    async fn deploy_submission(
        &self,
        mut plan: DeploymentPlan,
        inventory: &SupervisorInventory,
        generation: u64,
    ) -> Result<(Vec<MintedMaster>, DeploymentPlan), String> {
        let aliases = Self::placed_aliases(&plan)?;

        // Mint before connecting anywhere: a locked vault or a bad plan
        // must fail before the supervisor spends a network round trip on
        // substrates it cannot yet certify anything for.
        let (minted, masters) =
            keys::mint_and_substitute(&mut plan, &self.vault).await.map_err(|e| e.to_string())?;

        let clients = self.build_clients(&aliases, inventory).await?;

        // However this returns, every client this call opened must be
        // closed -- not just on the success path (S6).
        let result =
            self.apply_with_membership_pushes(&plan, &masters, &clients, generation, minted).await;
        Self::shutdown_clients(clients.into_values()).await;
        result.map(|minted| (minted, plan))
    }

    /// Mints, certifies, and applies `plan`, except for whatever member the
    /// same classifier `reconcile_instance_pass` uses
    /// (`classify_update_actions`) would route to a binding push or
    /// exclude outright -- those get `push_bindings` after the redeploy of
    /// the rest, rather than a full `deploy_with_context` reinstall (M05A
    /// A5e, D-A5e-7). Shared by `deploy_submission` (`force-reconcile`) and
    /// `handle_submit`, which each used to call `apply_with_clients`
    /// directly over the whole plan: an operator resubmit that only scales
    /// a dependency now takes the exact same push path the loop's own write
    /// phase does for an identical diff, instead of reinstalling every
    /// dependent every time.
    ///
    /// A push failure does not stop the redeploy half, and a redeploy
    /// failure does not stop the pushes -- the two work lists are
    /// independent members, the same way the loop's write phase treats
    /// them.
    async fn apply_with_membership_pushes(
        &self,
        plan: &DeploymentPlan,
        masters: &BTreeMap<ServiceId, Identity>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        minted: Vec<MintedMaster>,
    ) -> Result<Vec<MintedMaster>, String> {
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&plan.app_instance_id)
            .unwrap_or_default();
        let (redeploy_exclusions, push_candidates) =
            match Reconciler::new(&self.store.journal).compute_diff(plan) {
                Ok(diff) => Self::classify_update_actions(&landed, &diff.actions),
                Err(_) => (BTreeSet::new(), Vec::new()),
            };

        let mut apply_plan = plan.clone();
        apply_plan.services.retain(|s| !redeploy_exclusions.contains(&s.member_ref().to_string()));

        let apply_result =
            self.apply_with_clients(&apply_plan, plan, masters, clients, generation, minted).await;
        let apply_result_is_ok = apply_result.is_ok();

        let mut opened = Vec::new();
        let mut push_errors = Vec::new();
        for (svc, substrate_did) in &push_candidates {
            let Some(client) = svc.substrate.as_ref().and_then(|a| clients.get(a)) else {
                // M05A A5e review (matrix row 11): visible on `alerts`, the
                // same as any other push failure, not just returned to
                // this call's own caller -- the resident loop's next pass
                // does not re-raise a fresh alert for the same cause until
                // this one clears. M05B B1: also queued, the same reason
                // the resident loop's own analogous branch is
                // (`enqueue_unreachable_push`'s doc comment) -- a fallback-
                // placed member with no client this call is exactly as
                // unreachable as one the resident loop could not connect
                // to, and needs the same durability.
                self.enqueue_unreachable_push(
                    &plan.app_instance_id,
                    &plan.app_instance_id.to_string(),
                    plan,
                    svc,
                    substrate_did,
                    generation,
                    "not connected to its landed substrate this call",
                    &mut opened,
                )
                .await;
                push_errors.push(format!(
                    "{}: not connected to its landed substrate this call",
                    svc.member_ref()
                ));
                continue;
            };
            let actor = self.durable_actor(
                client.clone(),
                &plan.app_instance_id.to_string(),
                &svc.member_ref().to_string(),
                substrate_did,
            );
            // `Deferred` means the push did not land this call -- must
            // count the same as an error here too (M05B B1 review
            // follow-on 1), so `submit`/`force-reconcile` reports it and
            // the downgrade below fires, same as the resident loop's own
            // call site. `Landed` with zero outcomes (every dependency
            // just removed) is a real success, not deferred.
            match self
                .push_bindings(
                    &plan.app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    &actor,
                    generation,
                    &mut opened,
                )
                .await
            {
                Ok(PushOutcome::Deferred) => {
                    push_errors.push(format!(
                        "{}: deferred to an already-pending queued delivery",
                        svc.member_ref()
                    ));
                }
                Ok(PushOutcome::Landed(_)) => {}
                Err(e) => push_errors.push(format!("{}: {e}", svc.member_ref())),
            }
        }
        self.publish_opened_alerts(&plan.app_instance_id.to_string(), &opened).await;

        // Review round 2, finding A: `apply_with_clients` above already
        // journaled `plan` -- the *full* desired state, including this
        // pushed member's new `resolved_dependencies` -- as `Active` the
        // moment the redeploy half landed, regardless of whether the
        // pushes below it then succeeded. Left alone, a failed push here
        // leaves that `Active` record as the next pass's diff baseline, so
        // `compute_diff` reads the member as already converged: not in
        // `needs_work` (it has a landed placement) and not a push
        // candidate either (nothing differs from the "desired" record
        // anymore), so the `BindingConflict` this call just raised is
        // never retried and never clears. Downgrading the just-journaled
        // record to `Degraded` makes `compute_diff` fall back to the
        // *previous* `Active` baseline instead, so the next pass sees the
        // same diff this call did and reclassifies the member as a push
        // candidate again -- the same recovery shape a partially-failed
        // redeploy already gets.
        //
        // Gated on `apply_result.is_ok()`, not just `push_errors` being
        // non-empty: `apply_with_clients` returns `Ok` only when it just
        // journaled *this* call's `record_plan` as `Active` -- if it
        // returned `Err` instead, either nothing was journaled this call
        // at all (a certify failure before the journal write, in which
        // case `get_latest` would read a stale, unrelated record left by
        // an earlier call and must not be touched), or it already
        // journaled `Degraded` itself (in which case there is nothing to
        // downgrade).
        if apply_result_is_ok
            && !push_errors.is_empty()
            && let Ok(Some(latest)) = self.store.journal.get_latest(&plan.app_instance_id)
            && latest.state == DeploymentState::Active
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id = %plan.app_instance_id,
                error = %e,
                "failed to mark this submit's record Degraded after a binding push did not land"
            );
        }

        match (apply_result, push_errors.is_empty()) {
            (Ok(minted), true) => Ok(minted),
            (Ok(_), false) => {
                Err(format!("binding push did not fully land: {}", push_errors.join("; ")))
            }
            (Err(e), true) => Err(e),
            (Err(e), false) => {
                Err(format!("{e}; binding push did not fully land: {}", push_errors.join("; ")))
            }
        }
    }

    /// `plan` is what this call actually mints, certifies, and deploys.
    /// `record_plan` is what gets journaled as the new baseline for
    /// `Reconciler::compute_diff` to read next time -- equal to `plan`
    /// for every full apply (`deploy_submission`, `handle_submit`), but
    /// deliberately wider than it for the loop's filtered pass (M05A
    /// A5c review finding A-1): see `record_plan_for_pass`'s own doc for
    /// why the two must not be conflated.
    async fn apply_with_clients(
        &self,
        plan: &DeploymentPlan,
        record_plan: &DeploymentPlan,
        masters: &BTreeMap<ServiceId, Identity>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        minted: Vec<MintedMaster>,
    ) -> Result<Vec<MintedMaster>, String> {
        // M05A A5d / D-A5d-15: the one place every certificate-minting
        // caller passes through -- the resident loop, `submit`, and
        // `force-reconcile` alike. Filtering here rather than only in the
        // renewal work-list is what makes revocation stick: `submit` and
        // `force-reconcile` both call this with the full stored plan, so
        // without this an ordinary resubmit would silently re-mint and
        // reinstall the very key the operator just revoked. Skipped, not
        // failed, the same way a placement-changed service is -- the rest
        // of the plan still reconciles.
        let app_instance_id = plan.app_instance_id.to_string();
        let revoked = self.store.revoked_placements(&app_instance_id).unwrap_or_default();
        // `None` on the ordinary path, so a plan carrying hex-inlined wasm
        // artifacts is not cloned just to discover nothing is revoked.
        let filtered: Option<(DeploymentPlan, DeploymentPlan)> = if revoked.is_empty() {
            None
        } else {
            let mut opened = Vec::new();
            if let Ok(instance_id) = AppInstanceId::try_new(app_instance_id.clone()) {
                for svc in &plan.services {
                    let l_ref = svc.member_ref().to_string();
                    if !revoked.contains(&l_ref) {
                        continue;
                    }
                    // This used to pass the *alias* for
                    // both arguments, so the alert row's `substrate_did`
                    // column held e.g. `edge-1` where every other call
                    // site records a real DID -- resolved through this
                    // pass's own connected clients instead, the same
                    // source `apply_with_clients`'s certify step already
                    // trusts for the substrate a service is placed on.
                    let substrate_did = svc
                        .substrate
                        .as_ref()
                        .and_then(|a| clients.get(a))
                        .map(|c| c.service_id().to_string())
                        .unwrap_or_default();
                    if let Ok(true) = self.store.alerts.raise(
                        &instance_id,
                        Some(&l_ref),
                        svc.substrate.as_ref().map(SubstrateAlias::as_str),
                        &substrate_did,
                        AlertKind::InstanceRevoked,
                        &format!(
                            "'{l_ref}' has a revoked instance key, so it is not reinstalled or \
                             re-certified; the rest of the plan still reconciles. Undeploy it \
                             separately if the process itself should stop"
                        ),
                    ) {
                        opened.push((AlertKind::InstanceRevoked, l_ref));
                    }
                }
            }
            self.publish_opened_alerts(&app_instance_id, &opened).await;
            let mut filtered = plan.clone();
            filtered.services.retain(|s| !revoked.contains(&s.member_ref().to_string()));
            // `record_plan` must keep the revoked member, not drop it.
            // This is the same baseline `Reconciler::
            // compute_diff` reads next pass -- filtering it here as well
            // as `plan` above tells the diff the member was never landed,
            // so every later pass reports it as a fresh `Add`, re-enters
            // `needs_work`, and lands right back here to be filtered out
            // again: a permanent no-op write, journaled forever. A
            // revoked member is not undeployed (the alert above says so
            // explicitly); it is still the same landed placement, just
            // one this supervisor will not re-mint or reinstall for --
            // so the baseline should keep saying it is there.
            Some((filtered, record_plan.clone()))
        };
        let (plan, record_plan) = match &filtered {
            Some((p, r)) => (p, r),
            None => (plan, record_plan),
        };

        let (instance_certs, registry_certs) = deploy::certify_placed_members(
            plan,
            masters,
            clients,
            None,
            self.renewed_cert_expires_hours,
        )
        .await
        .map_err(|e| e.to_string())?;

        let deployment_id = self
            .store
            .journal
            .append(record_plan, DeploymentState::Applying)
            .map_err(|e| e.to_string())?;
        // Deliberately the plain, undurable constructor (M05B B1): `deploy::
        // apply_plan` only ever calls `actor.apply_plan(..)` on these
        // targets, never `write_bindings`, and `apply_plan` is never queued
        // regardless (D-B1-12) -- there is no per-service logical ref to
        // bind a queue key to here anyway, since one alias's actor covers
        // every service placed on it.
        let targets: BTreeMap<SubstrateAlias, DeployTarget> = clients
            .iter()
            .map(|(alias, c)| {
                (
                    alias.clone(),
                    DeployTarget {
                        alias: Some(alias.clone()),
                        substrate_did: c.service_id().to_string(),
                        actor: deploy::build_actor(c.clone()),
                    },
                )
            })
            .collect();

        // M05A A5c §19.3/D-A5c-4: the counter always advances before a
        // write -- a deploy is an authoritative write like any other, so
        // every dependent service this apply touches gets a fresh epoch
        // here, not just the standalone push (phase 7). A service with no
        // declared dependencies emits no bindings at all, so its epoch is
        // never read; advancing it anyway would be harmless but pointless.
        let mut binding_epochs: BTreeMap<MemberRef, u64> = BTreeMap::new();
        for svc in &plan.services {
            if svc.resolved_dependencies.is_empty() {
                continue;
            }
            let epoch = self
                .store
                .advance_binding_epoch(
                    &plan.app_instance_id.to_string(),
                    &svc.member_ref().to_string(),
                )
                .map_err(|e| e.to_string())?;
            binding_epochs.insert(svc.member_ref(), epoch);
        }

        let report = deploy::apply_plan(
            ApplyRequest {
                plan,
                targets: &targets,
                fallback: None,
                instance_certificates: &instance_certs,
                registry_certificates: &registry_certs,
                // Always true on the supervisor's apply path (§12): the
                // supervisor holds masters by construction, so the
                // condition `roymctl app deploy` ties this flag to is
                // always met here.
                emit_bindings: true,
                generation,
                binding_epochs: &binding_epochs,
            },
            &self.store.journal,
            deployment_id,
        )
        .await
        .map_err(|e| e.to_string())?;

        self.store
            .journal
            .update_state(
                deployment_id,
                if report.is_complete() {
                    DeploymentState::Active
                } else {
                    DeploymentState::Degraded
                },
            )
            .map_err(|e| e.to_string())?;

        if !report.is_complete() {
            let failures: Vec<String> =
                report.failures.iter().map(|f| format!("{}: {}", f.member_ref, f.error)).collect();
            return Err(format!("deploy applied with failures: {}", failures.join("; ")));
        }

        Ok(minted)
    }

    /// One dependent member's bindings, at its next epoch, without a
    /// redeploy (M05A A5c phase 7, D-A5c-16): reuses `map_deployment_
    /// plan_to_wit`'s own binding-construction logic (called the same
    /// way `apply_plan` calls it internally, over `&[svc]`) rather than
    /// duplicating it, so the two paths cannot drift apart on what a
    /// binding looks like on the wire. Its production caller is the
    /// membership-change classifier in `reconcile_instance_pass`/
    /// `apply_write_phase` (M05A A5e, D-A5e-7).
    ///
    /// `Stale(held)` is retried exactly once, at `held + 1` (D-A5c-19 /
    /// F4): no re-read, since `Stale` already carries the number a
    /// second round trip would only relearn. `Conflict` is not retried --
    /// a second writer exists, and retrying would only race it again.
    /// Either failure raises `BindingConflict`, folded into `opened` so
    /// the caller can publish it the same way every other alert this
    /// pass raised gets published. A push that lands cleanly clears it
    /// instead (M05A A5e, D-A5e-8/§33.19) -- the clear site this alert
    /// kind never had, without which `Degraded` derived from it would be
    /// permanent.
    ///
    /// `substrate_did` is the member's real, already-landed substrate DID
    /// (M05A A5e, §33.21) -- not `svc.substrate`, an operator-chosen
    /// alias (empty when placement falls back), which used to be written
    /// into the alert's `substrate_did` column and could then never match
    /// a clear keyed on the real DID every other alert kind uses.
    #[allow(clippy::too_many_arguments)]
    async fn push_bindings(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        substrate_did: &str,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> Result<PushOutcome, String> {
        let app_instance_id = plan.app_instance_id.to_string();
        let l_ref = svc.member_ref().to_string();

        // M05B B1 review finding 1: this call advances the binding epoch
        // unconditionally, before every attempt. `DurableActor::write_
        // bindings` only enqueues on a transport failure, and its own
        // dedup guard (`already_pending`) discards a second enqueue for a
        // key that already has a pending row -- so a *second* transport
        // failure for the same key (this pass reconnects fine but the
        // write itself times out, a narrower case than the connect-level
        // failure `enqueue_unreachable_push` already guards this way)
        // would advance the local epoch again while the queue still only
        // holds the first, older-epoch payload. `written_epoch` -- read
        // back for convergence from the same counter this advances --
        // would then race ahead of what the worker can ever actually
        // deliver, and `is_converged` would read the eventual successful
        // delivery of the *queued* item as still unconverged. Checking
        // first, and deferring entirely to the queue when a row already
        // exists, is the same fix `enqueue_unreachable_push` already
        // applies to its own, more common case.
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.clone(),
            logical_ref: l_ref.clone(),
            substrate_did: substrate_did.to_string(),
        };
        // Deliberately not `SupervisorOutbox::already_pending`, whose
        // fail-*closed* default (an unreadable queue reads as "already
        // pending") is right for its own purpose -- a guard against
        // writing a duplicate row should err toward not writing. Here it
        // would mean the opposite: an unreadable queue silently skips the
        // live attempt and returns `Ok`, reporting success for a push that
        // never happened and was never durably queued either (M05B B1
        // review follow-on 1). Failing *open* instead is safe specifically
        // because the queue and every other supervisor table share one
        // connection (D-B1-5) -- a genuinely broken connection surfaces a
        // proper `Err` on the very next line's `advance_binding_epoch`
        // instead of a silent no-op.
        if self.store.queue.has_pending(&queue_key.to_string()).unwrap_or(false) {
            return Ok(PushOutcome::Deferred);
        }

        let epoch = self
            .store
            .advance_binding_epoch(&app_instance_id, &l_ref)
            .map_err(|e| e.to_string())?;
        // Review finding A-5: a `write_bindings` call that fails outright
        // (the dependent unreachable, matrix row 11) used to propagate
        // with `?`, before the alert-raising code below was ever reached
        // -- the alert only fired for a `Stale`/`Conflict` *outcome*, a
        // clean round trip reporting a problem, never for the round trip
        // itself failing.
        let outcomes = match self.write_bindings_at_epoch(plan, svc, actor, generation, epoch).await
        {
            Ok(o) => o,
            Err(e) => {
                self.raise_binding_push_failure(instance_id, substrate_did, &l_ref, &e, opened);
                return Err(e);
            }
        };

        let stale_held = outcomes.iter().find_map(|o| match o {
            BindingWriteOutcome::Stale(held) => Some(*held),
            _ => None,
        });
        let outcomes = if let Some(held) = stale_held {
            let retry_epoch = held + 1;
            self.store
                .set_binding_epoch_at_least(&app_instance_id, &l_ref, retry_epoch)
                .map_err(|e| e.to_string())?;
            match self.write_bindings_at_epoch(plan, svc, actor, generation, retry_epoch).await {
                Ok(o) => o,
                Err(e) => {
                    self.raise_binding_push_failure(instance_id, substrate_did, &l_ref, &e, opened);
                    return Err(e);
                }
            }
        } else {
            outcomes
        };

        let failed = outcomes
            .iter()
            .any(|o| matches!(o, BindingWriteOutcome::Stale(_) | BindingWriteOutcome::Conflict(_)));
        if failed {
            if let Ok(true) = self.store.alerts.raise(
                instance_id,
                Some(&l_ref),
                None,
                substrate_did,
                AlertKind::BindingConflict,
                &format!(
                    "a binding push for '{l_ref}' did not land cleanly after one retry: \
                     {outcomes:?}"
                ),
            ) {
                opened.push((AlertKind::BindingConflict, l_ref));
            }
        } else {
            let _ = self.store.alerts.clear(
                instance_id,
                Some(&l_ref),
                substrate_did,
                AlertKind::BindingConflict,
            );
        }
        Ok(PushOutcome::Landed(outcomes))
    }

    /// The alert half of matrix row 11: a push that fails to reach the
    /// dependent at all (not a clean `Stale`/`Conflict` outcome) still
    /// needs to be visible on `alerts`, the same `AlertKind` a bad
    /// outcome raises -- an operator reading `alerts` should not have to
    /// know which of the two shapes a failed push took.
    fn raise_binding_push_failure(
        &self,
        instance_id: &AppInstanceId,
        substrate_did: &str,
        l_ref: &str,
        error: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(l_ref),
            None,
            substrate_did,
            AlertKind::BindingConflict,
            &format!("a binding push for '{l_ref}' failed to reach the dependent: {error}"),
        ) {
            opened.push((AlertKind::BindingConflict, l_ref.to_string()));
        }
    }

    /// Builds the `binding-write` a real deploy would emit for `svc`
    /// alone, at `epoch`, and sends it -- the standalone half of
    /// `push_bindings`, split out so a retry at a different epoch is a
    /// second call to this, not a copy of the mapping logic.
    /// The `binding-write` a real deploy would emit for `svc` alone, at
    /// `epoch` -- pure, no actor, no store. Shared by `write_bindings_at_
    /// epoch` (which sends it through a live actor) and
    /// `enqueue_unreachable_push` (which has no actor to send it through
    /// at all and must still capture *what* would have been sent).
    fn build_binding_write(
        plan: &DeploymentPlan,
        svc: &PlannedService,
        generation: u64,
        epoch: u64,
    ) -> Result<BindingWrite, String> {
        let binding_epochs = BTreeMap::from([(svc.member_ref(), epoch)]);
        let wit_plan = map_deployment_plan_to_wit(
            plan,
            &[svc],
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
            generation,
            &binding_epochs,
        )
        .map_err(|e| e.to_string())?;
        let bindings = wit_plan
            .services
            .into_iter()
            .next()
            .and_then(|s| s.app_context)
            .map(|ctx| ctx.bindings)
            .unwrap_or_default();
        Ok(BindingWrite {
            service_id: svc.service_id.to_string(),
            app_instance_id: plan.app_instance_id.to_string(),
            bindings,
            generation,
        })
    }

    async fn write_bindings_at_epoch(
        &self,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        epoch: u64,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let write = Self::build_binding_write(plan, svc, generation, epoch)?;
        actor.write_bindings(write).await
    }

    /// The durable half of a push candidate this pass could not even reach
    /// an actor for -- no known alias for its landed DID, or a connect
    /// that timed out before a client existed to wrap in a `DurableActor`
    /// at all. `DurableActor::write_bindings` is what normally enqueues on
    /// a transport failure (D-B1-1), but that only fires *inside* an
    /// attempted call; a substrate this pass never managed to dial has no
    /// call to attempt. Left at "raise an alert and move on" (the shape
    /// this had before), a substrate that is durably offline -- the exact
    /// case ADR-0023's reference scenario is built around -- would never
    /// be queued at all, only ever reported.
    ///
    /// Advances the binding epoch itself, the same as `push_bindings`
    /// does before a live attempt: the queued payload must carry a real
    /// epoch for the epoch guard to mean anything once a worker delivers
    /// it, and skipping the advance here would leave every queued item
    /// from this pass sharing the stale epoch a *reachable* pass last
    /// used.
    #[allow(clippy::too_many_arguments)]
    async fn enqueue_unreachable_push(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        substrate_did: &str,
        generation: u64,
        reason: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let l_ref = svc.member_ref().to_string();
        self.raise_binding_push_failure(instance_id, substrate_did, &l_ref, reason, opened);
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: l_ref.clone(),
            substrate_did: substrate_did.to_string(),
        };
        let outbox = SupervisorOutbox::new(self.store.queue.clone());
        // A5e's own retry (falling `compute_diff` back to the previous
        // baseline on Degraded) reclassifies this member as a push
        // candidate every pass until its push lands, so this branch runs
        // repeatedly while the substrate stays offline. A pending row
        // already covers the intent; advancing the epoch again for a
        // write that will not even be queued would strand the local
        // counter ahead of whatever the eventually-delivered, earlier-
        // epoch write actually lands -- `is_converged` would then never
        // agree, even after delivery succeeds.
        if outbox.already_pending(&queue_key.to_string()) {
            return;
        }
        let epoch = match self.store.advance_binding_epoch(app_instance_id, &l_ref) {
            Ok(epoch) => epoch,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    l_ref,
                    error = %e,
                    "failed to advance the binding epoch for an unreachable push; not queued \
                     this pass"
                );
                return;
            }
        };
        let write = match Self::build_binding_write(plan, svc, generation, epoch) {
            Ok(write) => write,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    l_ref,
                    error = %e,
                    "failed to build the binding write for an unreachable push; not queued this \
                     pass"
                );
                return;
            }
        };
        outbox.enqueue(&queue_key.to_string(), substrate_did, &write).await;
    }

    async fn handle_submit(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (s,): (Submission,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse submit params: {e}")))?;
        // M05A A5c D-A5c-7: held for the whole call, so a loop pass or
        // another operator write for this same instance cannot interleave
        // with it (§19.7's read-then-write races).
        let lock = self.instance_lock(&s.app_instance_id);
        let _guard = lock.lock().await;

        let plan = DeploymentPlan::from_json(&s.plan_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid plan-json: {e}")))?;
        let inventory: SupervisorInventory = serde_json::from_str(&s.inventory_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid inventory-json: {e}")))?;

        // `deploy_submission`, the journal, and the vault key all derive
        // from `plan.app_instance_id`; the desired-state row and every
        // later `adopt`/`status`/`retire` key on `s.app_instance_id`
        // instead. A mismatch (both fields are caller-supplied) would
        // split the instance in two -- `status` querying the journal under
        // a key nothing wrote, `adopt` stamping a generation the substrate
        // never associates with the deployed services (S4, Slice A5b
        // review).
        if plan.app_instance_id.as_str() != s.app_instance_id {
            return Err(RpcError::InvalidParams(format!(
                "submission names app instance '{}' but its plan-json is compiled for '{}'",
                s.app_instance_id, plan.app_instance_id
            )));
        }

        // Checked before any deploy work runs, not only after (B3, Slice
        // A5b review): `store.submit`'s own guards, below, live past the
        // whole mint/certify/apply pipeline. For `retired` that used to
        // mean only a late rejection. For `generation` it is worse (N1,
        // Slice A5b review round 2): `deploy_submission` already presents
        // `s.generation` to the substrate's own `check_generation` on the
        // way there, and an `Ordering::Greater` presentation is *accepted*
        // there and advances the substrate's own stamp -- so a wrong
        // upward `--generation` would leave the substrate ahead of this
        // supervisor's own store the instant `store.submit`'s check then
        // refused to record it, making the supervisor immediately
        // superseded by its own write. One read covers both, so both are
        // checked before either has a chance to run.
        if let Some(existing) = self
            .store
            .get(&s.app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
        {
            if existing.retired {
                return Err(RpcError::InternalError(format!(
                    "app instance '{}' is retired; run `supervisor adopt` to resume managing it \
                     before submitting new desired state",
                    s.app_instance_id
                )));
            }
            if s.generation != existing.generation {
                return Err(RpcError::InternalError(format!(
                    "submit presented generation {}, but app instance '{}' is on record at \
                     generation {}; only `adopt` mints a new one -- run `supervisor adopt`, or \
                     omit --generation to resubmit at the current one",
                    s.generation, s.app_instance_id, existing.generation
                )));
            }
        }

        // M05A A5c D-A5c-1: checked in the same pre-flight as `retired`/
        // `generation` above, before any deploy work runs -- a changed
        // placement must be refused, not silently applied.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // D-A5e-14: the manifest-time cap re-checked at the interface that
        // actually accepts a compiled plan.
        Self::refuse_replicas_above_cap(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(&plan).map_err(RpcError::InternalError)?;
        validate_plan_visibility(&plan).map_err(|errs| RpcError::InvalidParams(errs.join("; ")))?;

        // Mint before connecting anywhere -- a locked vault or a bad plan
        // must fail before anything is persisted or a network round trip
        // spent (unchanged ordering from before this change). The
        // substituted plan is what the stored desired state carries, so
        // the loop and `force-reconcile` see real master DIDs, not the
        // compiler's fabricated ones (H5, Slice A5b review).
        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let mut plan = plan;
        let (minted, masters) = keys::mint_and_substitute(&mut plan, &self.vault)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let plan_json_substituted =
            plan.to_json().map_err(|e| RpcError::InternalError(e.to_string()))?;

        // ADR-0022 §6/§3: a per-logical-service topology epoch for
        // every service this plan names. Computed here, ahead of
        // `store.submit`'s durable write, alongside everything else that
        // can fail -- `service_topology` can refuse an inconsistent plan
        // (a compiler bug), and that must refuse the submit with nothing
        // written, not land a stored plan no later `resolve` can build a
        // document from. Over `plan` post-`mint_and_substitute`, so the
        // fingerprint is over the members a document will actually carry,
        // not the compiler's fabricated ids.
        let service_names: BTreeSet<_> =
            plan.services.iter().map(|svc| svc.logical_ref.service_name.clone()).collect();
        let mut topology_fingerprints = Vec::with_capacity(service_names.len());
        for service_name in service_names {
            let topo = topology::service_topology(&plan, &service_name)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            let fingerprint =
                topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref());
            topology_fingerprints.push((service_name, fingerprint));
        }

        // M05A A5c (matrix row 12): persisted here, before the deploy
        // attempt below -- so a substrate that is down or slow at this
        // exact moment does not stop the desired state itself from
        // becoming durable. Every check above (retired/generation/
        // placement) has already refused a configuration problem before
        // this point runs, so nothing that used to be refused before any
        // deploy work ran is now silently accepted instead. The resident
        // loop (or a later `force-reconcile`) retries whatever the
        // best-effort apply just below does not land.
        self.store
            .submit(
                &s.app_instance_id,
                &plan_json_substituted,
                &s.inventory_json,
                &caller.caller_did,
                s.generation,
            )
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Infallible in practice: a failure here is a stale epoch on an
        // otherwise-correct stored plan, which `resolve`'s own insert-only
        // backfill repairs on the next read. The cache eviction is
        // belt-and-braces -- `handle_resolve` re-signs on an epoch
        // mismatch anyway -- but removing it here means a scale-out is
        // visible without waiting for that comparison.
        for (service_name, fingerprint) in topology_fingerprints {
            let before =
                self.store.topology_epoch(&s.app_instance_id, service_name.as_str()).unwrap_or(0);
            match self.store.record_topology_fingerprint(
                &s.app_instance_id,
                service_name.as_str(),
                &fingerprint,
            ) {
                Ok(after) => {
                    if after != before {
                        self.signed_documents
                            .remove(&(s.app_instance_id.clone(), service_name.to_string()));
                    }
                }
                Err(e) => tracing::warn!(
                    app_instance_id = %s.app_instance_id,
                    %service_name,
                    error = %e,
                    "failed to record this submit's topology fingerprint; a later resolve will \
                     repair it"
                ),
            }
        }

        // Best-effort immediate apply: still surfaced to the caller as an
        // error if it does not fully land (an operator's `submit` should
        // know when nothing landed), but the desired state above is
        // already durable regardless of this outcome.
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;
        let apply_result = self
            .apply_with_membership_pushes(&plan, &masters, &clients, s.generation, minted)
            .await;
        Self::shutdown_clients(clients.into_values()).await;
        // Review finding D-3: this error and a pre-flight refusal
        // (retired/generation/placement, all above) used to read
        // identically to the caller -- a plain string -- despite being
        // opposites: a refusal wrote nothing and needs a corrected plan,
        // while reaching here means the desired state above is already
        // durable and the resident loop will retry whatever did not
        // land. Said explicitly so an operator does not have to already
        // know that ordering to read the error correctly.
        let minted = apply_result.map_err(|e| {
            RpcError::InternalError(format!(
                "desired state was recorded; the immediate apply did not fully land and will be \
                 retried by the resident loop: {e}"
            ))
        })?;

        let result = SubmitResult {
            masters: minted
                .into_iter()
                .map(|m| WitMintedMaster {
                    service_name: m.service_name,
                    master_did: m.master_did,
                    vault_name: m.vault_name,
                    member_index: m.member_index,
                })
                .collect(),
        };
        Ok(NativeResponse { payload: serde_json::to_value(result).unwrap_or(Value::Null) })
    }

    /// Reads the held generation across every given client and claims
    /// `held + 1` on each. Split out of `handle_adopt` so that function
    /// can close every client it opened however this returns, success or
    /// failure (N2, Slice A5b review round 2) -- `?` inside either loop
    /// here used to return straight out of `handle_adopt` itself, leaking
    /// every client already connected and every one still left to try.
    async fn claim_next_generation(
        app_instance_id: &str,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> RpcResult<u64> {
        let mut held_max = 0u64;
        for client in clients.values() {
            if let Some(g) =
                client.held_generation(app_instance_id).await.map_err(RpcError::InternalError)?
            {
                held_max = held_max.max(g);
            }
        }
        let next_generation = held_max + 1;

        for client in clients.values() {
            client
                .request(
                    "orchestrator",
                    "claim-app-instance",
                    serde_json::to_value((app_instance_id.to_string(), next_generation))
                        .map_err(|e| RpcError::InternalError(e.to_string()))?,
                )
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        }
        Ok(next_generation)
    }

    async fn handle_adopt(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse adopt params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;

        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'; run \
                     `supervisor submit` first"
                ))
            })?;

        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // M05A A7 (D-A7-1): resolved or minted before any substrate
        // connection is opened -- same ordering `submit`'s own mint uses
        // ("a locked vault or a bad plan must fail before anything is
        // persisted or a network round trip spent"). A locked vault fails
        // the whole call here, before `claim_next_generation` burns a
        // generation, through the ordinary `VaultError::Locked` message
        // (which already names `inject-kek`) rather than a `kek_is_loaded`
        // pre-check -- that check answers `false` on a working vault
        // whenever `storage.encryption = false`.
        let (app_master_did, app_master_vault_name) =
            keys::app_master(&self.vault, &app_instance_id)
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;

        let result = Self::claim_next_generation(&app_instance_id, &clients).await;
        Self::shutdown_clients(clients.into_values()).await;
        let next_generation = result?;

        // `adopt` is the way back in from `retired` -- the message every
        // refusal on a retired instance points to (N3, Slice A5b review
        // round 2). Idempotent when the instance was never retired.
        //
        // M05A A7 (D-A7-5, review finding 6): the generation, the
        // un-retired flag, and the resolved app master DID land in one
        // combined store write rather than three separate ones -- a crash
        // between them used to be able to leave a claimed generation with
        // no recorded app master, which is exactly the state D-A7-4's "the
        // row always agrees with the vault" claim rests on not happening.
        // The DID is written *after* the claim succeeds, deliberately
        // asymmetric with the mint above, which runs before it: a vault
        // key with no row is recoverable (the next `adopt` resolves the
        // same key), while a row naming a DID whose key was never stored
        // is not. Written on every successful `adopt`, not only the one
        // that minted, so the row always agrees with whatever the vault
        // holds -- this is what makes `import-master` followed by `adopt`
        // correct after a handover.
        self.store
            .record_adopt(&app_instance_id, next_generation, &app_master_did)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // D-A5c-20 (§19.20/F5): a fresh generation is a fresh start, so a
        // terminal `InstanceNotRunning` service -- one nothing will ever
        // restart again on its own -- becomes escapable here. Stays a
        // separate, best-effort call (unlike the combined write above):
        // its own failure has never blocked `adopt` from succeeding.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);
        // Same fresh-start reasoning, applied to what this supervisor has
        // *done* about each schedule. The watermark is kept, not cleared:
        // see `clear_schedule_state_for_instance` for why dropping it
        // would swallow a tick that was legitimately due.
        let _ = self.store.clear_schedule_state_for_instance(&app_instance_id);

        let result = AdoptResult {
            generation: next_generation,
            app_master_did,
            vault_name: app_master_vault_name,
        };
        Ok(NativeResponse { payload: serde_json::to_value(result).unwrap_or(Value::Null) })
    }

    /// `build_clients`' own contract is all-or-nothing (a deploy correctly
    /// wants that), which is wrong for release: an unreachable substrate
    /// must not stop this call from releasing every *other* substrate the
    /// instance is placed on (S7, Slice A5b review). Connects what it can
    /// and reports the rest as `(alias, reason)` instead of failing the
    /// whole batch on the first one that cannot be reached.
    async fn connect_best_effort(
        &self,
        aliases: &[String],
        inventory: &SupervisorInventory,
    ) -> (BTreeMap<SubstrateAlias, Arc<SyneroymClient>>, Vec<(String, String)>) {
        let mut clients = BTreeMap::new();
        let mut failed = Vec::new();
        for alias in aliases {
            // Shutdown must not wait out every remaining alias's own
            // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` -- unlike
            // `queue_worker_tick`'s per-item check (M05B B1 review finding
            // 5), `run()`'s outer `select!` only races cancellation against
            // *waiting for the next tick*, not against a pass already in
            // flight, so without a check here a pass stuck connecting to
            // one unreachable alias silently drags the whole shutdown out
            // by however many alias timeouts remain.
            if self.cancellation_token.is_cancelled() {
                break;
            }
            let Some(entry) = inventory.get(alias) else {
                failed.push((
                    alias.clone(),
                    "no inventory entry for this substrate alias".to_string(),
                ));
                continue;
            };
            if entry.ucan.is_none() {
                failed.push((
                    alias.clone(),
                    "substrate carries no credential (ucan) in the submitted inventory".to_string(),
                ));
                continue;
            }
            tokio::select! {
                () = self.cancellation_token.cancelled() => break,
                result = self.connected_client(entry) => match result {
                    Ok(client) => {
                        clients.insert(SubstrateAlias::new(alias.clone()), Arc::new(client));
                    }
                    Err(e) => failed.push((alias.clone(), e.to_string())),
                },
            }
        }
        (clients, failed)
    }

    /// Shared by `release` and `retire`: clears the management stamp on
    /// every substrate the instance is placed on that can actually be
    /// reached, and returns the `(alias, reason)` of every one that
    /// could not be -- reachable or not, `release`/`retire` still act on
    /// what they can (S7, Slice A5b review).
    async fn release_on_every_substrate(
        &self,
        app_instance_id: &str,
    ) -> RpcResult<Vec<(String, String)>> {
        let state = self
            .store
            .get(app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let (clients, mut failed) = self.connect_best_effort(&aliases, &inventory).await;

        let params = serde_json::to_value((app_instance_id.to_string(), state.generation))
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        for (alias, client) in &clients {
            if let Err(e) =
                client.request("orchestrator", "release-app-instance", params.clone()).await
            {
                failed.push((alias.to_string(), e.to_string()));
            }
        }
        Self::shutdown_clients(clients.into_values()).await;
        Ok(failed)
    }

    /// A JSON payload reporting which, if any, placed substrates could not
    /// be released -- `unreleased_substrates` is present and non-empty
    /// only then, so an operator (and `roymctl`'s own printout) can tell a
    /// clean release from a partial one without parsing prose.
    fn release_payload(status: &str, failed: Vec<(String, String)>) -> Value {
        if failed.is_empty() {
            return serde_json::json!({"status": status});
        }
        serde_json::json!({
            "status": status,
            "warning": "one or more placed substrates could not be reached; their generation \
                        stamp was not cleared and must be released once they are reachable \
                        again",
            "unreleased_substrates": failed
                .into_iter()
                .map(|(alias, reason)| serde_json::json!({"alias": alias, "reason": reason}))
                .collect::<Vec<_>>(),
        })
    }

    async fn handle_release(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse release params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let failed = self.release_on_every_substrate(&app_instance_id).await?;
        Ok(NativeResponse { payload: Self::release_payload("released", failed) })
    }

    /// `retire` withdraws nothing from the registry: a retired instance
    /// drops out of `all_active`, so its Tier-1 record (if any) simply
    /// stops refreshing and lapses on the registry's own TTL/`not_after`,
    /// the same self-limiting decay a pause causes. Left implicit rather
    /// than an explicit withdraw, since nothing else in this slice
    /// withdraws a record early either.
    async fn handle_retire(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse retire params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let failed = self.release_on_every_substrate(&app_instance_id).await?;
        // Retiring must not be blocked by a substrate that happens to be
        // down right now -- exactly the state an operator is most likely
        // to be retiring around (S7). The supervisor's own store always
        // stops managing the instance; an unreachable substrate keeps its
        // stale stamp, reported above, until it comes back and is
        // released.
        self.store.retire(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: Self::release_payload("retired", failed) })
    }

    async fn handle_pause(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse pause params: {e}")))?;
        self.store.pause(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        let mut payload = serde_json::json!({"status": "paused"});
        // A paused instance gets zero write-phase work (D-A5c-1), so its
        // Tier-1 registry record stops refreshing along with everything
        // else -- `pause`'s own promise ("stops reconciliation and
        // nothing else") does not cover this, since the record decays
        // toward `not_after` on the clock, not on a reconcile. Rather than
        // reopen that promise, the cost is made visible here, at the
        // moment an operator chooses it. The refresh fact is keyed by the
        // app master DID, not the instance id (a handover must not inherit
        // a stale stamp) -- an instance with no DID on its row yet, or one
        // never published, has nothing to warn about.
        let app_master_did =
            self.store.get(&app_instance_id).ok().flatten().map(|s| s.app_master_did);
        if let Some(app_master_did) = app_master_did.filter(|did| !did.is_empty())
            && let Ok(Some(last)) = self.store.last_tier1_refresh(&app_master_did)
        {
            let expires_at = (last as u64).saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
            tracing::warn!(
                app_instance_id,
                expires_at,
                "pausing this instance stops its Tier-1 registry record from refreshing; callers \
                 outside it will stop being able to discover its supervisor once it passes this \
                 Unix time, unless the instance is resumed before then"
            );
            payload["app_record_expires_at"] = serde_json::json!(expires_at);
        }
        Ok(NativeResponse { payload })
    }

    async fn handle_resume(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse resume params: {e}")))?;
        self.store.resume(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "resumed"}) })
    }

    async fn handle_force_reconcile(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse force-reconcile params: {e}"))
        })?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        // Unlike `submit`, this path never calls `store.submit`, so nothing
        // else on it would ever refuse a retired instance -- it would just
        // redeploy every service indefinitely (B3, Slice A5b review).
        if state.retired {
            return Err(RpcError::InternalError(format!(
                "app instance '{app_instance_id}' is retired; run `supervisor adopt` to resume \
                 managing it before reconciling"
            )));
        }
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // M05A A5c D-A5c-1: `force-reconcile` never calls `store.submit`,
        // so nothing else on this path checks placement either -- the
        // identical fixture-trick reasoning as the `retired` check above.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // D-A5e-14: the same re-check `submit` runs -- a desired-state row
        // written before this check existed must not get a permanent pass.
        Self::refuse_replicas_above_cap(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(&plan).map_err(RpcError::InternalError)?;
        // D-A5c-20 (§19.20/F5): a directed reconcile is a fresh start,
        // regardless of what this call's own outcome turns out to be --
        // a terminal `InstanceNotRunning` service is otherwise never
        // restarted again, so the loop's own healthy-sweep clearing path
        // never fires for it.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);
        let _ = self.store.clear_schedule_state_for_instance(&app_instance_id);
        self.deploy_submission(plan, &inventory, state.generation)
            .await
            .map_err(RpcError::InternalError)?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "reconciled"}) })
    }

    async fn handle_export_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse export-master params: {e}"))
        })?;
        let path = self
            .vault
            .export_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse {
            payload: serde_json::to_value(path.to_string_lossy().into_owned())
                .unwrap_or(Value::Null),
        })
    }

    async fn handle_import_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse import-master params: {e}"))
        })?;
        self.vault
            .import_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "imported"}) })
    }

    /// The DID `revoke-instance` actually anchors as revoked. Read from
    /// the hosting substrate rather than a stored table: the substrate is
    /// the authority on what key is actually installed.
    ///
    /// `instance_did` is what *this caller* (this supervisor) would
    /// derive -- correct for the certify flow that reads it before
    /// anything is installed, wrong here whenever the installed
    /// certificate was minted for a different caller (a member deployed
    /// by an operator and only later adopted, not yet redeployed).
    /// Revoking the derived DID in that case anchors a key nothing
    /// presents, while the key actually in use stays fully trusted -- so
    /// this prefers `installed_temporary_did`, the substrate's ground
    /// truth for what is installed right now, and only falls back to the
    /// derived DID when nothing is installed yet (nothing to read, so the
    /// prospective key is the closest thing to "the key this placement
    /// would use"). A free function of the RPC's answer alone, so the
    /// choice is directly testable without a live client.
    fn select_revocation_did(identity: syneroym_sdk::InstanceIdentity) -> String {
        identity.installed_temporary_did.unwrap_or(identity.instance_did)
    }

    /// Revoke one placed member's instance key: append its derived DID to
    /// the master anchor's revoked list, then record the placement revoked
    /// so nothing mints it a fresh certificate afterwards.
    ///
    /// Under the instance lock for the whole verb, the same discipline
    /// every other instance-scoped write follows. Without it, this and a
    /// resident pass's renewal of the same member race: the pass could mint
    /// and install a fresh certificate in the gap between the anchor write
    /// and the exclusion write landing, which is precisely the window this
    /// verb exists to close.
    ///
    /// Order matters. The local exclusion is written **after** the anchor
    /// publish succeeds, so a failed publish leaves the placement under
    /// ordinary management rather than half-revoked -- excluded from
    /// renewal here while still fully trusted by every consumer.
    async fn handle_revoke_instance(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, logical_ref): (String, String) = serde_json::from_value(params)
            .map_err(|e| {
                RpcError::InvalidParams(format!("failed to parse revoke-instance params: {e}"))
            })?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;

        // Checked here as well as inside `record_revocation`, so a node
        // with no registry refuses before spending a round trip resolving
        // an instance identity it can do nothing with.
        if self.anchor_writer.is_none() {
            return Err(RpcError::InternalError(
                "this supervisor's node has no registry configured (substrate.registry_url), so \
                 it cannot publish a revocation; a revocation nothing can resolve is not a \
                 revocation"
                    .to_string(),
            ));
        }

        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let svc =
            plan.services.iter().find(|s| s.member_ref().to_string() == logical_ref).ok_or_else(
                || {
                    RpcError::InvalidParams(format!(
                        "app instance '{app_instance_id}' has no member '{logical_ref}' in its \
                         stored plan"
                    ))
                },
            )?;
        let alias = svc.substrate.as_ref().ok_or_else(|| {
            RpcError::InternalError(format!("member '{logical_ref}' has no substrate placement"))
        })?;
        let entry = inventory.get(alias.as_str()).ok_or_else(|| {
            RpcError::InternalError(format!("no inventory entry for substrate alias '{alias}'"))
        })?;

        // `select_revocation_did`'s own doc explains the choice below.
        let mut client = self
            .connected_client(entry)
            .await
            .map_err(|e| RpcError::InternalError(format!("failed to reach '{alias}': {e}")))?;
        let identity = client.instance_identity(svc.service_id.as_str()).await;
        let _ = client.shutdown().await;
        let identity = identity.map_err(|e| {
            RpcError::InternalError(format!(
                "failed to resolve the instance identity for '{logical_ref}': {e}"
            ))
        })?;
        let instance_did = Self::select_revocation_did(identity);

        self.record_revocation(
            &app_instance_id,
            &logical_ref,
            svc.logical_ref.service_name.as_str(),
            svc.member_index,
            &instance_did,
        )
        .await
        .map_err(RpcError::InternalError)?;

        Ok(NativeResponse {
            payload: serde_json::json!({
                "status": "revoked",
                "instance_did": instance_did,
                "note": "the member's process is still running; undeploy it separately if that is \
                         intended",
            }),
        })
    }

    /// `revoke-instance`'s two writes, once the instance DID is known.
    /// Split from the verb so the ordering below is exercisable without a
    /// live substrate answering `resolve-instance-identity` -- which is the
    /// only reason the verb needs a network at all.
    ///
    /// The anchor publish comes first and the local exclusion only after it
    /// succeeds. Reversed, a failed publish would leave the placement
    /// half-revoked: excluded from renewal here, while every consumer still
    /// fully trusts the key -- so it would quietly age out instead of
    /// failing closed, which is the opposite of what was asked for.
    async fn record_revocation(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        service_name: &str,
        member_index: u32,
        instance_did: &str,
    ) -> Result<(), String> {
        let writer = self.anchor_writer.as_ref().ok_or_else(|| {
            "this supervisor's node has no registry configured (substrate.registry_url), so it \
             cannot publish a revocation"
                .to_string()
        })?;
        let master =
            keys::master_for_member(&self.vault, app_instance_id, service_name, member_index)
                .await
                .map_err(|e| e.to_string())?;
        writer
            .revoke_instance(&master, instance_did)
            .await
            .map_err(|e| format!("failed to publish the revocation: {e}"))?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.store
            .revoke_placement(app_instance_id, logical_ref, now as i64)
            .map_err(|e| e.to_string())
    }

    /// The read half of D-A5c-4/D-A5c-5 (§19.4): per declared dependency
    /// of every dependent in the plan, what this supervisor last wrote
    /// (`SupervisorStore::binding_epoch`) versus what the sweep's
    /// `HealthReport` observed the hosting substrate serving *for that
    /// dependent* (`ServiceHealth.binding_epochs`, keyed by dependency
    /// name). A dependent absent from the report (unreachable, or no
    /// completed placement) reports every one of its dependencies
    /// `observed_epoch: None`, `converged: false` -- unconverged, not
    /// silently absent from the list, so an operator sees the gap rather
    /// than an empty table that looks like nothing was ever declared.
    fn binding_convergence_rows(
        &self,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        report: &health::HealthReport,
    ) -> Vec<BindingConvergence> {
        let mut rows = Vec::new();
        for svc in &plan.services {
            if svc.resolved_dependencies.is_empty() {
                continue;
            }
            let dependent_ref = svc.member_ref().to_string();
            let written_epoch =
                self.store.binding_epoch(app_instance_id, &dependent_ref).unwrap_or(0);
            let observed: BTreeMap<&str, u64> = report
                .services
                .iter()
                .find(|s| s.member_ref().to_string() == dependent_ref)
                .map(|s| s.binding_epochs.iter().map(|(n, e)| (n.as_str(), *e)).collect())
                .unwrap_or_default();
            for dependency_name in svc.resolved_dependencies.keys() {
                let observed_epoch = observed.get(dependency_name.as_str()).copied();
                rows.push(BindingConvergence {
                    dependent_logical_ref: dependent_ref.clone(),
                    dependency_name: dependency_name.to_string(),
                    written_epoch,
                    observed_epoch,
                    converged: observed_epoch == Some(written_epoch),
                });
            }
        }
        rows
    }

    /// Services this pass's sweep reported `InstanceNotRunning` **and**
    /// landed (a real `substrate_did`) -- restart candidates (M05A A5c
    /// phase 6, §21/D-A5c-17). Deliberately excludes `ProbeFailing` (an
    /// author-declared assertion, not a substrate-verified fact -- alert
    /// only, per §21's own three reasons) and `SubstrateUnreachable`
    /// (D-A4-13: restarting cannot fix a substrate that did not answer).
    /// Its own function so this filter is directly testable against a
    /// synthetic `HealthReport`, with no live substrate (§23 tests
    /// 39-40).
    fn restart_candidates(report: &health::HealthReport) -> Vec<(String, String, String)> {
        report
            .services
            .iter()
            .filter(|s| {
                matches!(s.signal, Signal::InstanceNotRunning(_)) && !s.substrate_did.is_empty()
            })
            .map(|s| (s.member_ref().to_string(), s.service_id.clone(), s.substrate_did.clone()))
            .collect()
    }

    fn signal_str(signal: &Signal) -> &'static str {
        match signal {
            Signal::Healthy => "healthy",
            Signal::SubstrateUnreachable(_) => "substrate-unreachable",
            Signal::InstanceNotRunning(_) => "instance-not-running",
            Signal::ProbeFailing(_) => "probe-failing",
            Signal::Unknown(_) => "unknown",
            Signal::NotDeployed => "not-deployed",
        }
    }

    fn signal_detail(signal: &Signal) -> String {
        match signal {
            Signal::Healthy | Signal::NotDeployed => String::new(),
            Signal::SubstrateUnreachable(d)
            | Signal::InstanceNotRunning(d)
            | Signal::ProbeFailing(d)
            | Signal::Unknown(d) => d.clone(),
        }
    }

    /// D-A5-21: runs a fresh health sweep inside the RPC rather than
    /// reading rows nothing writes -- A5b's read surface is not idle, it
    /// just isn't on a resident timer yet.
    async fn handle_status(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse status params: {e}")))?;

        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let instance_id = AppInstanceId::try_new(app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let mut expected = Vec::new();
        let mut missing_placement: BTreeSet<String> = BTreeSet::new();
        let mut did_to_alias: BTreeMap<String, String> = BTreeMap::new();
        for svc in &plan.services {
            match deploy::current_placement(&landed, &svc.member_ref().to_string()) {
                None => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                        member_index: svc.member_index,
                    });
                    missing_placement.insert(svc.member_ref().to_string());
                }
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
                        member_index: svc.member_index,
                    });
                    if let Some(alias) = &row.substrate_alias {
                        did_to_alias.insert(row.substrate_did.clone(), alias.clone());
                    }
                }
            }
        }

        // M05A A5c D-A5c-9: one client set for the whole call, shared by
        // the health sweep and the generation read below -- `handle_status`
        // used to connect to every substrate twice. The connected set is
        // the union of every alias the plan declares (needed for the
        // generation read, which must reach a substrate even before
        // anything has landed there) and every alias a landed placement
        // names (needed for the health sweep).
        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(&plan).unwrap_or_default().into_iter().collect();
        let connect_aliases = Self::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, &inventory).await;
        // Review finding A-6: these used to be discarded entirely. An
        // unreachable substrate is already visible another way (the
        // health sweep reports it as a fault for a service placed
        // there), but an alias with no inventory entry or no credential
        // is a configuration problem the health sweep cannot see at
        // all, since it never gets far enough to try connecting.
        for (alias, reason) in &failed {
            tracing::warn!(
                app_instance_id,
                alias,
                reason,
                "failed to connect to a substrate this pass needs"
            );
        }

        let mut targets: BTreeMap<String, HealthTarget> = BTreeMap::new();
        for (did, alias) in &did_to_alias {
            // No inventory entry at all is a caller-side configuration gap,
            // not a live outage -- no target is built, and `poll_once`
            // reports `NoTargetBuilt`/`Unknown` for it, unchanged from
            // before this fix.
            if !inventory.contains_key(alias) {
                continue;
            }
            let query: Arc<dyn StatusQuery> = match clients.get(&SubstrateAlias::new(alias.clone()))
            {
                Some(c) => c.clone() as Arc<dyn StatusQuery>,
                None => Arc::new(UnreachableQuery(format!(
                    "failed to connect to substrate alias '{alias}'"
                ))),
            };
            targets.insert(
                did.clone(),
                HealthTarget {
                    alias: Some(SubstrateAlias::new(alias.clone())),
                    substrate_did: did.clone(),
                    query,
                },
            );
        }

        let report = health::poll_once(&targets, &expected).await;
        // Drops `targets`' `Arc<dyn StatusQuery>` clones so `clients`
        // holds the sole remaining `Arc` to each client, which is what
        // lets `shutdown_clients` reach `Arc::get_mut` below.
        drop(targets);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        // M05A A5c D-A5c-10 (§19.11): a planned service the journal has
        // never recorded landed is a deploy failure the sweep cannot see
        // (it has no `service_id`/`substrate_did` to probe at all,
        // reported as `NotDeployed`, deliberately not a fault, D-A4-19).
        // The supervisor holds the plan, so it knows the difference
        // between "not in the plan" and "in the plan and missing" --
        // reuses `InstanceNotRunning` rather than a fifth `AlertKind`,
        // since the operator reads this as the same problem.
        //
        // Keyed on `NEVER_LANDED_SUBSTRATE_DID`, not the empty string:
        // every planned-but-unlanded service also appears in `report.
        // services` as `Signal::NotDeployed` with `substrate_did == ""`,
        // and `record_report`'s own per-service loop unconditionally
        // *clears* `(instance, logical_ref, "", InstanceNotRunning)` for
        // exactly that case (no active fault to report) -- raising under
        // that same empty-string key would have it cleared on every
        // subsequent call, then re-raised here as a "new" incident every
        // time. A distinct sentinel dodges that loop, but is then itself
        // invisible to `record_report`'s *other* pass -- the "this
        // (logical_ref, substrate_did) pair left the sweep entirely, so
        // clear it" cleanup (A4-03) -- which would otherwise clear this
        // alert every single call, for the identical reason in reverse.
        // `extra_live_pairs` is exactly the exemption that cleanup needs.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        // Same call, same constant, as the resident loop's own (D-A5d-9).
        let mut opened = health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
            now,
            &extra_live_pairs,
            SUPERVISOR_CERT_ALERT_POLICY,
        )
        .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Folded into `opened` (not published separately) so the publish
        // call below sees every alert this pass newly raised, not only
        // the ones `record_report` itself knows about.
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if missing_placement.contains(&l_ref) {
                if self
                    .store
                    .alerts
                    .raise(
                        &instance_id,
                        Some(&l_ref),
                        None,
                        NEVER_LANDED_SUBSTRATE_DID,
                        AlertKind::InstanceNotRunning,
                        "planned but never deployed; the supervisor holds no completed placement \
                         for this service",
                    )
                    .map_err(|e| RpcError::InternalError(e.to_string()))?
                {
                    opened.push((AlertKind::InstanceNotRunning, l_ref));
                }
            } else {
                self.store
                    .alerts
                    .clear(
                        &instance_id,
                        Some(&l_ref),
                        NEVER_LANDED_SUBSTRATE_DID,
                        AlertKind::InstanceNotRunning,
                    )
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
            }
        }

        // D-A5-13/D-A5c-6: publication happens here, in `record_report`'s
        // caller, over the newly-opened list above -- every store write
        // that could add to it has already committed, so a publish
        // failure below can never lose an alert by construction. Never
        // propagated with `?`: an unreachable/slow MQTT broker must not
        // fail the whole `status` call.
        self.publish_opened_alerts(&app_instance_id, &opened).await;

        // ADR-0021 §4 / matrix row 9: a substrate reporting a higher
        // generation than this supervisor holds means a second supervisor
        // has adopted the instance. Checked against every substrate the
        // *plan* places a service on, not `did_to_alias` above (which only
        // covers substrates this supervisor's own journal already shows a
        // landed placement on, and is empty until the first one lands) --
        // see `max_held_generation_from_clients`'s own doc.
        let held_max = Self::max_held_generation_from_clients(
            &app_instance_id,
            &plan_aliases,
            &Self::actors_from_clients(&clients),
        )
        .await;
        let superseded = self
            .update_superseded_alert(&instance_id, &app_instance_id, held_max, state.generation)
            .map_err(RpcError::InternalError)?;

        // D-A5c-9: closed once, at the end, now that both the health
        // sweep and the generation read above are done with them.
        Self::shutdown_clients(clients.into_values()).await;

        let services: Vec<ManagedService> = report
            .services
            .iter()
            .map(|s| ManagedService {
                logical_ref: s.member_ref().to_string(),
                service_id: s.service_id.clone(),
                substrate_alias: s
                    .alias
                    .as_ref()
                    .map(SubstrateAlias::to_string)
                    .unwrap_or_default(),
                substrate_did: s.substrate_did.clone(),
                signal: Self::signal_str(&s.signal).to_string(),
                detail: Self::signal_detail(&s.signal),
                // Review finding A-3: Phase 6's stated deliverable was
                // this field leaving 0 -- the `remediation` table has
                // recorded every attempt since phase 6, this was simply
                // never read back. `Ok(None)` (no restart ever attempted)
                // and a read failure both fall back to 0, which is the
                // correct value for "no attempts recorded", not a
                // reported error.
                restart_attempts: self
                    .store
                    .remediation_state(&app_instance_id, &s.member_ref().to_string())
                    .ok()
                    .flatten()
                    .map_or(0, |r| r.attempts),
            })
            .collect();

        // M05A A5c D-A5c-13 (§19.15): a reconcile in flight is now
        // observable -- `apply_with_clients` writes `Applying` and this is
        // the first caller able to read it mid-pass. Ranked after `paused`
        // (a paused instance's own state matters more than "busy") and
        // before the health-derived branch (a health verdict computed
        // from a half-applied plan is less useful than "ask again").
        let is_applying = self
            .store
            .journal
            .get_latest(&instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .is_some_and(|r| r.state == DeploymentState::Applying);

        // M05A A5e (D-A5e-8, ADR-0021 §5): a binding push that has been
        // attempted and did not land leaves the instance `Degraded` --
        // reachable now that D-A5e-7 gives `push_bindings` a production
        // caller. Read off the *active* alert set rather than "any
        // unconverged row": a push that just landed cleanly reads as
        // unconverged on `binding-epochs` for up to one poll interval
        // simply because the observed epoch has not been re-polled yet,
        // and that must not flap the instance `Degraded` on every
        // ordinary change.
        let has_binding_conflict = self
            .store
            .alerts
            .active(&instance_id)
            .map(|active| active.iter().any(|a| a.kind == AlertKind::BindingConflict))
            .unwrap_or(false);

        let overall_state = if state.retired {
            ManagedState::Retired
        } else if superseded {
            ManagedState::Superseded
        } else if state.paused {
            ManagedState::Paused
        } else if is_applying {
            ManagedState::Applying
        // D-A5c-10 (§19.11): a service the plan names but the journal has
        // never recorded landed is a deploy failure the sweep alone
        // cannot see (D-A4-19's `NotDeployed` is deliberately not a
        // fault) -- the supervisor adds the plan knowledge the poll
        // does not have.
        } else if report.faults().is_empty()
            && missing_placement.is_empty()
            && !has_binding_conflict
        {
            ManagedState::Active
        } else {
            ManagedState::Degraded
        };

        // Computed before `state.app_master_did` is moved into the
        // literal below -- keyed by the app master DID, not the instance
        // id, so a handover's new DID starts at "never refreshed" rather
        // than inheriting the old DID's stamp.
        let app_record_expires_at = (!state.app_master_did.is_empty())
            .then(|| self.store.last_tier1_refresh(&state.app_master_did).unwrap_or(None))
            .flatten()
            .map(|at| (at as u64).saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS));

        let status = InstanceStatus {
            app_instance_id: app_instance_id.clone(),
            state: overall_state,
            generation: state.generation,
            supervisor_did: self.node_did.clone(),
            // A5b ran no reconcile loop, so this used to be permanently
            // `None` (D-A5-21; H1, Slice A5b review, on why it is not
            // `Some(now)` either -- that reported every instance as
            // having just reconciled, even one that never has). A5c's
            // loop now stamps `last_reconciled` at the end of every pass
            // it actually runs for this instance (review finding A-8);
            // `status`'s own on-demand sweep, right here, deliberately
            // does not count as one.
            last_reconciled_at: self.last_reconciled.get(&app_instance_id).map(|v| *v as u64),
            services,
            // M05A A5c §19.4/D-A5c-5 (F7, the exit criterion's own test):
            // read off the store's own written epoch and this pass's
            // observed one, per declared dependency.
            bindings: self.binding_convergence_rows(&app_instance_id, &plan, &report),
            delivery_note: "delivery is best-effort synchronous; a converged status is not a \
                            durability guarantee"
                .to_string(),
            // Reads the same table `apply_with_clients`
            // already consults on every write pass, so a revocation is
            // visible here the moment it lands, not only once some other
            // change triggers a write that reaches the member.
            revoked_placements: self
                .store
                .revoked_placements(&app_instance_id)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            // M05A A7 (D-A7-4/D-A7-6): read from the stored row only, never
            // the vault -- a locked vault is the ordinary state of a
            // freshly-booted supervisor, and this field must stay readable
            // through it. Empty means "never adopted under A7", mapped to
            // `None` here so a caller does not have to know `""` is a
            // sentinel.
            app_master_did: (!state.app_master_did.is_empty()).then_some(state.app_master_did),
            // ADR-0022 §2, D-C-2: derived from the last successful
            // refresh this supervisor stamped, not read back from the
            // registry -- the deadline an operator has before a locked
            // vault (or a pause) costs this instance's cross-app
            // discoverability, made visible here alongside `VaultLocked`.
            app_record_expires_at,
        };
        Ok(NativeResponse { payload: serde_json::to_value(status).unwrap_or(Value::Null) })
    }

    async fn handle_alerts(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, all): (String, bool) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse alerts params: {e}")))?;
        let instance_id = AppInstanceId::try_new(app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let rows = if all {
            self.store.alerts.all(&instance_id)
        } else {
            self.store.alerts.active(&instance_id)
        }
        .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let alerts: Vec<Alert> = rows
            .into_iter()
            .map(|r| Alert {
                logical_ref: r.logical_ref,
                substrate_did: r.substrate_did,
                kind: r.kind.to_string(),
                detail: r.detail,
                first_seen_at: r.first_seen_at,
                last_seen_at: r.last_seen_at,
                cleared_at: r.cleared_at,
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(alerts).unwrap_or(Value::Null) })
    }

    /// Every item belonging to this instance still in the outbox -- pending
    /// or claimed, not yet dead-lettered (M05B B1 review finding 13):
    /// `roymctl supervisor outbox`'s own listing, and what the reference
    /// scenario's own steps 4/5/7 need to assert the item is actually
    /// queued rather than only inferring it from alerts or `is_converged`.
    async fn handle_outbox(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse outbox params: {e}")))?;

        let rows = self.store.queue.all().map_err(|e| RpcError::InternalError(e.to_string()))?;
        let items: Vec<OutboxItem> = rows
            .into_iter()
            .filter_map(|item| {
                let key: QueueKey = item.queue_key.parse().ok()?;
                (key.app_instance_id == app_instance_id).then_some(OutboxItem {
                    id: item.id as u64,
                    logical_ref: key.logical_ref,
                    substrate_did: key.substrate_did,
                    attempts: item.attempts,
                })
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(items).unwrap_or(Value::Null) })
    }

    /// Every dead letter belonging to this instance, oldest first (M05B B1,
    /// D-B1-6) -- `roymctl supervisor dead-letters`'s own listing.
    async fn handle_dead_letters(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse dead-letters params: {e}"))
        })?;

        let rows =
            self.store.queue.dead_letters().map_err(|e| RpcError::InternalError(e.to_string()))?;
        let dead_letters: Vec<DeadLetter> = rows
            .into_iter()
            .filter_map(|d| {
                let key: QueueKey = d.queue_key.parse().ok()?;
                (key.app_instance_id == app_instance_id).then_some(DeadLetter {
                    id: d.id as u64,
                    logical_ref: key.logical_ref,
                    substrate_did: key.substrate_did,
                    attempts: d.attempts,
                    last_error: d.last_error,
                    created_at: d.created_at,
                })
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(dead_letters).unwrap_or(Value::Null) })
    }

    /// Re-enqueues a dead letter -- it never executes inline (D-B1-7).
    /// Refuses a dead letter belonging to a different app instance, rather
    /// than silently replaying it: the caller named one instance, and a
    /// wrong id must not act on someone else's queued work.
    async fn handle_replay(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, dead_letter_id): (String, u64) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse replay params: {e}")))?;
        let instance_id = AppInstanceId::try_new(app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let rows =
            self.store.queue.dead_letters().map_err(|e| RpcError::InternalError(e.to_string()))?;
        // Both branches below are the caller naming a dead letter that is
        // not theirs to replay -- an unknown id and one that belongs to a
        // different instance are the same class of mistake as a malformed
        // parameter, not a server problem, so both answer `InvalidParams`
        // (M05B B1 review finding 12). Neither names which other instance
        // (if any) actually owns the id: that would confirm to a caller
        // that an id exists under someone else's instance, which an
        // instance-scoped admin grant should not leak.
        let Some(row) = rows.iter().find(|d| d.id as u64 == dead_letter_id) else {
            return Err(RpcError::InvalidParams(format!(
                "no dead letter with id {dead_letter_id} for app instance '{app_instance_id}'"
            )));
        };
        let key: QueueKey = row.queue_key.parse().map_err(|e| {
            RpcError::InternalError(format!("dead letter carries an unparseable queue key: {e}"))
        })?;
        if key.app_instance_id != app_instance_id {
            return Err(RpcError::InvalidParams(format!(
                "no dead letter with id {dead_letter_id} for app instance '{app_instance_id}'"
            )));
        }

        self.store
            .queue
            .replay(dead_letter_id as i64, outbox::now_ms())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        self.clear_delivery_exhausted_if_empty(&instance_id, &key);
        Ok(NativeResponse { payload: serde_json::json!({"status": "replayed"}) })
    }

    /// Every schedule this instance declares, in logical-ref order:
    /// `roymctl supervisor schedules`'s own listing.
    /// Left-joins the stored plan's declarations against `schedule_states`
    /// -- a declared schedule with no state row yet (never evaluated)
    /// appears with `evaluated-at: 0`.
    async fn handle_schedules(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse schedules params: {e}"))
        })?;

        let Some(state) =
            self.store.get(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?
        else {
            return Ok(NativeResponse {
                payload: serde_json::to_value(Vec::<ScheduledTask>::new()).unwrap_or(Value::Null),
            });
        };
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let schedule_states = self
            .store
            .schedule_states(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let tasks: Vec<ScheduledTask> = Self::declared_schedules(&plan)
            .into_iter()
            .map(|(logical_ref, sched)| {
                let recorded = schedule_states.get(&logical_ref);
                ScheduledTask {
                    logical_ref: logical_ref.clone(),
                    cron: sched.cron.clone(),
                    interface: sched.interface.to_string(),
                    method: sched.method.clone(),
                    evaluated_at: recorded.map_or(0, |s| s.evaluated_at),
                    last_run_at: recorded.and_then(|s| s.last_run_at),
                    last_member_index: recorded.and_then(|s| s.last_member_index),
                    last_error: recorded.and_then(|s| s.last_error.clone()),
                }
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(tasks).unwrap_or(Value::Null) })
    }

    /// Tier 2 (ADR-0022 §3): signs and serves this app instance's topology
    /// document for one logical service.
    ///
    /// - looked up by the app's **master DID**, not its human name -- Tier 1
    ///   answers with a DID;
    /// - an unknown app and an unauthorized caller are refused identically, so
    ///   a caller with no grant cannot probe for an app's existence;
    /// - a refusal carries no member DIDs at all: the document is built whole
    ///   or not at all, and the authorization check runs before it is built;
    /// - answers for a paused instance, refuses for a retired one -- pause
    ///   stops the resident loop touching an instance, not its members, which
    ///   stay worth routing to;
    /// - signs once per `(service, epoch)` and serves the cached copy
    ///   afterwards, re-signing once less than half its validity remains.
    async fn handle_resolve(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        let (app_did_str, service_name_str): (String, String) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse resolve params: {e}")))?;
        let app_did = AppDid::try_new(&app_did_str)
            .map_err(|e| RpcError::InvalidParams(format!("invalid app DID: {e}")))?;
        let service_name = LogicalServiceName::try_new(&service_name_str)
            .map_err(|e| RpcError::InvalidParams(format!("invalid service name: {e}")))?;

        // Look up first, authorize second, and report both failures the
        // same way: the lookup is a local read that tells the caller
        // nothing, and returning a distinguishable "no such app" would let
        // an ungranted caller enumerate this node's apps.
        let denied = || {
            RpcError::Custom(
                PERMISSION_DENIED_CODE,
                format!(
                    "no app instance '{app_did}' is resolvable by caller {} on this supervisor",
                    caller.caller_did
                ),
                None,
            )
        };
        let mut state = self
            .store
            .instance_by_app_master_did(app_did.as_str())
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(denied)?;

        let mut open_to_all = false;
        if let Ok(plan) = DeploymentPlan::from_json(&state.plan_json)
            && let Ok(name) = topology::resolve_service_name(&plan, &service_name)
        {
            open_to_all = topology::service_topology_visibility(&plan, &name)
                .map(|v| v == TopologyVisibility::Open)
                .unwrap_or(false);
        }

        // `synapp:<app-did>`, not `substrate:<node>/app/<id>`: the
        // latter's `app/` slot already holds a `service_id`, and it dies on
        // the handover ADR-0022 §5 explicitly worried about. A bare
        // `substrate:<node>` `substrate/admin` grant still covers this,
        // because `Capability::grants` short-circuits on
        // `is_substrate_scope`.
        if !open_to_all
            && !caller.has_capability(
                &ResourceUri(format!("synapp:{app_did}")),
                &Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
            )
        {
            return Err(denied());
        }
        // A retired instance answers nothing -- the same denial as an
        // unknown app. `paused` is deliberately NOT checked: pause stops
        // the resident loop touching an instance, not its members, which
        // stay worth routing to.
        if state.retired {
            return Err(denied());
        }

        // The plan read, the epoch, and the signature must describe one
        // plan. This call holds no instance lock, so a `submit` can land
        // between the read and the sign -- retry once on the lock-free
        // insert-only path, then fall back to a locked repair below rather
        // than sign a mismatched pair. `NoSuchService` is caller input (an
        // authorized caller asking for a service this app does not have),
        // unlike `InconsistentPlan`, which is a compiler defect.
        let map_topology_err = |e: TopologyBuildError| match e {
            TopologyBuildError::NoSuchService(_) | TopologyBuildError::AmbiguousHash(_) => {
                RpcError::InvalidParams(e.to_string())
            }
            TopologyBuildError::InconsistentPlan(_) => RpcError::InternalError(e.to_string()),
        };
        // The supplied name is canonicalised (S3, D-S3-3: `resolve` accepts
        // a logical service name *or* its `short_hash`) **inside** the
        // two-attempt loop, since each attempt re-reads `state.plan_json`
        // and a `submit` landing between attempts can change the declared
        // names. `resolved_name` then replaces `service_name` at every
        // later use in this function -- the epoch key, the cache key, and
        // `TopologyDocument.service_name` -- so the document always names
        // the real service name, never the hash a caller sent. That
        // property is what lets the gateway's D-S3-5 check
        // (`short_hash(doc.service_name) == s_hash`) be meaningful rather
        // than tautological.
        let mut topo = None;
        let mut resolved_name = None;
        let mut epoch = 0u64;
        for _attempt in 0..2 {
            let plan = DeploymentPlan::from_json(&state.plan_json)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            let name =
                topology::resolve_service_name(&plan, &service_name).map_err(map_topology_err)?;
            let t = topology::service_topology(&plan, &name).map_err(map_topology_err)?;
            let fp = topology_fingerprint(t.mode, &t.members, t.sharding_strategy.as_ref());
            let (e, stored_fp) = self
                .store
                .initialise_topology_epoch(&state.app_instance_id, name.as_str(), &fp)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            if stored_fp == fp {
                topo = Some(t);
                resolved_name = Some(name);
                epoch = e;
                break;
            }
            // A submit landed under us. Re-read and try once more.
            state = self
                .store
                .instance_by_app_master_did(app_did.as_str())
                .map_err(|e| RpcError::InternalError(e.to_string()))?
                .ok_or_else(denied)?;
        }
        let (resolved_name, topo, epoch) = match topo {
            Some(t) => (resolved_name.unwrap_or_else(|| service_name.clone()), t, epoch),
            None => {
                // Two lock-free attempts still disagreed with the stored
                // fingerprint: either a `submit` is genuinely still in
                // flight, or an earlier `submit`'s fingerprint write never
                // landed (that write is best-effort against a durable
                // write already made, so a failure there only ever
                // surfaces as a `tracing::warn!` in `handle_submit`),
                // leaving a permanently stale row the insert-only form can
                // never correct on its own. The instance lock is an async
                // per-instance mutex never held across the store's own
                // (synchronous, short) critical section, so taking it here
                // can only ever wait behind an in-flight submit/adopt/
                // retire/force-reconcile finishing -- not deadlock one --
                // and the advancing form is safe once this call is the
                // sole writer for the instance.
                let lock = self.instance_lock(&state.app_instance_id);
                let _guard = lock.lock().await;
                state = self
                    .store
                    .instance_by_app_master_did(app_did.as_str())
                    .map_err(|e| RpcError::InternalError(e.to_string()))?
                    .ok_or_else(denied)?;
                // The lock hand-off makes this the expected ordering, not
                // a narrow race: a `retire` holding the same instance lock
                // can finish while this call waits for it, and the
                // pre-lock `retired` check above read a state from before
                // that happened.
                if state.retired {
                    return Err(denied());
                }
                let plan = DeploymentPlan::from_json(&state.plan_json)
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
                let name = topology::resolve_service_name(&plan, &service_name)
                    .map_err(map_topology_err)?;
                let t = topology::service_topology(&plan, &name).map_err(map_topology_err)?;
                let fp = topology_fingerprint(t.mode, &t.members, t.sharding_strategy.as_ref());
                let e = self
                    .store
                    .record_topology_fingerprint(&state.app_instance_id, name.as_str(), &fp)
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
                (name, t, e)
            }
        };
        let service_name = resolved_name;

        // One signature per (service, epoch), re-signed when less than
        // half the document's own validity remains. Keyed only by
        // (app_instance_id, service_name), which a handover can leave
        // pointing at a document signed by a *different* master -- the
        // instance id survives `import-master`/`adopt`, the app DID does
        // not -- so the hit condition, not the key, has to bind to the DID
        // actually being resolved. `generation` is checked for the same
        // reason: `adopt` can advance it with no membership change, so the
        // epoch alone does not prove a cached document's `generation` is
        // still current.
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let cache_key = (state.app_instance_id.clone(), service_name.to_string());
        if let Some(cached) = self.signed_documents.get(&cache_key)
            && cached.signed.document.app_did == app_did
            && cached.signed.document.generation == state.generation
            && cached.epoch == epoch
            && cached.signed.document.not_after.saturating_sub(now)
                > self.topology_document_not_after_secs / 2
        {
            return Ok(NativeResponse {
                payload: serde_json::to_value(&cached.signed).unwrap_or(Value::Null),
            });
        }

        let instance_id = AppInstanceId::try_new(state.app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let master = match keys::existing_app_master(&self.vault, &state.app_instance_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                return Err(RpcError::InternalError(format!(
                    "app instance '{}' has no app master; run `adopt`",
                    state.app_instance_id
                )));
            }
            Err(keys::VaultError::Locked) => {
                let _ = self.store.alerts.raise(
                    &instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::VaultLocked,
                    &format!(
                        "'{}' cannot sign a Tier-2 topology document because this supervisor's \
                         vault is locked. Run: roymctl --substrate {} security inject-kek \
                         --kek-hex <...>",
                        state.app_instance_id, self.node_did
                    ),
                );
                return Err(RpcError::InternalError(format!(
                    "this supervisor's vault is locked; run `inject-kek` before {} can be resolved",
                    state.app_instance_id
                )));
            }
            Err(e) => return Err(RpcError::InternalError(e.to_string())),
        };
        let actual_did = syneroym_identity::substrate::derive_did_key(&master.public_key());
        if actual_did != app_did.as_str() {
            let _ = self.store.alerts.raise(
                &instance_id,
                None,
                None,
                &self.node_did,
                AlertKind::AppIdentityMismatch,
                &format!(
                    "this instance's row records app master {app_did}, but the vault's app-<id> \
                     key derives {actual_did} -- run `import-master` for the correct key, then \
                     `adopt`, before this app instance can be resolved"
                ),
            );
            return Err(RpcError::InternalError(
                "this instance's vault key does not match its recorded app master DID".to_string(),
            ));
        }
        let _ = self.store.alerts.clear(&instance_id, None, &self.node_did, AlertKind::VaultLocked);
        let _ = self.store.alerts.clear(
            &instance_id,
            None,
            &self.node_did,
            AlertKind::AppIdentityMismatch,
        );

        let document = TopologyDocument {
            app_instance_id: instance_id,
            app_did: app_did.clone(),
            service_name,
            mode: topo.mode,
            members: topo.members,
            sharding_strategy: topo.sharding_strategy,
            epoch: TopologyEpoch(epoch),
            generation: state.generation,
            issued_at: now,
            not_after: now.saturating_add(self.topology_document_not_after_secs),
            cache_ttl_ms: self.topology_document_cache_ttl_secs.saturating_mul(1_000),
        };
        let signed = document.sign(&master).map_err(|e| RpcError::InternalError(e.to_string()))?;
        self.signed_documents.insert(cache_key, CachedDocument { signed: signed.clone(), epoch });

        Ok(NativeResponse { payload: serde_json::to_value(&signed).unwrap_or(Value::Null) })
    }
}

/// A `StatusQuery` that always fails, for a substrate this supervisor
/// could not connect to -- mirrors `roymctl app health`'s
/// `UnreachableTarget`, letting `poll_once` report `SubstrateUnreachable`
/// through its normal error path instead of a special case.
#[derive(Debug)]
struct UnreachableQuery(String);

#[async_trait::async_trait]
impl StatusQuery for UnreachableQuery {
    async fn status(
        &self,
        _service_ids: Vec<String>,
    ) -> Result<syneroym_sdk::SubstrateStatus, String> {
        Err(self.0.clone())
    }
}

#[async_trait::async_trait]
impl NativeService for SupervisorService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        if invocation.interface.as_str() != SUPERVISOR_INTERFACE {
            return Err(RpcError::InternalError(format!(
                "interface {} not handled by the supervisor service",
                invocation.interface
            )));
        }
        match invocation.method.as_str() {
            "submit" => self.handle_submit(&invocation.caller, invocation.params).await,
            "adopt" => self.handle_adopt(&invocation.caller, invocation.params).await,
            "release" => self.handle_release(&invocation.caller, invocation.params).await,
            "pause" => self.handle_pause(&invocation.caller, invocation.params).await,
            "resume" => self.handle_resume(&invocation.caller, invocation.params).await,
            "retire" => self.handle_retire(&invocation.caller, invocation.params).await,
            "force-reconcile" => {
                self.handle_force_reconcile(&invocation.caller, invocation.params).await
            }
            "export-master" => {
                self.handle_export_master(&invocation.caller, invocation.params).await
            }
            "import-master" => {
                self.handle_import_master(&invocation.caller, invocation.params).await
            }
            "revoke-instance" => {
                self.handle_revoke_instance(&invocation.caller, invocation.params).await
            }
            "status" => self.handle_status(&invocation.caller, invocation.params).await,
            "alerts" => self.handle_alerts(&invocation.caller, invocation.params).await,
            "outbox" => self.handle_outbox(&invocation.caller, invocation.params).await,
            "dead-letters" => self.handle_dead_letters(&invocation.caller, invocation.params).await,
            "replay" => self.handle_replay(&invocation.caller, invocation.params).await,
            "schedules" => self.handle_schedules(&invocation.caller, invocation.params).await,
            "resolve" => self.handle_resolve(&invocation.caller, invocation.params).await,
            method => Err(RpcError::MethodNotFound(method.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        future,
        path::{Path, PathBuf},
        sync::Mutex,
        time::Instant,
    };

    use syneroym_app_orchestration::{
        ActionState, DEFAULT_SCHEDULE_TIMEOUT_MS, DeploymentJournal,
        models::{
            AppBlueprintId, InterfaceName, LogicalServiceRef, ServiceConfig, ServiceType,
            TopologyMode,
        },
    };
    use syneroym_async_queue::{Queue, QueueConfig};
    use syneroym_core::{config::SupervisorRole, dht_registry::SignedEndpointInfo, util};
    use syneroym_identity::{DelegationCertificate, substrate};
    use syneroym_rpc::AuthLevel;

    use super::*;
    use crate::tier1::RegistryTier1Writer;

    fn test_broker() -> Arc<MqttBroker> {
        Arc::new(MqttBroker::new(syneroym_mqtt_broker::MqttBrokerConfig::default()).unwrap())
    }

    /// What a fixture varies about the supervisor under test. Everything
    /// else -- node DID, broker, alert topic, intervals -- is fixed, since
    /// no test has a reason to change it.
    #[derive(Default)]
    struct Fixture {
        /// Encryption on with no KEK injected, so the vault genuinely
        /// refuses reads. §0.31's whole point is that a
        /// disabled-encryption fixture proves nothing about the locked
        /// case.
        locked_vault: bool,
        /// Injects a KEK even when `locked_vault` turned encryption on --
        /// an encrypted vault that is currently *open*. Only the vault-race
        /// test needs this: it then clears the KEK to reach the state
        /// `kek_is_loaded()` cannot describe, where the check has already
        /// passed and the read that follows fails locked.
        inject_kek_anyway: bool,
        /// Skips the KEK injection this builder otherwise gives an
        /// unencrypted (`locked_vault: false`) fixture by default -- the
        /// one way to reach `storage.encryption = false` with no KEK
        /// ever injected, which `kek_is_loaded()` (a `KeyStore`-only
        /// check) cannot distinguish from a genuinely locked, encrypted
        /// vault. Only a test proving a caller reads the vault by
        /// attempting it rather than pre-checking `kek_is_loaded()` needs
        /// this -- on this fixture, every vault read still succeeds.
        skip_kek_injection: bool,
        /// `None` leaves the default (5).
        max_renewals_per_pass: Option<u32>,
        anchor_writer: Option<Arc<dyn AnchorWriter>>,
        tier1_writer: Option<Arc<dyn Tier1Writer>>,
        master_anchor_refresh_interval_secs: Option<u64>,
        /// `None` leaves the default -- a private `dir.path().join(
        /// "backups")` on the `TempDir` the built service now keeps alive
        /// on its own `_fixture_tempdir` field, for its own lifetime
        /// (M05A A7 review finding 4 -- an earlier version of this
        /// comment described a `TempDir` the builder dropped before
        /// returning, which was true before that fix). M05A A7's handover
        /// test needs two fixture-built services to share one backup
        /// directory (a stand-in for two supervisors handed the same
        /// operator-carried file), which the default cannot do -- so the
        /// test owns and passes one in, held for the whole test (§0.5).
        backup_dir: Option<PathBuf>,
        /// `None` leaves the default (5s). Tests driving the queue worker
        /// against a fake clock set this explicitly (M05B B1).
        queue_tick_secs: Option<u64>,
        /// `None` leaves the default (30s). Tests proving recovery happens
        /// within one worker tick rather than one poll interval set this
        /// explicitly, far above the tick (M05B B1).
        poll_interval_secs: Option<u64>,
        /// `None` leaves the default (3600s). Tests proving a document
        /// re-signs once less than half its validity remains set this low.
        topology_document_not_after_secs: Option<u64>,
        /// `None` leaves the default (300s).
        topology_document_cache_ttl_secs: Option<u64>,
    }

    impl Fixture {
        fn build(self) -> SupervisorService {
            self.build_with_key_store().0
        }

        /// Hands back the `KeyStore` alongside the service, so a test can
        /// change the vault's locked state *after* construction -- the only
        /// way to reach the race D-A5d-17 carves out, where
        /// `kek_is_loaded()` answers "unlocked" and the vault read that
        /// follows still fails.
        fn build_with_key_store(
            self,
        ) -> (SupervisorService, Arc<syneroym_data_keystore::KeyStore>) {
            // Kept alive on the returned `SupervisorService` itself
            // (`_fixture_tempdir`), not left to drop here: a dropped
            // `TempDir` deletes the directory tree from disk the instant
            // this function returns, while `SqliteStorageProvider` already
            // holds an open connection into it (M05A A7, found while
            // adding the first fixture-built test that performs a real
            // *encrypted* write -- every earlier fixture's writes went
            // through `open_service_db`'s own on-demand directory
            // recreation and never touched the provider's `substrate_conn`,
            // so this never surfaced before). An *unencrypted* mint
            // recreates its directory on demand and keeps working
            // regardless -- the DEK path does not, since it writes through
            // the provider's own top-level connection, opened once at
            // construction against a file an early drop would have already
            // unlinked, and a `-journal` file cannot be created in a
            // directory that no longer exists. An earlier fix instead
            // called `.keep()` on the `TempDir`, which stopped it from
            // dropping *ever* -- fixing the encrypted path at the cost of
            // leaking every fixture-built test's directory permanently
            // (M05A A7 review finding 4). Tying its lifetime to the
            // service's own restores ordinary cleanup on every ordinary
            // test's `Drop`, ~150 of them, while keeping the fix for the
            // handful that mint under encryption.
            let dir = tempfile::tempdir().unwrap();
            let store = SupervisorStore::open_in_memory().unwrap();
            let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
                syneroym_data_db::SqliteStorageProvider::new(
                    dir.path().join("db"),
                    self.locked_vault,
                )
                .unwrap(),
            );
            let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
            // An unlocked fixture must actually report its KEK as loaded:
            // `kek_is_loaded` is what gates the renewal work-list, and it
            // reads the `KeyStore`, not the storage provider's encryption
            // flag. `skip_kek_injection` is the one deliberate exception --
            // an unencrypted vault whose `KeyStore` never sees a KEK,
            // needed to tell a real read attempt apart from a
            // `kek_is_loaded()` pre-check.
            if (!self.locked_vault || self.inject_kek_anyway) && !self.skip_kek_injection {
                key_store.inject_kek([7u8; 32]).unwrap();
            }
            let vault = MasterVault::new(
                storage_provider,
                key_store.clone(),
                "supervisor".to_string(),
                self.backup_dir.clone().unwrap_or_else(|| dir.path().join("backups")),
            );
            let identity = Identity::generate().unwrap();
            let mut service = SupervisorService::new(
                "did:key:zSupervisorNode".to_string(),
                store,
                vault,
                &identity,
                false,
                test_broker(),
                "supervisor/alerts".to_string(),
                self.poll_interval_secs.unwrap_or(30),
                3,
                30,
                4,
                self.max_renewals_per_pass.unwrap_or(5),
                self.master_anchor_refresh_interval_secs.unwrap_or(12 * 3600),
                self.anchor_writer,
                self.tier1_writer,
                self.queue_tick_secs.unwrap_or(5),
                self.topology_document_not_after_secs.unwrap_or(3600),
                self.topology_document_cache_ttl_secs.unwrap_or(300),
            );
            service._fixture_tempdir = Some(dir);
            (service, key_store)
        }
    }

    fn service() -> SupervisorService {
        Fixture::default().build()
    }

    fn service_with_locked_vault() -> SupervisorService {
        Fixture { locked_vault: true, ..Fixture::default() }.build()
    }

    fn unauthenticated_caller() -> CallerContext {
        CallerContext {
            caller_did: "did:key:zRandom".to_string(),
            app_instance: None,
            session: Default::default(),
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    fn admin_caller(node_did: &str) -> CallerContext {
        use syneroym_rpc::{Capability, SessionContext};
        CallerContext {
            caller_did: "did:key:zAdmin".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zAdmin".to_string(),
                capabilities: vec![Capability {
                    with: ResourceUri::substrate(node_did),
                    can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        }
    }

    async fn dispatch(
        service: &SupervisorService,
        caller: CallerContext,
        method: &str,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        service
            .dispatch(NativeInvocation {
                interface: SUPERVISOR_INTERFACE.to_string(),
                method: method.to_string(),
                params,
                caller,
            })
            .await
    }

    #[tokio::test]
    async fn every_verb_is_refused_without_substrate_admin() {
        let s = service();
        for (method, params) in [
            (
                "submit",
                serde_json::json!([{"app_instance_id": "i", "plan_json": "{}", "inventory_json": "{}", "generation": 0}]),
            ),
            ("adopt", serde_json::json!(["i"])),
            ("release", serde_json::json!(["i"])),
            ("pause", serde_json::json!(["i"])),
            ("resume", serde_json::json!(["i"])),
            ("retire", serde_json::json!(["i"])),
            ("force-reconcile", serde_json::json!(["i"])),
            ("export-master", serde_json::json!(["m"])),
            ("import-master", serde_json::json!(["m"])),
            ("status", serde_json::json!(["i"])),
            ("alerts", serde_json::json!(["i", false])),
            // M05B B1, D-B1-6/D-B1-7, test 32: no new resource namespace --
            // gated exactly like the neighbouring verbs above.
            ("outbox", serde_json::json!(["i"])),
            ("dead-letters", serde_json::json!(["i"])),
            ("replay", serde_json::json!(["i", 1])),
            // No new resource namespace here either.
            ("schedules", serde_json::json!(["i"])),
            // `resolve` checks `synapp:<app-did>`, not `substrate:<node>`,
            // but a caller with no capabilities at all still denies on
            // either resource -- a syntactically valid DID is needed so
            // the call reaches the capability check rather than failing at
            // `InvalidParams` first.
            ("resolve", serde_json::json!(["did:key:zX", "backend"])),
        ] {
            let err = dispatch(&s, unauthenticated_caller(), method, params).await.unwrap_err();
            assert_eq!(err.code(), PERMISSION_DENIED_CODE, "{method} must deny without admin");
        }
    }

    /// S4 (Slice A5b review): the plan's own `app_instance_id` and the
    /// submission's outer `app_instance_id` are both caller-supplied and,
    /// before this check, never compared. A mismatch would key the journal
    /// and vault under one instance while `status`/`adopt`/`retire` key on
    /// the other, splitting the instance in two.
    #[tokio::test]
    async fn submit_is_refused_when_the_outer_instance_id_does_not_match_the_plans_own() {
        let s = service();
        let plan_json = plan_json_no_services("plan-says-inst-1");

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "outer-says-inst-2",
                "plan_json": plan_json,
                "inventory_json": "{}",
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("plan-says-inst-1") && err.contains("outer-says-inst-2"), "{err}");
        assert!(s.store.get("outer-says-inst-2").unwrap().is_none());
        assert!(s.store.get("plan-says-inst-1").unwrap().is_none());
    }

    /// Test 20: `submit` is the backstop entry point for `D-B2-14`'s
    /// contradiction check -- a plan reaching the supervisor was never
    /// necessarily compiled through `compile()` (a hand-built plan, or a
    /// client other than `roymctl`), so the same refusal must fire here
    /// too, not only inside the compiler.
    #[tokio::test]
    async fn submit_is_refused_when_the_plan_declares_open_topology_over_a_private_service() {
        let s = service();
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-contradiction",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hFabricated",
                "logical_ref": "inst-contradiction/backend",
                "substrate": null,
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton",
                "visibility": "private",
                "topology_visibility": "open",
            }]
        })
        .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-contradiction",
                "plan_json": plan_json,
                "inventory_json": "{}",
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RpcError::InvalidParams(_)), "expected InvalidParams, got {err:?}");
        let err = err.to_string();
        assert!(err.contains("open") && err.contains("private"), "{err}");
        assert!(s.store.get("inst-contradiction").unwrap().is_none());
    }

    #[tokio::test]
    async fn submit_is_refused_when_a_placed_alias_carries_no_credential() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json =
            serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
                .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": inventory_json,
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no credential"), "{err}");
    }

    /// B3 (Slice A5b review): `deploy_submission` used to run the whole
    /// mint/certify/apply pipeline *before* `store.submit`'s retired guard
    /// ever ran, so a submit against a retired instance redeployed every
    /// service and only then reported the rejection. The inventory here
    /// carries no credential for the placed alias -- exactly
    /// `submit_is_refused_when_a_placed_alias_carries_no_credential`'s
    /// fixture -- so if the retired check did not run first, this would
    /// fail with "no credential" instead, proving the ordering rather than
    /// merely the outcome.
    #[tokio::test]
    async fn submit_against_a_retired_instance_is_refused_before_any_deploy_work_runs() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json =
            serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
                .to_string();
        s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
        s.store.retire("inst-1").unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": inventory_json,
                "generation": 1,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("retired"), "{err}");
        assert!(!err.to_string().contains("credential"), "{err}");
    }

    /// N1 (Slice A5b review round 2): H3's generation check lived only at
    /// `store.submit`, which still ran *after* `deploy_submission` --
    /// including after that pipeline presented `s.generation` to the
    /// substrate's own `check_generation`, which *accepts* a higher
    /// generation and advances its stamp. So a wrong upward
    /// `--generation` would have left the substrate ahead of this
    /// supervisor's own store the moment the store then refused to
    /// record it. The inventory here carries no credential, exactly
    /// `submit_against_a_retired_instance_is_refused_before_any_deploy_
    /// work_runs`'s fixture: a "generation" failure rather than a
    /// "credential" one proves the check ran before any deploy work, not
    /// merely that the submit failed for some other reason.
    #[tokio::test]
    async fn submit_at_the_wrong_generation_is_refused_before_any_deploy_work_runs() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json =
            serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
                .to_string();
        s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
        s.store.set_generation("inst-1", 3).unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": inventory_json,
                "generation": 5,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("generation"), "{err}");
        assert!(!err.to_string().contains("credential"), "{err}");
        // Store state must be untouched by the rejected attempt.
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().generation, 3);
    }

    /// Same defect, `force-reconcile`'s side: it never calls `store.submit`
    /// at all, so nothing on it refused a retired instance -- it would
    /// just redeploy indefinitely.
    #[tokio::test]
    async fn force_reconcile_against_a_retired_instance_is_refused() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json =
            serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
                .to_string();
        s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
        s.store.retire("inst-1").unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "force-reconcile",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("retired"), "{err}");
        assert!(!err.to_string().contains("credential"), "{err}");
    }

    /// S7 (Slice A5b review): before `connect_best_effort`,
    /// `release_on_every_substrate` used `build_clients`, whose contract
    /// fails the whole call the moment one placed alias cannot be
    /// reached -- so retiring an instance placed on even one unreachable
    /// substrate was permanently impossible. `ucan: null` fails fast at
    /// the credential check rather than waiting out a real connect
    /// timeout; either way is "cannot reach it" for this purpose.
    #[tokio::test]
    async fn retire_succeeds_and_marks_the_store_retired_even_when_a_placed_substrate_is_unreachable()
     {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json = serde_json::json!({
            "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
        })
        .to_string();
        s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "retire",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(res.payload.get("status").and_then(|v| v.as_str()), Some("retired"));
        let unreleased =
            res.payload.get("unreleased_substrates").and_then(|v| v.as_array()).unwrap();
        assert_eq!(unreleased.len(), 1, "{unreleased:?}");

        assert!(
            s.store.get("inst-1").unwrap().unwrap().retired,
            "the local store must still mark the instance retired"
        );
    }

    #[tokio::test]
    async fn submit_against_a_locked_vault_names_inject_kek() {
        let s = service_with_locked_vault();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json = serde_json::json!({
            "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
        })
        .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": inventory_json,
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("inject-kek"), "{err}");
    }

    #[tokio::test]
    async fn status_reports_the_delivery_note_rather_than_implying_convergence() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        assert!(status.delivery_note.contains("best-effort"));
        assert!(status.bindings.is_empty());
    }

    #[tokio::test]
    async fn status_polls_on_demand_so_its_signals_are_not_empty() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        // No completed placement was ever journaled, so the sweep reports
        // exactly one `not-deployed` signal rather than an empty list --
        // it really ran, not merely echoed stored rows.
        assert_eq!(status.services.len(), 1);
        assert_eq!(status.services[0].signal, "not-deployed");
    }

    /// A revocation with nothing else changed used to be
    /// invisible on `status` until some unrelated write reached the
    /// member and raised `InstanceRevoked` inside `apply_with_clients`.
    /// `revoked_placements` is a local table read, so it belongs on the
    /// read surface directly, not gated behind a write pass ever
    /// happening to touch this member again.
    #[tokio::test]
    async fn status_reports_a_revoked_placement_with_nothing_else_changed() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

        assert_eq!(status.revoked_placements, vec!["inst-1/backend#0".to_string()]);
    }

    /// M05A A5e §33.22/test 82: the journal keys every completed action row
    /// on a `MemberRef`, not a bare `LogicalServiceRef` -- if `handle_status`'s
    /// own expected-service builder (one of three, alongside the loop's
    /// sweep and `roymctl`'s two) ever went back to reading it by the old
    /// key, member 1's placement would silently stop matching and this
    /// service would report `substrate_did` empty and land in
    /// `missing_placement` even though it is fully landed. Scaled (index 1,
    /// not 0) on purpose: an unscaled member's `MemberRef` string is
    /// unchanged from before A5e and would not catch a regression to the
    /// old key.
    #[tokio::test]
    async fn a_members_placement_is_found_after_the_journal_is_re_keyed() {
        let s = service();
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hFabricated",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "redundant",
                "member_index": 1
            }]
        })
        .to_string();
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#1",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

        assert_eq!(status.services.len(), 1, "{:?}", status.services);
        assert_eq!(status.services[0].logical_ref, "inst-1/backend#1");
        assert_eq!(
            status.services[0].substrate_did, "did:key:zEdge1",
            "member 1's completed placement must be found by its own MemberRef, not read as \
             missing: {:?}",
            status.services[0]
        );
        assert_ne!(
            status.services[0].signal, "instance-not-running",
            "a landed member must not be reported as never-deployed: {:?}",
            status.services[0]
        );
    }

    /// M05A A5c D-A5c-1 (§19.1, matrix row 20's blast-radius neighbor): a
    /// re-submit that moves a landed service to a different substrate must
    /// be refused before anything is deployed -- A5b shipped `submit` with
    /// no such check, so this silently ran a second live copy of the same
    /// member.
    #[tokio::test]
    async fn submit_is_refused_when_the_plan_moves_a_service_to_another_substrate() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
        let inventory_json =
            serde_json::json!({"edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1"}})
                .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": moved_plan_json,
                "inventory_json": inventory_json,
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
    }

    /// Same fixture trick as `submit_against_a_retired_instance_is_refused_
    /// before_any_deploy_work_runs`: the inventory carries no credential
    /// for the new alias, so a "placement" failure (not a "credential"
    /// one) proves the refusal runs before `deploy_submission`.
    #[tokio::test]
    async fn submit_with_a_changed_placement_is_refused_before_any_deploy_work_runs() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
        // No credential for edge-2: if the placement refusal did not run
        // first, this would fail with "no credential" instead.
        let inventory_json = serde_json::json!({
            "edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1", "ucan": null}
        })
        .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": moved_plan_json,
                "inventory_json": inventory_json,
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
        assert!(!err.contains("credential"), "{err}");
    }

    /// N3's lesson applied here too: `force-reconcile` never calls
    /// `store.submit`, so it needs its own placement check -- without one
    /// it would just keep redeploying the moved service indefinitely.
    #[tokio::test]
    async fn force_reconcile_is_refused_when_the_stored_plan_moves_a_service() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
        let inventory_json =
            serde_json::json!({"edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1"}})
                .to_string();
        s.store.submit("inst-1", &moved_plan_json, &inventory_json, "did:key:owner", 0).unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "force-reconcile",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
    }

    /// The boundary: a re-submit that keeps the same substrate must not
    /// be caught by the placement refusal -- otherwise it refuses
    /// everything, not just a real move.
    #[tokio::test]
    async fn submit_is_allowed_when_a_service_keeps_its_substrate() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        // Same alias, no credential -- so a run past the placement check
        // must fail later, at the credential gate, not be refused for
        // "placement".
        let inventory_json = serde_json::json!({
            "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
        })
        .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": inventory_json,
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no credential"), "{}", err);
    }

    /// M05A A5c D-A5c-10 (§19.11, matrix row 12): a planned service the
    /// journal has never recorded landed must report the instance
    /// `Degraded`, not `Active` -- today's (A5b) gap, since
    /// `Signal::NotDeployed` is deliberately not a fault (D-A4-19).
    #[tokio::test]
    async fn an_instance_with_a_planned_service_that_never_landed_reports_degraded() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        assert!(matches!(status.state, ManagedState::Degraded), "{:?}", status.state);
    }

    /// Row 12's boundary: an instance whose only service is fully landed
    /// and healthy must still report `Active` -- the fix above must not
    /// degrade every instance.
    #[tokio::test]
    async fn a_fully_landed_healthy_instance_still_reports_active() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();
        // No inventory entry for edge-1: `poll_once` then reports
        // `Unknown` (no health target built), not a fault, so the
        // instance still reads `Active` -- exactly today's (A5b) behavior
        // for an unreachable target, unaffected by this fix.
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        assert!(matches!(status.state, ManagedState::Active), "{:?}", status.state);
    }

    /// D-A5e-8, ADR-0021 §5: the same fully-landed, otherwise-healthy
    /// instance above must report `Degraded`, not `Active`, once one of
    /// its dependents has an active `BindingConflict` -- a binding push
    /// that has been attempted and did not land.
    #[tokio::test]
    async fn a_fully_landed_instance_with_an_active_binding_conflict_reports_degraded() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        s.store
            .alerts
            .raise(
                &AppInstanceId::new("inst-1"),
                Some("inst-1/backend#0"),
                None,
                "did:key:zEdge1",
                AlertKind::BindingConflict,
                "a binding push did not land cleanly after one retry",
            )
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        assert!(matches!(status.state, ManagedState::Degraded), "{:?}", status.state);
    }

    /// M05A A5c D-A5c-13 (§19.15): a reconcile in flight is now
    /// observable -- `apply_with_clients` writes `Applying` before it
    /// writes `Active`/`Degraded`, and `status` landing mid-pass must
    /// read it rather than guessing from a half-applied plan's health.
    #[tokio::test]
    async fn status_reports_applying_while_a_reconcile_is_in_flight() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.journal.append(&plan, DeploymentState::Applying).unwrap();
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
        assert!(matches!(status.state, ManagedState::Applying), "{:?}", status.state);
    }

    /// M05A A5c D-A5c-9 (§19.9): the whole point of the fix is that an
    /// alias serving double duty -- both a landed placement's alias and
    /// the plan's own declared placement -- is connected to once, not
    /// twice. Tested at the dedup itself, which is directly and
    /// deterministically testable with no live substrate; the RPC-level
    /// behavior (one client set shared by the sweep and the generation
    /// read) has no network-free way to observe a connection count
    /// through the public `status` call, since `SyneroymClient` connects
    /// for real rather than through an injectable fake.
    #[test]
    fn status_connects_to_each_substrate_once() {
        let plan_aliases: BTreeSet<String> = ["edge-1".to_string()].into_iter().collect();
        let did_to_alias: BTreeMap<String, String> =
            BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())]);

        let aliases = SupervisorService::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        assert_eq!(aliases, vec!["edge-1".to_string()]);
    }

    /// Review finding A-1: the whole fix in one assertion. `svc-a` is
    /// outside `needs_work` (already landed, unchanged) and `svc-b` is
    /// inside it and reachable this pass -- both must survive into the
    /// record. Before this fix, `record_plan_for_pass` did not exist and
    /// the filtered (`needs_work`-only) plan was journaled directly,
    /// which is what `svc-a` dropping out of this assertion would
    /// reproduce.
    #[test]
    fn record_plan_for_pass_keeps_untouched_services_alongside_this_passs_subset() {
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [
                {
                    "service_id": "did:key:hSvcA",
                    "logical_ref": "inst-1/svc-a",
                    "substrate": "edge-1",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "singleton"
                },
                {
                    "service_id": "did:key:hSvcB",
                    "logical_ref": "inst-1/svc-b",
                    "substrate": "edge-2",
                    "service_type": "tcp", "source": "127.0.0.1:9001",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "singleton"
                }
            ]
        })
        .to_string();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let needs_work: BTreeSet<String> = ["inst-1/svc-b#0".to_string()].into_iter().collect();
        let identity = Identity::generate().unwrap();
        let client = Arc::new(SyneroymClient::new_with_identity(
            "did:key:zEdge2".to_string(),
            String::new(),
            identity,
        ));
        let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
            BTreeMap::from([(SubstrateAlias::new("edge-2"), client)]);

        let record_plan = SupervisorService::record_plan_for_pass(&plan, &needs_work, &clients);

        let refs: BTreeSet<String> =
            record_plan.services.iter().map(|s| s.logical_ref.to_string()).collect();
        assert_eq!(
            refs,
            BTreeSet::from(["inst-1/svc-a".to_string(), "inst-1/svc-b".to_string()]),
            "svc-a (untouched) and svc-b (this pass's subset) must both survive"
        );
    }

    /// The other half: a `needs_work` service whose substrate this pass
    /// never reached has not landed and must not be recorded as if it
    /// had -- recording it would make a later pass believe it is already
    /// active and never retry it.
    #[test]
    fn record_plan_for_pass_drops_a_needs_work_service_still_unreachable_this_pass() {
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hSvcB",
                "logical_ref": "inst-1/svc-b",
                "substrate": "edge-2",
                "service_type": "tcp", "source": "127.0.0.1:9001",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            }]
        })
        .to_string();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let needs_work: BTreeSet<String> = ["inst-1/svc-b#0".to_string()].into_iter().collect();

        let record_plan =
            SupervisorService::record_plan_for_pass(&plan, &needs_work, &BTreeMap::new());

        assert!(record_plan.services.is_empty(), "an unreachable needs_work service must not land");
    }

    // ── M05A A5c: MQTT alert publication (D-A5-13/D-A5c-6) ──────────────

    fn expected_alert_topic(app_instance_id: &str) -> String {
        namespace_topic_for_publish(
            SUPERVISOR_RESERVED_SERVICE_ID,
            &format!("supervisor/alerts/{app_instance_id}"),
        )
    }

    /// D-A5-13/D-A5c-6: a sweep that opens a new alert publishes it under
    /// the supervisor's own topic -- `<alert_topic>/<app_instance_id>`,
    /// namespaced with the publish-side rule under
    /// `SUPERVISOR_RESERVED_SERVICE_ID`, the exact string the router's own
    /// subscribe-side fix (`dispatch.rs::subscribe_namespaced_topic`)
    /// produces for the same service id.
    #[tokio::test]
    async fn a_newly_opened_alert_is_published_under_the_supervisors_own_topic() {
        let s = service();
        // No substrate placement: the sweep reports `not-deployed`, which
        // D-A5c-10 turns into a raised `InstanceNotRunning` alert -- the
        // cheapest fixture that opens a real alert with no live substrate.
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let topic = expected_alert_topic("inst-1");
        let (_handle, mut receiver) = s.messaging_broker.subscribe(topic.clone()).await.unwrap();

        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        let (received_topic, payload) =
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .expect("did not time out waiting for the published alert")
                .expect("broker channel closed");
        assert_eq!(received_topic, topic);
        let value: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["app_instance_id"], "inst-1");
        assert_eq!(value["kind"], AlertKind::InstanceNotRunning.to_string());
    }

    /// §19.5f: `publish_opened_alerts` returns `()`, not a `Result` --
    /// there is no `?` for a publish failure to propagate through, by
    /// construction. This is the observable half of that guarantee: the
    /// `status` call succeeds and the alert is stored and readable
    /// through `alerts`, regardless of what publication itself did.
    #[tokio::test]
    async fn a_publish_failure_leaves_the_alert_stored_and_the_pass_running() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await;
        assert!(res.is_ok(), "the status call must succeed even if MQTT publication does not");

        let instance_id = AppInstanceId::new("inst-1");
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].kind, AlertKind::InstanceNotRunning);
    }

    /// The property `record_report`'s newly-opened return value provides:
    /// an alert already active before this sweep is not published again,
    /// so an operator subscribed to the topic sees one message per
    /// incident, not one per poll.
    #[tokio::test]
    async fn an_already_open_alert_is_not_republished_on_the_next_sweep() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let topic = expected_alert_topic("inst-1");
        let (_handle, mut receiver) = s.messaging_broker.subscribe(topic).await.unwrap();

        for _ in 0..2 {
            dispatch(
                &s,
                admin_caller("did:key:zSupervisorNode"),
                "status",
                serde_json::json!(["inst-1"]),
            )
            .await
            .unwrap();
        }

        // Exactly one message must have arrived, from the first sweep.
        let _first = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("did not time out waiting for the first publish")
            .expect("broker channel closed");
        let second = tokio::time::timeout(Duration::from_millis(300), receiver.recv()).await;
        assert!(second.is_err(), "the second sweep must not republish the still-open alert");
    }

    fn plan_json_no_services(instance: &str) -> String {
        serde_json::json!({
            "app_instance_id": instance,
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": []
        })
        .to_string()
    }

    /// `logical_ref` serializes as `"<instance>/<service>"`, not a nested
    /// object (`LogicalServiceRef`'s `#[serde(try_from = "String")]`), and
    /// `ServiceConfig` is `#[serde(flatten)]`ed onto `PlannedService`, not
    /// nested under a `config` key.
    fn plan_json_one_service(
        instance: &str,
        service_name: &str,
        substrate: Option<&str>,
    ) -> String {
        serde_json::json!({
            "app_instance_id": instance,
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hFabricated",
                "logical_ref": format!("{instance}/{service_name}"),
                "substrate": substrate,
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            }]
        })
        .to_string()
    }

    fn supervisor_interface() -> (wit_parser::Resolve, wit_parser::InterfaceId) {
        let wit_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wit_interfaces/wit/supervisor/supervisor.wit");
        let mut resolve = wit_parser::Resolve::default();
        let content = std::fs::read_to_string(&wit_path).expect("failed to read supervisor.wit");
        let group = wit_parser::UnresolvedPackageGroup::parse(&wit_path, &content)
            .expect("failed to parse supervisor.wit");
        let pkg_id = resolve.push(group.main, 0).expect("failed to resolve supervisor package");
        let package = &resolve.packages[pkg_id];
        let iface_id = package.interfaces["supervisor"];
        (resolve, iface_id)
    }

    #[tokio::test]
    async fn the_supervisor_wit_dispatch_table_covers_every_declared_function() {
        let (resolve, iface_id) = supervisor_interface();
        let iface = &resolve.interfaces[iface_id];
        assert!(!iface.functions.is_empty(), "supervisor interface should have functions");

        let s = service();
        for name in iface.functions.keys() {
            let method_name = name.strip_prefix('%').unwrap_or(name);
            let res = dispatch(&s, unauthenticated_caller(), method_name, Value::Null).await;
            if let Err(RpcError::MethodNotFound(m)) = res {
                panic!("WIT function '{name}' maps to method name '{m}' but was not dispatched");
            }
        }
    }

    /// §0.29's by-construction property, pinned so a later slice cannot
    /// quietly reintroduce a key-bearing verb: walks the WIT interface and
    /// asserts no function or record field is named like key material.
    #[test]
    fn no_supervisor_verb_accepts_or_returns_key_material() {
        let (resolve, iface_id) = supervisor_interface();
        let iface = &resolve.interfaces[iface_id];

        let suspicious = |name: &str| {
            let lower = name.to_lowercase();
            (lower.contains("key") && !lower.contains("key-hex")) || lower.contains("secret")
        };
        for (type_name, ty) in &iface.types {
            assert!(!suspicious(type_name), "type '{type_name}' looks like key material");
            if let wit_parser::TypeDefKind::Record(record) = &resolve.types[*ty].kind {
                for field in &record.fields {
                    assert!(
                        !suspicious(&field.name),
                        "field '{}' looks like key material",
                        field.name
                    );
                }
            }
        }
        for func_name in iface.functions.keys() {
            assert!(
                !suspicious(func_name),
                "function '{func_name}' looks like it handles key material"
            );
        }
    }

    // ── M05A A5c phase 5: the loop (§19.7/§19.8/§19.12/§19.16, D-A5c-7/8/11/14) ──

    /// §19.16/D-A5c-14: `all_active` already excludes both flags from the
    /// loop's own work list, so a pass over either instance never runs at
    /// all -- proven from the outside by the alert `reconcile_instance_
    /// pass` would otherwise raise: `plan_json_one_service(..., None)` has
    /// no placement, which every other alert test in this file uses as
    /// the cheapest fixture that opens `InstanceNotRunning` the moment a
    /// pass actually processes the instance.
    #[tokio::test]
    async fn the_loop_skips_paused_and_retired_instances() {
        let s = service();
        let paused_plan = plan_json_one_service("paused-inst", "backend", None);
        let retired_plan = plan_json_one_service("retired-inst", "backend", None);
        s.store.submit("paused-inst", &paused_plan, "{}", "did:key:owner", 0).unwrap();
        s.store.submit("retired-inst", &retired_plan, "{}", "did:key:owner", 0).unwrap();
        s.store.pause("paused-inst").unwrap();
        s.store.retire("retired-inst").unwrap();

        s.run_pass().await;

        assert!(s.store.alerts.active(&AppInstanceId::new("paused-inst")).unwrap().is_empty());
        assert!(s.store.alerts.active(&AppInstanceId::new("retired-inst")).unwrap().is_empty());
    }

    /// Review finding A-8: `last_reconciled_at` used to be hardcoded
    /// `None` forever, under a stale comment claiming no loop existed to
    /// fill it. A paused/retired instance is skipped before the health
    /// sweep even runs (see the test above), so it must stay unstamped;
    /// a plain instance with an empty service list still gets a full
    /// pass (the health sweep and the diff both run over zero services)
    /// and must be stamped by it.
    #[tokio::test]
    async fn a_loop_pass_stamps_last_reconciled_at_but_a_skipped_instance_is_untouched() {
        let s = service();
        let plan = plan_json_no_services("inst-1");
        let paused_plan = plan_json_no_services("paused-inst");
        s.store.submit("inst-1", &plan, "{}", "did:key:owner", 0).unwrap();
        s.store.submit("paused-inst", &paused_plan, "{}", "did:key:owner", 0).unwrap();
        s.store.pause("paused-inst").unwrap();

        s.run_pass().await;

        assert!(s.last_reconciled.contains_key("inst-1"));
        assert!(!s.last_reconciled.contains_key("paused-inst"));
    }

    /// §19.16/F6/D-A5c-14: `apply_write_phase` is the write phase
    /// `reconcile_instance_pass` calls after its health sweep -- this
    /// tests its own re-read directly, standing in for a `pause` that
    /// lands during the sweep (which does not hold the per-instance lock
    /// a pass otherwise holds for its whole duration, D-A5c-7). If the
    /// write phase used the state the pass started with instead of
    /// re-reading, this would append a journal record; it must not.
    #[tokio::test]
    async fn a_pause_landing_mid_pass_stops_that_passs_writes() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let needs_work: BTreeSet<String> = ["inst-1/backend".to_string()].into_iter().collect();

        // The pause lands before the write phase's own re-read -- exactly
        // the F6 window, simulated directly rather than raced.
        s.store.pause("inst-1").unwrap();

        s.apply_write_phase(WritePhase {
            instance_id: &AppInstanceId::new("inst-1"),
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &needs_work,
            restart_candidates: &[],
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            push_candidates: &[],
            schedule_decisions: &[],
            did_to_alias: &BTreeMap::new(),
            clients: &BTreeMap::new(),
            now: 0,
        })
        .await;

        assert!(
            s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().is_none(),
            "a paused instance must not have had a deploy attempted"
        );
    }

    /// Review finding A-7: a record left `Applying` by a process that
    /// crashed between `journal.append` and `journal.update_state` must
    /// not pin `handle_status` to "Applying" forever. The per-instance
    /// lock a pass holds is what makes this safe to recover on sight --
    /// nothing can genuinely still be applying for this instance while
    /// the pass itself holds that lock.
    #[tokio::test]
    async fn a_pass_recovers_a_deployment_record_stuck_in_applying() {
        let s = service();
        let plan_json = plan_json_no_services("inst-1");
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.journal.append(&plan, DeploymentState::Applying).unwrap();

        s.reconcile_instance_pass("inst-1").await;

        let latest = s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
        assert_eq!(latest.state, DeploymentState::Degraded);
    }

    /// Review finding A-4: a placement change was already refused
    /// (D-A5c-1), but nothing raised the alert §21 q9 specifies for it --
    /// only `Display`/`FromStr` ever touched the variant. A refusal must
    /// now be visible on `alerts`, not only as this call's own `Err`.
    #[tokio::test]
    async fn refuse_placement_change_raises_and_stores_placement_change_refused() {
        let s = service();
        let landed_plan =
            DeploymentPlan::from_json(&plan_json_one_service("inst-1", "backend", Some("edge-1")))
                .unwrap();
        let deployment_id = s.store.journal.append(&landed_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let moved_plan =
            DeploymentPlan::from_json(&plan_json_one_service("inst-1", "backend", Some("edge-2")))
                .unwrap();
        let inventory = SupervisorInventory::from([(
            "edge-2".to_string(),
            SupervisorInventoryEntry {
                did: "did:key:zEdge2".to_string(),
                api_url: None,
                ucan: None,
            },
        )]);

        let err = s.refuse_placement_change(&moved_plan, &inventory).await.unwrap_err();
        assert!(err.contains("does not relocate"), "{err}");

        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(alerts.iter().any(|a| a.kind == AlertKind::PlacementChangeRefused), "{alerts:?}");
    }

    /// M05A A5e §33.6/D-A5e-6, test 59: `refuse_placement_change` used to
    /// compare a member's plan entry against `current_placement(&landed,
    /// &l_ref)` keyed on the bare logical ref, so with two members placed
    /// on different substrates, member 1's entry was compared against
    /// member 0's landed row -- different DIDs, refused as a relocation
    /// though nothing moved. Keying on `member_ref()` (§33.2) is what
    /// makes cross-substrate `replicas` even expressible.
    #[tokio::test]
    async fn a_second_member_placed_on_a_different_substrate_is_not_refused_as_a_relocation() {
        let s = service();
        let landed_plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [
                {
                    "service_id": "did:key:hFabricated0",
                    "logical_ref": "inst-1/backend",
                    "substrate": "edge-1",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "redundant",
                    "member_index": 0
                },
                {
                    "service_id": "did:key:hFabricated1",
                    "logical_ref": "inst-1/backend",
                    "substrate": "edge-2",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "redundant",
                    "member_index": 1
                }
            ]
        })
        .to_string();
        let landed_plan = DeploymentPlan::from_json(&landed_plan_json).unwrap();
        let deployment_id = s.store.journal.append(&landed_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/backend#1",
                Some("edge-2"),
                "did:key:zEdge2",
                ActionState::Completed,
            )
            .unwrap();

        // The same plan resubmitted -- neither member's substrate changed,
        // but member 1 sits on a substrate distinct from member 0's, the
        // exact shape that used to compare it against the wrong sibling.
        let inventory = SupervisorInventory::from([
            (
                "edge-1".to_string(),
                SupervisorInventoryEntry {
                    did: "did:key:zEdge1".to_string(),
                    api_url: None,
                    ucan: None,
                },
            ),
            (
                "edge-2".to_string(),
                SupervisorInventoryEntry {
                    did: "did:key:zEdge2".to_string(),
                    api_url: None,
                    ucan: None,
                },
            ),
        ]);

        s.refuse_placement_change(&landed_plan, &inventory).await.unwrap();

        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(
            !alerts.iter().any(|a| a.kind == AlertKind::PlacementChangeRefused),
            "neither member actually moved: {alerts:?}"
        );
    }

    /// D-A5e-14: `SynAppManifest::validate()`'s cap is a compile-time
    /// check on a manifest `submit`/`force-reconcile` never see -- they
    /// take an already-compiled plan straight as JSON, so this is the
    /// re-check at the interface that actually accepts one.
    #[test]
    fn refuse_replicas_above_cap_refuses_a_plan_naming_more_members_than_the_cap() {
        let services: Vec<PlannedService> = (0..=MAX_REPLICAS)
            .map(|i| {
                let mut svc = dependent_service("backend", "unrelated");
                svc.member_index = i;
                svc
            })
            .collect();
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services,
        };
        let err = SupervisorService::refuse_replicas_above_cap(&plan).unwrap_err();
        assert!(err.contains(&format!("above the cap of {MAX_REPLICAS}")), "{err}");
    }

    /// A plan naming exactly `MAX_REPLICAS` members is not refused --
    /// only strictly above the cap is, matching `validate()`'s own rule.
    #[test]
    fn refuse_replicas_above_cap_allows_a_plan_exactly_at_the_cap() {
        let services: Vec<PlannedService> = (0..MAX_REPLICAS)
            .map(|i| {
                let mut svc = dependent_service("backend", "unrelated");
                svc.member_index = i;
                svc
            })
            .collect();
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services,
        };
        assert!(SupervisorService::refuse_replicas_above_cap(&plan).is_ok());
    }

    /// D-A5c-11 (§19.12): `update_superseded_alert` is the exact decision
    /// `reconcile_instance_pass` gates its write phase on (`if superseded
    /// { return }`, before `apply_write_phase` is ever reached) -- tested
    /// directly, since driving a real higher `held_max` through a full
    /// pass needs a live substrate actually reporting one. The "still
    /// polls it" half is `an_instance_with_a_planned_service_that_never_
    /// landed_reports_degraded` and this file's other alert tests: the
    /// health sweep in `reconcile_instance_pass` always runs before this
    /// check, unconditionally, so nothing about being superseded can
    /// suppress it.
    #[test]
    fn the_loop_skips_every_write_for_a_superseded_instance_but_still_polls_it() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");

        let superseded = s.update_superseded_alert(&instance_id, "inst-1", Some(5), 2).unwrap();
        assert!(superseded);
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::SupervisorSuperseded)
        );
    }

    /// The boundary `max_held_generation_from_clients`'s own doc names:
    /// nothing reachable must not be confused with "reachable and behind"
    /// -- `reconcile_instance_pass` never even computes a `Some` held-max
    /// when every alias for this plan-only-placed instance is
    /// unreachable (no clients ever connect, since the plan places
    /// nothing), so `superseded` stays `false` and the pass is not
    /// short-circuited: its health sweep still ran and raised
    /// `InstanceNotRunning`.
    #[tokio::test]
    async fn an_unreachable_generation_read_does_not_mark_an_instance_superseded() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        s.reconcile_instance_pass("inst-1").await;

        let instance_id = AppInstanceId::new("inst-1");
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(!active.iter().any(|a| a.kind == AlertKind::SupervisorSuperseded), "{active:?}");
        assert!(active.iter().any(|a| a.kind == AlertKind::InstanceNotRunning), "{active:?}");
    }

    /// Review finding C-3: D-A5c-12's poll-cost budget is "at most 2 RPCs
    /// per substrate per pass" (one batched `status`, one
    /// `app-instance-management-of`) -- the shipped budget test
    /// (`orchestration.rs`) measures wall-clock duration only, and
    /// nothing anywhere asserted the RPC-count half as a number that
    /// could regress. This pins the `app-instance-management-of` half
    /// directly: `max_held_generation_from_clients` must call
    /// `held_generation` exactly once per *alias* (one call per
    /// substrate), never once per service placed on it -- three aliases
    /// here stand in for a substrate hosting many services, and the
    /// count must stay 3, not grow with however many services this test
    /// does not even bother placing. The "one batched status" half has
    /// no equivalent unit seam (`SyneroymClient` is concrete, not
    /// injectable into the health-poll path) and stays a duration-only
    /// regression guard; recorded in the deferred backlog.
    #[tokio::test]
    async fn max_held_generation_from_clients_calls_held_generation_once_per_alias() {
        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let aliases: BTreeSet<String> =
            ["edge-1", "edge-2", "edge-3"].into_iter().map(String::from).collect();
        let clients: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
            aliases.iter().map(|a| (SubstrateAlias::new(a.clone()), dyn_actor.clone())).collect();

        let held_max =
            SupervisorService::max_held_generation_from_clients("inst-1", &aliases, &clients).await;

        assert_eq!(held_max, Some(0));
        assert_eq!(*actor.held_generation_calls.lock().unwrap(), 3);
    }

    /// D-A5c-7 (§19.7): `instance_lock` itself, which two concurrently
    /// driven holders for the *same* instance id must never both be
    /// inside at once. `instance_lock` for two *different* ids would
    /// return two different mutexes and is not what this proves. Review
    /// finding C-4: this pins the lock's own mutual exclusion, not that
    /// `submit` and a loop pass actually reach for it -- the two tests
    /// below drive the real methods.
    #[tokio::test]
    async fn a_submit_and_a_loop_pass_for_one_instance_do_not_interleave() {
        let s = service();
        let inside = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let overlapped = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let lock = s.instance_lock("inst-1");
            let inside = inside.clone();
            let overlapped = overlapped.clone();
            handles.push(tokio::spawn(async move {
                let _guard = lock.lock().await;
                if inside.fetch_add(1, std::sync::atomic::Ordering::SeqCst) != 0 {
                    overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                inside.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Review finding C-4: drives the real `run_pass` against a real
    /// externally-held `instance_lock`, rather than four anonymous
    /// holders of it -- proof that a loop pass genuinely blocks on the
    /// same lock `instance_lock(app_instance_id)` returns, not merely
    /// that the lock type is a working mutex.
    #[tokio::test]
    async fn a_loop_pass_blocks_on_this_instances_externally_held_lock() {
        let s = Arc::new(service());
        let plan_json = plan_json_no_services("inst-1");
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let held = s.instance_lock("inst-1");
        let guard = held.lock().await;

        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pass_s = s.clone();
        let pass_done = done.clone();
        let handle = tokio::spawn(async move {
            pass_s.run_pass().await;
            pass_done.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !done.load(std::sync::atomic::Ordering::SeqCst),
            "a loop pass must not proceed while this instance's lock is held elsewhere"
        );

        drop(guard);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("the pass must proceed once the lock is released")
            .unwrap();
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Review finding C-4's other half: `handle_submit` for the same
    /// instance id must block on that instance's lock too, driven
    /// through the real `dispatch("submit", …)` path rather than a
    /// stand-in.
    #[tokio::test]
    async fn a_submit_blocks_on_this_instances_externally_held_lock() {
        let s = Arc::new(service());
        let plan_json = plan_json_no_services("inst-1");

        let held = s.instance_lock("inst-1");
        let guard = held.lock().await;

        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let submit_s = s.clone();
        let submit_done = done.clone();
        let submit_plan_json = plan_json.clone();
        let handle = tokio::spawn(async move {
            dispatch(
                &submit_s,
                admin_caller("did:key:zSupervisorNode"),
                "submit",
                serde_json::json!([{
                    "app_instance_id": "inst-1",
                    "plan_json": submit_plan_json,
                    "inventory_json": "{}",
                    "generation": 0,
                }]),
            )
            .await
            .unwrap();
            submit_done.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !done.load(std::sync::atomic::Ordering::SeqCst),
            "submit must not proceed while this instance's lock is held elsewhere"
        );

        drop(guard);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("submit must proceed once the lock is released")
            .unwrap();
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// D-A5c-8 (§19.8/F1): the loop is spawned, not pinned in a `select!`
    /// that would drop it mid-pass -- `shutdown` only cancels the token
    /// (production's own `RuntimeServices` is what holds the
    /// `JoinHandle`), so this test spawns and joins it the same way that
    /// caller does, and asserts the join resolves promptly rather than
    /// hanging or requiring a second cancellation.
    #[tokio::test]
    async fn shutdown_cancels_the_spawned_loop_and_waits_for_it_to_close_its_clients() {
        let s = Arc::new(service());
        let spawned = s.clone();
        let handle = tokio::spawn(async move { spawned.run().await });

        // Let the loop reach its first `interval.tick()` wait (the first
        // tick fires immediately and `run_pass` over an empty store
        // returns at once).
        tokio::time::sleep(Duration::from_millis(20)).await;

        s.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("the spawned loop did not stop within 2s of shutdown")
            .unwrap()
            .unwrap();
    }

    /// D-A5c-7/§19.7: pins `run`'s interval configuration directly, under
    /// a paused clock rather than a real slow pass -- `Skip` must let a
    /// tick that arrives long after a missed period fire once,
    /// immediately, rather than the default `Burst` behavior firing once
    /// per period that elapsed.
    #[tokio::test(start_paused = true)]
    async fn a_pass_that_outruns_the_interval_does_not_queue_a_burst() {
        let mut interval = SupervisorService::build_pass_interval(1);
        interval.tick().await;

        // Ten missed periods' worth of virtual time elapses while a pass
        // is imagined to be still running.
        tokio::time::advance(Duration::from_secs(10)).await;

        let before_catchup = tokio::time::Instant::now();
        interval.tick().await;
        assert_eq!(
            tokio::time::Instant::now(),
            before_catchup,
            "a Skip interval must resolve the missed ticks immediately, not wait out each one"
        );

        let after_catchup = tokio::time::Instant::now();
        interval.tick().await;
        assert!(
            tokio::time::Instant::now() >= after_catchup + Duration::from_secs(1),
            "no burst of queued ticks should remain after catching up once"
        );
    }

    // ── M05A A5c phase 6: remediation (§14 step 3/6, §21, D-A5c-15/17/20/21) ──

    /// A fake `SubstrateActor` that only counts `restart` calls -- every
    /// other method is unreachable from a remediation test (M05A A5c
    /// §23, tests 35-38).
    #[derive(Debug, Default)]
    struct CountingActor {
        restart_calls: Mutex<u32>,
        held_generation_calls: Mutex<u32>,
    }

    #[async_trait::async_trait]
    impl SubstrateActor for CountingActor {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by remediation tests")
        }

        async fn write_bindings(
            &self,
            _write: syneroym_sdk::BindingWrite,
        ) -> Result<Vec<syneroym_sdk::BindingWriteOutcome>, String> {
            unimplemented!("not exercised by remediation tests")
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            *self.restart_calls.lock().unwrap() += 1;
            Ok(())
        }

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by remediation tests")
        }

        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("not exercised by remediation tests")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            *self.held_generation_calls.lock().unwrap() += 1;
            Ok(Some(0))
        }
    }

    fn service_health(l_ref: &str, substrate_did: &str, signal: Signal) -> health::ServiceHealth {
        let (instance, name) = l_ref.split_once('/').unwrap();
        health::ServiceHealth {
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new(instance),
                service_name: LogicalServiceName::new(name),
            },
            service_id: format!("did:key:h{name}"),
            alias: None,
            substrate_did: substrate_did.to_string(),
            signal,
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            binding_epochs: Vec::new(),
            member_index: 0,
        }
    }

    /// §14 step 3: a landed service the sweep finds `InstanceNotRunning`
    /// gets one bounded restart attempt.
    #[tokio::test]
    async fn instance_not_running_triggers_a_restart_on_the_next_pass() {
        let s = service();
        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let mut opened = Vec::new();
        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            1_000,
            &mut opened,
        )
        .await;

        assert_eq!(*actor.restart_calls.lock().unwrap(), 1);
        let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
        assert_eq!(state.attempts, 1);
        assert!(!state.terminal);
        assert!(opened.is_empty(), "one attempt must not exhaust a 3-attempt budget");
    }

    /// M05A A5e §33.11, test 60: two members of one scaled service must
    /// each spend their own `max_restart_attempts` budget --
    /// `restart_candidates` keys on `ServiceHealth::member_ref()` (member
    /// 0 and member 1 are two distinct candidates, per D-A5e-2), and
    /// `attempt_restart`'s remediation row is keyed on that same string.
    /// A regression back to a bare logical ref would collapse the two
    /// into one shared counter -- member 1's failures exhausting member
    /// 0's budget, and vice versa.
    #[tokio::test]
    async fn restart_attempts_are_counted_per_member_not_per_logical_service() {
        let s = service();
        let report = report_of(vec![
            {
                let mut h = service_health(
                    "inst-1/backend",
                    "did:key:zEdge1",
                    Signal::InstanceNotRunning(String::new()),
                );
                h.member_index = 0;
                h
            },
            {
                let mut h = service_health(
                    "inst-1/backend",
                    "did:key:zEdge2",
                    Signal::InstanceNotRunning(String::new()),
                );
                h.member_index = 1;
                h
            },
        ]);
        let candidates = SupervisorService::restart_candidates(&report);
        assert_eq!(
            candidates.iter().map(|(l_ref, ..)| l_ref.as_str()).collect::<BTreeSet<_>>(),
            BTreeSet::from(["inst-1/backend#0", "inst-1/backend#1"]),
            "two members must be two distinct restart candidates: {candidates:?}"
        );

        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();
        // Member 0 spends its whole 3-attempt budget (the fixture's
        // default), well past its own backoff each time.
        for now in [1_000u64, 1_100u64, 1_200u64] {
            s.attempt_restart(
                &instance_id,
                "inst-1",
                "inst-1/backend#0",
                "did:key:hbackend0",
                "did:key:zEdge1",
                &dyn_actor,
                0,
                now,
                &mut opened,
            )
            .await;
        }
        let member0 = s.store.remediation_state("inst-1", "inst-1/backend#0").unwrap().unwrap();
        assert_eq!(member0.attempts, 3);
        assert!(member0.terminal, "member 0 must be exhausted after 3 attempts");

        // Member 1 has never been attempted -- its own row must still
        // read fresh, not inherit member 0's exhausted state.
        let member1 = s.store.remediation_state("inst-1", "inst-1/backend#1").unwrap();
        assert!(member1.is_none(), "member 1 must have its own, untouched remediation row");

        let mut opened1 = Vec::new();
        s.attempt_restart(
            &instance_id,
            "inst-1",
            "inst-1/backend#1",
            "did:key:hbackend1",
            "did:key:zEdge2",
            &dyn_actor,
            0,
            1_000,
            &mut opened1,
        )
        .await;
        let member1 = s.store.remediation_state("inst-1", "inst-1/backend#1").unwrap().unwrap();
        assert_eq!(member1.attempts, 1, "member 1's first attempt must not be refused as terminal");
        assert!(!member1.terminal);
    }

    /// `restart_backoff_secs` (30 in the fixture, D-A5c-14's table): a
    /// second attempt inside that window is refused before the actor is
    /// ever called again.
    #[tokio::test]
    async fn a_restart_is_not_retried_before_the_backoff_elapses() {
        let s = service();
        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let mut opened = Vec::new();
        for now in [1_000u64, 1_010u64] {
            s.attempt_restart(
                &AppInstanceId::new("inst-1"),
                "inst-1",
                "inst-1/backend",
                "did:key:hBackend",
                "did:key:zEdge1",
                &dyn_actor,
                0,
                now,
                &mut opened,
            )
            .await;
        }
        assert_eq!(
            *actor.restart_calls.lock().unwrap(),
            1,
            "the second attempt was inside backoff"
        );
        assert_eq!(
            s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().attempts,
            1
        );
    }

    /// Matrix row 13: exceeding `max_restart_attempts` (3 in the fixture)
    /// marks the service terminal and raises `RemediationExhausted`
    /// exactly once, on the attempt that crosses the ceiling.
    #[tokio::test]
    async fn remediation_stops_after_max_attempts_and_alerts_once() {
        let s = service();
        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let mut opened = Vec::new();
        // Each attempt spaced past `restart_backoff_secs` (30) so none is
        // refused for being too soon.
        for i in 0..3u64 {
            s.attempt_restart(
                &AppInstanceId::new("inst-1"),
                "inst-1",
                "inst-1/backend",
                "did:key:hBackend",
                "did:key:zEdge1",
                &dyn_actor,
                0,
                1_000 + i * 100,
                &mut opened,
            )
            .await;
        }
        assert_eq!(*actor.restart_calls.lock().unwrap(), 3);
        let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
        assert_eq!(state.attempts, 3);
        assert!(state.terminal);
        assert_eq!(
            opened.iter().filter(|(k, _)| *k == AlertKind::RemediationExhausted).count(),
            1,
            "{opened:?}"
        );
    }

    /// Row 13's other half: once terminal, a later pass's attempt must
    /// not call the actor again, however long it has been.
    #[tokio::test]
    async fn a_terminal_degraded_service_is_never_restarted_again() {
        let s = service();
        let actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let mut opened = Vec::new();
        for i in 0..3u64 {
            s.attempt_restart(
                &AppInstanceId::new("inst-1"),
                "inst-1",
                "inst-1/backend",
                "did:key:hBackend",
                "did:key:zEdge1",
                &dyn_actor,
                0,
                1_000 + i * 100,
                &mut opened,
            )
            .await;
        }
        assert!(s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().terminal);

        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            1_000_000,
            &mut opened,
        )
        .await;
        assert_eq!(
            *actor.restart_calls.lock().unwrap(),
            3,
            "a terminal service must not be restarted again"
        );
    }

    /// Review finding C-2: tests 35-38 (above) call `attempt_restart`
    /// directly, and 39-40 (below) test `restart_candidates` in
    /// isolation -- nothing drove the wiring between them, the
    /// `did_to_alias -> clients -> actor` lookup inside
    /// `apply_write_phase` that a mis-keyed alias or DID would silently
    /// `continue` past with no restart and no error. Uses a real,
    /// never-connected `SyneroymClient` rather than a fake: its `restart`
    /// fails fast with "Not connected" (no socket, no hang), which is
    /// enough to prove the lookup found it and called it -- the point
    /// here is the wiring, not the RPC outcome, which `attempt_restart`'s
    /// own tests already cover.
    #[tokio::test]
    async fn a_restart_candidate_reaches_the_actor_through_apply_write_phases_own_lookup() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();

        let identity = Identity::generate().unwrap();
        let client = Arc::new(SyneroymClient::new_with_identity(
            "did:key:zEdge1".to_string(),
            String::new(),
            identity,
        ));
        let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);
        let did_to_alias: BTreeMap<String, String> =
            BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())]);
        let restart_candidates = vec![(
            "inst-1/backend".to_string(),
            "did:key:hFabricated".to_string(),
            "did:key:zEdge1".to_string(),
        )];

        s.apply_write_phase(WritePhase {
            instance_id: &AppInstanceId::new("inst-1"),
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &BTreeSet::new(),
            restart_candidates: &restart_candidates,
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            push_candidates: &[],
            schedule_decisions: &[],
            did_to_alias: &did_to_alias,
            clients: &clients,
            now: 0,
        })
        .await;

        let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap();
        assert!(
            state.is_some_and(|r| r.attempts == 1),
            "the candidate must reach a real actor call and record an attempt, not be silently \
             dropped by the did_to_alias/clients lookup: {state:?}"
        );
    }

    /// §21/D-A5c-17: a declared readiness probe failing is an author
    /// assertion this supervisor cannot verify, not a substrate-verified
    /// fact -- alert only, pinned so a later slice cannot silently widen
    /// remediation onto it.
    #[test]
    fn probe_failing_never_triggers_a_restart() {
        let report = health::HealthReport {
            substrates: Vec::new(),
            services: vec![service_health(
                "inst-1/backend",
                "did:key:zEdge1",
                Signal::ProbeFailing("readiness check failing".to_string()),
            )],
        };
        assert!(SupervisorService::restart_candidates(&report).is_empty());
    }

    /// D-A4-13, re-pinned for the loop: a substrate that did not answer
    /// is never inferred to mean its services are down, so restarting
    /// cannot be the fix for it either.
    #[test]
    fn substrate_unreachable_never_triggers_a_restart() {
        let report = health::HealthReport {
            substrates: Vec::new(),
            services: vec![service_health(
                "inst-1/backend",
                "did:key:zEdge1",
                Signal::SubstrateUnreachable("no answer".to_string()),
            )],
        };
        assert!(SupervisorService::restart_candidates(&report).is_empty());
    }

    fn plan_json_two_services(instance: &str, a: &str, b: &str) -> String {
        serde_json::json!({
            "app_instance_id": instance,
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [
                {
                    "service_id": "did:key:hFabricatedA",
                    "logical_ref": format!("{instance}/{a}"),
                    "substrate": null,
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "singleton"
                },
                {
                    "service_id": "did:key:hFabricatedB",
                    "logical_ref": format!("{instance}/{b}"),
                    "substrate": null,
                    "service_type": "tcp", "source": "127.0.0.1:9001",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "singleton"
                }
            ]
        })
        .to_string()
    }

    /// D-A5c-3/D-A5c-21 (§19.2a/§19.21): a service the resubmitted plan no
    /// longer names, but that this supervisor's own journal still shows
    /// landed, is reported -- not undeployed. Undeploying a stateful
    /// service because a manifest was edited is destructive, and
    /// `retire` is deliberately not a teardown.
    #[tokio::test]
    async fn a_service_dropped_from_the_plan_raises_orphaned_service_and_is_not_undeployed() {
        let s = service();
        let old_plan_json = plan_json_two_services("inst-1", "backend", "frontend");
        let old_plan = DeploymentPlan::from_json(&old_plan_json).unwrap();
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/frontend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        // Resubmitted plan drops `frontend`.
        let new_plan_json = plan_json_one_service("inst-1", "backend", None);
        s.store.submit("inst-1", &new_plan_json, "{}", "did:key:owner", 0).unwrap();

        s.reconcile_instance_pass("inst-1").await;

        let instance_id = AppInstanceId::new("inst-1");
        let active = s.store.alerts.active(&instance_id).unwrap();
        let orphan = active
            .iter()
            .find(|a| a.kind == AlertKind::OrphanedService)
            .unwrap_or_else(|| panic!("no OrphanedService alert among {active:?}"));
        assert_eq!(orphan.logical_ref.as_deref(), Some("inst-1/frontend#0"));
        assert_eq!(orphan.substrate_did, "did:key:zEdge1");
    }

    // ── M05A A5c phase 7: the binding push and convergence (§19.3/§19.4/§19.19,
    // D-A5c-4/5/16/19) ──

    /// A fake `SubstrateActor` that only answers `write_bindings`, from a
    /// caller-queued sequence of responses (defaulting to `Applied` once
    /// the queue empties) -- every other method is unreachable from a
    /// push test (M05A A5c §23, tests 42-47).
    #[derive(Debug, Default)]
    struct BindingActor {
        responses: Mutex<Vec<Result<Vec<BindingWriteOutcome>, String>>>,
        calls: Mutex<Vec<BindingWrite>>,
    }

    #[async_trait::async_trait]
    impl SubstrateActor for BindingActor {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by push tests")
        }

        async fn write_bindings(
            &self,
            write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            self.calls.lock().unwrap().push(write);
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(vec![BindingWriteOutcome::Applied])
            } else {
                responses.remove(0)
            }
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("not exercised by push tests")
        }

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by push tests")
        }

        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("not exercised by push tests")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by push tests")
        }
    }

    fn dependent_service(name: &str, dep_name: &str) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(format!("did:key:h{name}")),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(name),
            },
            substrate: Some(SubstrateAlias::new("edge-1")),
            config: dummy_config(),
            resolved_dependencies: BTreeMap::from([(
                LogicalServiceName::new(dep_name),
                vec![ServiceId::new("did:key:hDepMember")],
            )]),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: Default::default(),
        }
    }

    fn dummy_config() -> ServiceConfig {
        ServiceConfig {
            service_type: ServiceType::Tcp,
            source: "127.0.0.1:9000".to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: Default::default(),
            fdae: None,
            health_check: None,
            assets: None,
            visibility: Default::default(),
        }
    }

    fn plan_with_one_dependent(svc: PlannedService) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![svc],
        }
    }

    /// D-A5e-9: the convergence budget is measured from the membership
    /// change to the last applied write returning `Applied`/`NoOp` --
    /// *not* off `binding-epochs`, whose own refresh is bounded by
    /// `poll_interval_secs` (default 30s, six times the 5s budget). This
    /// harness proves the two are not the same clock: a push against a
    /// fake actor that answers immediately completes in a time nowhere
    /// near a poll interval, so a measurement taken this way is the
    /// write's own latency, never silently the read surface's lag.
    ///
    /// M05A A5e review, second round: the clock starts at
    /// `Reconciler::compute_diff` and runs through
    /// `classify_update_actions`, the same classifier
    /// `apply_with_membership_pushes`/`apply_write_phase` call -- not just
    /// the `push_bindings` call after it has already decided -- so a
    /// regression that makes the routing decision itself slow (e.g. an
    /// O(n²) diff over a large plan) is inside what this measures, not
    /// hidden before it.
    #[tokio::test]
    async fn convergence_is_measured_from_the_membership_change_to_the_last_applied_write() {
        let s = service();
        let old_svc = dependent_service("frontend", "backend");
        let old_plan = plan_with_one_dependent(old_svc.clone());
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/frontend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let mut new_svc = old_svc.clone();
        new_svc.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
        )]);
        let plan = plan_with_one_dependent(new_svc);

        let actor = Arc::new(BindingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        // The membership change: the moment a `submit`'s own diff would
        // see it, before the classifier has decided anything. The clock
        // stops when the write this decision routes to returns.
        let start = Instant::now();
        let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
        let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
        let (_, push_candidates) =
            SupervisorService::classify_update_actions(&landed, &diff.actions);
        let (svc, substrate_did) =
            push_candidates.into_iter().next().expect("frontend must classify as a push candidate");
        let outcomes = s
            .push_bindings(&instance_id, &plan, &svc, &substrate_did, &dyn_actor, 0, &mut opened)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(outcomes, PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
        assert!(
            elapsed < Duration::from_secs(1),
            "the measured interval must cover the routing decision and the write's own latency, \
             far under a poll interval (default 30s) and the 5s budget alike, not \
             `binding-epochs`' own read lag: {elapsed:?}"
        );
    }

    /// D-A5c-4: a push advances this dependent's epoch before sending,
    /// and a clean `Applied` outcome leaves the new value on record --
    /// what the next pass's convergence read compares against.
    #[tokio::test]
    async fn a_membership_change_pushes_at_the_next_epoch_and_records_it() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        let outcomes = s
            .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        assert_eq!(outcomes, PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
        assert_eq!(actor.calls.lock().unwrap().len(), 1);
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
        assert!(opened.is_empty());
    }

    /// M05B B1 review follow-up (2026-08-05): a write with zero bindings
    /// is a real, converged success -- not the same value `push_bindings`
    /// used to signal "deferred to an already-pending queue item" before
    /// `PushOutcome` existed. Reachable in the ordinary course of a
    /// deploy: removing a service's last `depends_on` leaves
    /// `resolved_dependencies` empty, `only_resolved_dependencies_changed`
    /// still classifies the member as a push candidate purely because the
    /// field *changed*, and the write this produces legitimately carries
    /// zero bindings -- `orchestration.rs`'s `write_bindings_impl` builds
    /// one outcome per binding sent, so the substrate legitimately answers
    /// with zero too. Before `PushOutcome`, this collapsed onto the same
    /// `Vec::new()` the deferred-to-queue sentinel used, permanently
    /// downgrading the member every pass.
    #[tokio::test]
    async fn a_push_with_zero_bindings_lands_rather_than_reading_as_deferred() {
        let s = service();
        let svc = PlannedService {
            resolved_dependencies: BTreeMap::new(),
            ..dependent_service("frontend", "backend")
        };
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Ok(Vec::new()));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        let outcome = s
            .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            PushOutcome::Landed(Vec::new()),
            "zero bindings is a real, converged success, not a deferral"
        );
    }

    /// D-A5e-4: `map_deployment_plan_to_wit` reads a binding's `mode` off
    /// the *dependency's own* `PlannedService.topology_mode` in the plan
    /// -- not off `resolved_dependencies`' member count -- so `backend`
    /// must be present in `plan.services` with `Redundant` already
    /// compiled onto it (D-A5e-4: `replicas > 1` ⇒ `Redundant`) for the
    /// push to carry it correctly. The unit-level half of test 70's
    /// amended step 5 (R5): proves the flip at the binding-write layer
    /// itself, without needing a live substrate to observe cross-member
    /// resolution.
    #[tokio::test]
    async fn a_scale_out_push_carries_the_redundant_mode_to_the_dependent() {
        let s = service();
        let frontend = dependent_service("frontend", "backend");
        let mut backend = dependent_service("backend", "unrelated");
        backend.topology_mode = TopologyMode::Redundant;
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![frontend.clone(), backend],
        };
        let actor = Arc::new(BindingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.push_bindings(
            &instance_id,
            &plan,
            &frontend,
            "did:key:zEdge1",
            &dyn_actor,
            0,
            &mut opened,
        )
        .await
        .unwrap();

        let calls = actor.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].bindings.len(), 1);
        assert!(
            matches!(calls[0].bindings[0].mode, syneroym_sdk::TopologyMode::Redundant),
            "{:?}",
            calls[0].bindings[0].mode
        );
    }

    /// D-A5c-4/D-A5c-19: a second writer exists (`Conflict`) is never
    /// retried -- retrying would only race it again.
    #[tokio::test]
    async fn a_conflict_outcome_raises_binding_conflict_and_does_not_retry() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        assert_eq!(actor.calls.lock().unwrap().len(), 1, "a conflict must not be retried");
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
    }

    /// D-A5c-19/F4: `Stale(held)` retries exactly once, at `held + 1` --
    /// not a re-read, and not the held epoch itself (which the four-case
    /// rule would only ever answer with `Conflict`). The retry failing
    /// too still alerts, once.
    #[tokio::test]
    async fn a_stale_outcome_is_retried_once_above_the_held_epoch_then_alerts() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Stale(5)]));
        actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(6)]));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        let calls = actor.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "exactly one retry");
        assert_eq!(calls[1].bindings[0].epoch, 6, "the retry must land at held + 1, not held");
        drop(calls);
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
        assert_eq!(
            s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
            6,
            "the local counter must agree with the substrate after the retry"
        );
    }

    /// F7 -- the exit criterion's own test: an operator reads a converged
    /// binding once the written and observed epochs agree. Read directly
    /// off `binding_convergence_rows` (what `status` calls), since
    /// driving a real observed epoch through `handle_status` needs a
    /// live substrate to report one.
    #[tokio::test]
    async fn status_reports_a_converged_binding_after_a_push_lands() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();
        s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        let report = health::HealthReport {
            substrates: Vec::new(),
            services: vec![{
                let mut h = service_health("inst-1/frontend", "did:key:zEdge1", Signal::Healthy);
                h.binding_epochs = vec![("backend".to_string(), 1)];
                h
            }],
        };
        let rows = s.binding_convergence_rows("inst-1", &plan, &report);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].dependent_logical_ref, "inst-1/frontend#0");
        assert_eq!(rows[0].dependency_name, "backend");
        assert_eq!(rows[0].written_epoch, 1);
        assert_eq!(rows[0].observed_epoch, Some(1));
        assert!(rows[0].converged);
    }

    // ── M05A A5e phase 4: the push trigger (D-A5e-7/D-A5e-8) ────────────

    /// D-A5e-7: the classifier's whole point. A resubmit whose only change
    /// to a dependent member is which DIDs a dependency resolves to must
    /// be routed to a push, not a redeploy.
    #[test]
    fn only_resolved_dependencies_changed_is_true_when_only_the_dependency_map_differs() {
        let old = dependent_service("frontend", "backend");
        let mut new = old.clone();
        new.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
        )]);
        assert!(SupervisorService::only_resolved_dependencies_changed(&old, &new));
    }

    /// The other half of the classifier: any other kind of change --
    /// config, in this case -- still takes the redeploy path, even when
    /// `resolved_dependencies` also changed in the same resubmit.
    #[test]
    fn only_resolved_dependencies_changed_is_false_when_config_also_changes() {
        let old = dependent_service("frontend", "backend");
        let mut new = old.clone();
        new.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
        )]);
        new.config.source = "127.0.0.1:9001".to_string();
        assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
    }

    /// A change to `resolved_dependencies` alone, with nothing else
    /// different at all, is a no-op diff (`old == new`), not an `Update`
    /// action -- `only_resolved_dependencies_changed` is only ever asked
    /// about an actual `Update`, but must not misreport an identical pair.
    #[test]
    fn only_resolved_dependencies_changed_is_false_when_nothing_changed() {
        let old = dependent_service("frontend", "backend");
        let new = old.clone();
        assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
    }

    /// D-A5e-7, second review round: the classifier being correct
    /// (`only_resolved_dependencies_changed`) was never the gap -- the gap
    /// was that `handle_submit`/`deploy_submission` never called it at
    /// all, going straight to `apply_with_clients` over the whole plan.
    /// This drives `apply_with_membership_pushes` itself, the shared
    /// routing both now go through, with a completed placement journaled
    /// for `frontend` and no client built for its substrate: if the
    /// classifier is bypassed and `frontend` reaches `apply_with_clients`
    /// like every other service, the failure comes from `certify_placed_
    /// members`'s "no client"/"no member master" shape; if it is routed to
    /// `push_bindings` instead, the failure is this call's own "not
    /// connected to its landed substrate" -- the two are textually
    /// distinguishable, so this fails loudly if the routing regresses.
    #[tokio::test]
    async fn a_diff_whose_only_change_is_resolved_dependencies_pushes_instead_of_redeploying() {
        let s = service();
        let old_frontend = dependent_service("frontend", "backend");
        let old_plan = plan_with_one_dependent(old_frontend.clone());
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/frontend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let mut new_frontend = old_frontend.clone();
        new_frontend.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMemberScaledOut")],
        )]);
        let new_plan = plan_with_one_dependent(new_frontend);

        let err = s
            .apply_with_membership_pushes(
                &new_plan,
                &BTreeMap::new(),
                &BTreeMap::new(),
                0,
                Vec::new(),
            )
            .await
            .unwrap_err();
        assert!(
            err.contains("not connected to its landed substrate this call"),
            "frontend must be routed to a push attempt, not a redeploy: {err}"
        );
        assert!(err.contains("inst-1/frontend#0"), "{err}");

        // Round 2 review, finding A: the redeploy half journaled `new_plan`
        // -- carrying frontend's already-scaled `resolved_dependencies` --
        // as `Active` before the push above ever ran. Left there, the next
        // pass's diff would read frontend as already converged and never
        // retry the push that just failed. It must be downgraded to
        // `Degraded` instead, so `compute_diff` falls back to `old_plan`.
        let instance_id = AppInstanceId::new("inst-1");
        let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
        assert_eq!(
            latest.state,
            DeploymentState::Degraded,
            "a record carrying an unlanded push must not read as this instance's converged \
             baseline: {latest:?}"
        );

        // The real assertion the state check exists for: the next pass's
        // diff must still see frontend as a push candidate, not as already
        // converged.
        let diff = Reconciler::new(&s.store.journal).compute_diff(&new_plan).unwrap();
        let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
        let (redeploy_exclusions, _) =
            SupervisorService::classify_update_actions(&landed, &diff.actions);
        assert!(
            redeploy_exclusions.contains("inst-1/frontend#0"),
            "the next pass must reclassify frontend as a push candidate, not read it as landed: \
             {diff:?}"
        );
    }

    /// The other half: a member whose diff also changes something besides
    /// `resolved_dependencies` must still take the redeploy path through
    /// `apply_with_membership_pushes`, even though it has a completed
    /// placement too -- the same fixture as the push case above, but
    /// failing through `certify_placed_members`'s shape instead.
    #[tokio::test]
    async fn a_diff_that_also_changes_config_still_takes_the_redeploy_path_through_membership_pushes()
     {
        let s = service();
        let old_frontend = dependent_service("frontend", "backend");
        let old_plan = plan_with_one_dependent(old_frontend.clone());
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/frontend#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let mut new_frontend = old_frontend.clone();
        new_frontend.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMemberScaledOut")],
        )]);
        new_frontend.config.source = "127.0.0.1:9001".to_string();
        let new_plan = plan_with_one_dependent(new_frontend);

        let err = s
            .apply_with_membership_pushes(
                &new_plan,
                &BTreeMap::new(),
                &BTreeMap::new(),
                0,
                Vec::new(),
            )
            .await
            .unwrap_err();
        assert!(
            !err.contains("not connected to its landed substrate this call"),
            "a config change must not be routed through the push path: {err}"
        );
    }

    /// D-A5e-7/§33.7: `Reconciler::compute_diff` produces one `Update`
    /// action per member of the scaled dependency's dependent -- each is
    /// independently a push-only change, so a two-member dependent
    /// pushes on both, not just the first.
    #[test]
    fn a_membership_change_pushes_to_every_member_of_every_dependent() {
        let mut frontend_0 = dependent_service("frontend", "backend");
        let mut frontend_1 = frontend_0.clone();
        frontend_1.member_index = 1;
        frontend_1.service_id = ServiceId::new("did:key:hfrontend1");

        let old_plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![frontend_0.clone(), frontend_1.clone()],
        };
        let journal = DeploymentJournal::open_in_memory().unwrap();
        journal.append(&old_plan, DeploymentState::Active).unwrap();

        let scaled_deps = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
        )]);
        frontend_0.resolved_dependencies = scaled_deps.clone();
        frontend_1.resolved_dependencies = scaled_deps;
        let new_plan =
            DeploymentPlan { services: vec![frontend_0, frontend_1], ..old_plan.clone() };

        let diff = Reconciler::new(&journal).compute_diff(&new_plan).unwrap();
        assert_eq!(diff.actions.len(), 2, "{:?}", diff.actions);
        for action in &diff.actions {
            match action {
                ReconcileAction::Update { old, new } => {
                    assert!(SupervisorService::only_resolved_dependencies_changed(old, new));
                }
                other => panic!("expected an Update per member, got {other:?}"),
            }
        }
    }

    /// M05A A5e review (matrix row 11): a push candidate this pass could
    /// not even connect to used to be dropped with a bare `continue` --
    /// no alert, no `Degraded`. Drives `apply_write_phase` directly (the
    /// same seam `a_pause_landing_mid_pass_stops_that_passs_writes` uses)
    /// with `did_to_alias` empty, standing in for a dependent whose
    /// substrate this pass's own connect step never reached.
    #[tokio::test]
    async fn an_unreachable_push_candidate_raises_binding_conflict_instead_of_being_dropped_silently()
     {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "frontend", Some("edge-1"));
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        let svc = dependent_service("frontend", "backend");
        let instance_id = AppInstanceId::new("inst-1");

        s.apply_write_phase(WritePhase {
            instance_id: &instance_id,
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &BTreeSet::new(),
            restart_candidates: &[],
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            push_candidates: &[(svc, "did:key:zEdge1".to_string())],
            schedule_decisions: &[],
            did_to_alias: &BTreeMap::new(),
            clients: &BTreeMap::new(),
            now: 0,
        })
        .await;

        let alerts = s.store.alerts.active(&instance_id).unwrap();
        let conflict = alerts
            .iter()
            .find(|a| a.kind == AlertKind::BindingConflict)
            .unwrap_or_else(|| panic!("no BindingConflict alert among {alerts:?}"));
        assert_eq!(conflict.logical_ref.as_deref(), Some("inst-1/frontend#0"));
        assert_eq!(conflict.substrate_did, "did:key:zEdge1");
    }

    /// M05A A5e review round 2, finding A -- the narrower loop-path shape:
    /// `record_plan_for_pass` keeps a push candidate's *new*
    /// `resolved_dependencies` unconditionally, so a `needs_work` redeploy
    /// landing in the *same* pass as a *failing* push journals that push
    /// candidate as already converged before the push loop below ever
    /// runs. `backend` is revoked so `apply_with_clients`'s own filter
    /// empties it out before certify/deploy, letting the redeploy "land"
    /// (and journal `Active`) with no live substrate -- the same trick
    /// `a_submit_of_the_same_plan_does_not_recertify_a_revoked_placement`
    /// uses. `frontend`'s push then fails (no client for its alias).
    #[tokio::test]
    async fn a_needs_work_redeploy_and_a_failing_push_in_the_same_pass_leaves_the_record_degraded()
    {
        let s = service();
        let old_frontend = dependent_service("frontend", "backend");
        let backend = PlannedService {
            service_id: ServiceId::new("did:key:hbackend"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new("backend"),
            },
            substrate: Some(SubstrateAlias::new("edge-1")),
            config: dummy_config(),
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: Default::default(),
        };
        let old_plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![old_frontend.clone(), backend.clone()],
        };
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        for (l_ref, alias, did) in [
            ("inst-1/frontend#0", "edge-1", "did:key:zEdge1"),
            ("inst-1/backend#0", "edge-1", "did:key:zEdge1"),
        ] {
            s.store
                .journal
                .append_action(
                    deployment_id,
                    "ADD",
                    l_ref,
                    Some(alias),
                    did,
                    ActionState::Completed,
                )
                .unwrap();
        }
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        let mut new_frontend = old_frontend.clone();
        new_frontend.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMemberScaledOut")],
        )]);
        let plan = DeploymentPlan { services: vec![new_frontend.clone(), backend], ..old_plan };
        s.store.submit("inst-1", &plan.to_json().unwrap(), "{}", "did:key:owner", 0).unwrap();

        let identity = Identity::generate().unwrap();
        let client = Arc::new(SyneroymClient::new_with_identity(
            "did:key:zEdge1".to_string(),
            String::new(),
            identity,
        ));
        let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);
        let instance_id = AppInstanceId::new("inst-1");
        let needs_work: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();

        s.apply_write_phase(WritePhase {
            instance_id: &instance_id,
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &needs_work,
            restart_candidates: &[],
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            // No alias for frontend's DID this pass -- the push fails.
            push_candidates: &[(new_frontend, "did:key:zEdge1".to_string())],
            schedule_decisions: &[],
            did_to_alias: &BTreeMap::new(),
            clients: &clients,
            now: 0,
        })
        .await;

        let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
        assert_eq!(
            latest.state,
            DeploymentState::Degraded,
            "the redeploy landed (vacuously, backend was revoked and filtered out) but the push \
             did not -- the record must not read as this instance's converged baseline: {latest:?}"
        );

        let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
        let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
        let (redeploy_exclusions, _) =
            SupervisorService::classify_update_actions(&landed, &diff.actions);
        assert!(
            redeploy_exclusions.contains("inst-1/frontend#0"),
            "the next pass must still classify frontend as a push candidate: {diff:?}"
        );
    }

    /// The companion negative case: a push failing in a pass where nothing
    /// was journaled (`needs_work` empty, so `apply_with_clients` is never
    /// called) must not touch an unrelated, already-`Active` record left
    /// by an earlier pass.
    #[tokio::test]
    async fn a_failing_push_with_no_redeploy_in_the_same_pass_does_not_touch_an_unrelated_active_record()
     {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "frontend", Some("edge-1"));
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.journal.append(&plan, DeploymentState::Active).unwrap();
        let svc = dependent_service("frontend", "backend");
        let instance_id = AppInstanceId::new("inst-1");

        s.apply_write_phase(WritePhase {
            instance_id: &instance_id,
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &BTreeSet::new(),
            restart_candidates: &[],
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            push_candidates: &[(svc, "did:key:zEdge1".to_string())],
            schedule_decisions: &[],
            did_to_alias: &BTreeMap::new(),
            clients: &BTreeMap::new(),
            now: 0,
        })
        .await;

        let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
        assert_eq!(
            latest.state,
            DeploymentState::Active,
            "nothing was journaled this pass -- the pre-existing record must be left alone: \
             {latest:?}"
        );
    }

    /// D-A5e-8/§33.19: `push_bindings` clears `BindingConflict` for that
    /// member once a later push lands cleanly -- the clear site this
    /// alert kind never had before A5e.
    #[tokio::test]
    async fn a_binding_conflict_clears_once_a_later_push_for_that_member_lands_cleanly() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::BindingConflict),
            "the failed push must raise the alert"
        );

        // The next push lands cleanly (the fake's default response).
        let mut opened = Vec::new();
        s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();
        assert!(
            !s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::BindingConflict),
            "a clean push must clear the alert it previously raised"
        );
    }

    /// D-A5e-8, ADR-0021 §5: an instance with an active `BindingConflict`
    /// reports `Degraded`; once the retried push lands and the alert
    /// clears, `handle_status` reports it recovered.
    #[tokio::test]
    async fn an_instance_leaves_degraded_once_the_retried_push_lands() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        s.store
            .alerts
            .raise(
                &instance_id,
                Some("inst-1/frontend#0"),
                None,
                "did:key:zEdge1",
                AlertKind::BindingConflict,
                "did not land",
            )
            .unwrap();
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::BindingConflict)
        );

        s.store
            .alerts
            .clear(
                &instance_id,
                Some("inst-1/frontend#0"),
                "did:key:zEdge1",
                AlertKind::BindingConflict,
            )
            .unwrap();
        assert!(
            !s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::BindingConflict),
            "the active alert set (what handle_status's overall_state reads) must be clear once \
             the conflict clears"
        );
    }

    /// M05A A5e §33.21/S2: the raise site must write the substrate's real
    /// DID into the alert's `substrate_did` column, not `svc.substrate`
    /// (an operator-chosen alias, empty on fallback placement) -- a clear
    /// keyed on the real DID would otherwise never match a row keyed on
    /// the alias, and `Degraded` would be permanent.
    #[tokio::test]
    async fn a_binding_conflict_is_raised_under_the_substrate_did_not_the_alias() {
        let s = service();
        let mut svc = dependent_service("frontend", "backend");
        // The fallback-placement case: no alias at all.
        svc.substrate = None;
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.push_bindings(&instance_id, &plan, &svc, "did:key:zRealNode", &dyn_actor, 0, &mut opened)
            .await
            .unwrap();

        let active = s.store.alerts.active(&instance_id).unwrap();
        let conflict =
            active.iter().find(|a| a.kind == AlertKind::BindingConflict).expect("{active:?}");
        assert_eq!(conflict.substrate_did, "did:key:zRealNode");

        // A clear keyed on that same real DID must now match the row.
        assert!(
            s.store
                .alerts
                .clear(
                    &instance_id,
                    Some("inst-1/frontend#0"),
                    "did:key:zRealNode",
                    AlertKind::BindingConflict,
                )
                .unwrap(),
            "the clear must match the row the raise actually wrote"
        );
    }

    /// D-A5e-2: the epoch is per dependent *member*, not per logical
    /// service -- two members of one scaled dependent must advance their
    /// own epoch independently.
    #[tokio::test]
    async fn two_members_of_one_dependent_advance_their_binding_epochs_independently() {
        let s = service();
        let mut frontend_1 = dependent_service("frontend", "backend");
        frontend_1.member_index = 1;
        frontend_1.service_id = ServiceId::new("did:key:hfrontend1");
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![dependent_service("frontend", "backend"), frontend_1.clone()],
        };
        let actor = Arc::new(BindingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        // Only member 1 is pushed this round.
        s.push_bindings(
            &instance_id,
            &plan,
            &frontend_1,
            "did:key:zEdge1",
            &dyn_actor,
            0,
            &mut opened,
        )
        .await
        .unwrap();

        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 0);
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#1").unwrap(), 1);
    }

    /// Review finding C-1: §23 test 45 (above) calls
    /// `binding_convergence_rows` directly, by its own doc comment --
    /// never through a real `status` response, so nothing pins the wire
    /// shape (`InstanceStatus.bindings` field name, its serialization) a
    /// caller actually reads. This drives the exact same declared
    /// dependency through `dispatch("status", …)` instead: no live
    /// substrate exists in this test, so `observed_epoch` is `None`
    /// rather than `Some(1)` (the exact case
    /// `a_dependent_that_does_not_answer_reports_unconverged_rather_than_absent`
    /// covers directly against the pure function below), but the point
    /// here is that the array arrives non-empty at all, through the real
    /// call.
    #[tokio::test]
    async fn status_returns_a_populated_bindings_array_over_a_real_dispatch_call() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc);
        s.store.submit("inst-1", &plan.to_json().unwrap(), "{}", "did:key:owner", 0).unwrap();
        s.store.advance_binding_epoch("inst-1", "inst-1/frontend#0").unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

        assert_eq!(status.bindings.len(), 1, "{:?}", status.bindings);
        assert_eq!(status.bindings[0].dependent_logical_ref, "inst-1/frontend#0");
        assert_eq!(status.bindings[0].dependency_name, "backend");
        assert_eq!(status.bindings[0].written_epoch, 1);
        assert_eq!(status.bindings[0].observed_epoch, None);
        assert!(!status.bindings[0].converged);
    }

    /// F7's negative half: a dependent absent from the sweep (unreachable,
    /// or never landed) must still produce a row -- `observed_epoch:
    /// None`, `converged: false` -- not silently vanish from the list, or
    /// an operator reading an empty table cannot tell "nothing declared"
    /// from "declared but not answering".
    #[tokio::test]
    async fn a_dependent_that_does_not_answer_reports_unconverged_rather_than_absent() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc);
        let _ = s.store.advance_binding_epoch("inst-1", "inst-1/frontend#0");

        let report = health::HealthReport { substrates: Vec::new(), services: Vec::new() };
        let rows = s.binding_convergence_rows("inst-1", &plan, &report);

        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].written_epoch, 1);
        assert_eq!(rows[0].observed_epoch, None);
        assert!(!rows[0].converged);
    }

    /// Matrix row 11: a push against a dependent that cannot be reached
    /// fails (the epoch has still advanced, D-A5c-4's own invariant --
    /// the next attempt must never retry at an epoch already spent), is
    /// visible on the operator read surface (review finding A-5: the
    /// original version of this test asserted only the epoch and a
    /// successful retry -- the state-visible half of the row's own
    /// wording -- and never checked `opened`/`alerts` at all), and
    /// succeeds cleanly once that dependent answers again. A unit test
    /// against a fake actor, deliberately (§23's own note: the wire path
    /// is already proven live by A5a's `binding_push_e2e.rs`, so this is
    /// entirely the supervisor's own control flow).
    #[tokio::test]
    async fn a_dependent_unreachable_during_a_push_leaves_the_instance_degraded_and_is_retried_when_it_next_answers()
     {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let actor = Arc::new(BindingActor::default());
        actor.responses.lock().unwrap().push(Err("substrate unreachable".to_string()));
        let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        let first = s
            .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await;
        assert!(first.is_err());
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
        let alerts = s.store.alerts.active(&instance_id).unwrap();
        assert!(alerts.iter().any(|a| a.kind == AlertKind::BindingConflict), "{alerts:?}");

        let second = s
            .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
            .await;
        assert_eq!(second.unwrap(), PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
        assert_eq!(
            s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
            2,
            "the retry must carry a fresh epoch, not reuse the one the failed attempt spent"
        );
    }

    /// M05B B1 review finding 1: `push_bindings` advances the binding
    /// epoch before every attempt, but a *durable* actor only enqueues
    /// once per key (`already_pending`) -- so a second transport failure
    /// for the same key while the first attempt's item is still queued
    /// must not advance the epoch again, or `written_epoch` races ahead of
    /// what the worker can ever actually deliver and a later successful
    /// delivery of the (older, still-queued) item would read as
    /// unconverged forever. Uses `deploy::build_durable_actor` directly
    /// (not `BindingActor`, which is never durable) so this exercises the
    /// same enqueue path `DurableActor::write_bindings` takes in
    /// production.
    #[tokio::test]
    async fn two_consecutive_transport_failures_for_one_key_do_not_strand_the_written_epoch() {
        let s = service();
        let svc = dependent_service("frontend", "backend");
        let plan = plan_with_one_dependent(svc.clone());
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();
        let outbox: Arc<dyn WriteBindingsOutbox> =
            Arc::new(SupervisorOutbox::new(s.store.queue.clone()));
        let queue_key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/frontend#0".to_string(),
            substrate_did: "did:key:zEdge1".to_string(),
        }
        .to_string();

        let first_client = Arc::new(FakeSubstrateClient::default());
        *first_client.write_bindings_outcome.lock().unwrap() =
            Some(Err("connection reset mid-write".to_string()));
        let first_actor = deploy::build_durable_actor(
            first_client,
            "did:key:zEdge1".to_string(),
            queue_key.clone(),
            outbox.clone(),
        );
        let first = s
            .push_bindings(
                &instance_id,
                &plan,
                &svc,
                "did:key:zEdge1",
                &first_actor,
                0,
                &mut opened,
            )
            .await;
        assert!(first.is_err());
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
        let queued = s.store.queue.all().unwrap();
        assert_eq!(queued.len(), 1, "the first failure must have enqueued exactly one item");

        // A second pass: reconnects fine (a fresh `DurableActor`), but the
        // write itself fails again for the same key -- while the first
        // failure's item is still sitting in the outbox.
        let second_client = Arc::new(FakeSubstrateClient::default());
        *second_client.write_bindings_outcome.lock().unwrap() =
            Some(Err("connection reset mid-write".to_string()));
        let second_actor = deploy::build_durable_actor(
            second_client,
            "did:key:zEdge1".to_string(),
            queue_key,
            outbox,
        );
        let second = s
            .push_bindings(
                &instance_id,
                &plan,
                &svc,
                "did:key:zEdge1",
                &second_actor,
                0,
                &mut opened,
            )
            .await;
        assert_eq!(
            second.unwrap(),
            PushOutcome::Deferred,
            "an already-pending key defers to the queue, it is not an error"
        );

        assert_eq!(
            s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
            1,
            "the epoch must not advance again while the first attempt's item is still queued"
        );
        let queued = s.store.queue.all().unwrap();
        assert_eq!(queued.len(), 1, "the second pass must not have enqueued a duplicate");
        let payload: outbox::QueuedBindingWrite =
            serde_json::from_slice(&queued[0].payload).unwrap();
        assert_eq!(
            payload.write.generation, 0,
            "the queued payload is still the first attempt's, carrying epoch 1"
        );

        // Once the worker eventually delivers the queued (epoch-1) item,
        // it must read as converged, matching what is actually in the
        // outbox -- not stranded behind a local epoch nothing will ever
        // deliver.
        let report = health::HealthReport {
            substrates: Vec::new(),
            services: vec![health::ServiceHealth {
                logical_ref: svc.logical_ref.clone(),
                service_id: svc.service_id.to_string(),
                alias: svc.substrate.clone(),
                substrate_did: "did:key:zEdge1".to_string(),
                signal: Signal::Healthy,
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
                binding_epochs: vec![("backend".to_string(), 1)],
                member_index: svc.member_index,
            }],
        };
        let rows = s.binding_convergence_rows("inst-1", &plan, &report);
        assert!(
            rows[0].converged,
            "the epoch the worker will eventually deliver (1) must still be the one convergence \
             checks against: {rows:?}"
        );
    }

    // ── M05A A5d: unattended renewal, anchor refresh, revocation ─────────

    /// A fake substrate for the renewal path: answers `instance_identity`
    /// with a fixed, real ed25519 key (so a certificate minted over it is
    /// genuinely valid), records every `renew_cert`/`restart`, and can be
    /// told to fail either one.
    #[derive(Debug)]
    struct RenewalActor {
        instance_key: Identity,
        instance_identity_error: Option<String>,
        renew_error: Option<String>,
        restart_error: Option<String>,
        renewed: Mutex<Vec<(String, u64, String)>>,
        restarted: Mutex<Vec<String>>,
    }

    impl Default for RenewalActor {
        fn default() -> Self {
            Self {
                instance_key: Identity::generate().unwrap(),
                instance_identity_error: None,
                renew_error: None,
                restart_error: None,
                renewed: Mutex::new(Vec::new()),
                restarted: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SubstrateActor for RenewalActor {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by renewal tests")
        }

        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("not exercised by renewal tests")
        }

        async fn restart(&self, service_id: String, _generation: u64) -> Result<(), String> {
            if let Some(e) = &self.restart_error {
                return Err(e.clone());
            }
            self.restarted.lock().unwrap().push(service_id);
            Ok(())
        }

        async fn renew_cert(
            &self,
            service_id: String,
            generation: u64,
            instance_certificate: String,
        ) -> Result<(), String> {
            if let Some(e) = &self.renew_error {
                return Err(e.clone());
            }
            self.renewed.lock().unwrap().push((service_id, generation, instance_certificate));
            Ok(())
        }

        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            if let Some(e) = &self.instance_identity_error {
                return Err(e.clone());
            }
            Ok(syneroym_sdk::InstanceIdentity {
                instance_did: substrate::derive_did_key(&self.instance_key.public_key()),
                pubkey_hex: hex::encode(self.instance_key.public_key().to_bytes()),
                installed_temporary_did: None,
            })
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by renewal tests")
        }
    }

    /// An `AnchorWriter` that records what it was asked to publish and can
    /// be made to fail, so both the schedule and the revocation write are
    /// testable with no registry.
    #[derive(Debug, Default)]
    struct RecordingAnchorWriter {
        refreshed: Mutex<Vec<String>>,
        revoked: Mutex<Vec<(String, String)>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl AnchorWriter for RecordingAnchorWriter {
        async fn refresh(&self, master: &Identity) -> Result<(), String> {
            if self.fail {
                return Err("registry unreachable".to_string());
            }
            self.refreshed.lock().unwrap().push(substrate::derive_did_key(&master.public_key()));
            Ok(())
        }

        async fn revoke_instance(
            &self,
            master: &Identity,
            instance_did: &str,
        ) -> Result<(), String> {
            if self.fail {
                return Err("registry unreachable".to_string());
            }
            self.revoked
                .lock()
                .unwrap()
                .push((substrate::derive_did_key(&master.public_key()), instance_did.to_string()));
            Ok(())
        }
    }

    /// A `Tier1Writer` that records what it was asked to publish and can be
    /// made to fail, the same shape `RecordingAnchorWriter` above uses.
    /// `calls` counts every attempt, successful or not -- `published` only
    /// grows on success, so a test proving a failing writer is still
    /// retried needs the former.
    #[derive(Debug, Default)]
    struct RecordingTier1Writer {
        published: Mutex<Vec<SignedEndpointInfo>>,
        calls: Mutex<u32>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl Tier1Writer for RecordingTier1Writer {
        async fn publish(&self, record: &SignedEndpointInfo) -> Result<(), String> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                return Err("registry unreachable".to_string());
            }
            self.published.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    /// A member with a real master in the supervisor's vault, under the
    /// same computable name `mint_and_substitute` would have stored it as
    /// -- so the renewal path finds it exactly the way production does.
    async fn seeded_member(s: &SupervisorService, service_name: &str) -> String {
        let master = s
            .vault
            .get_or_mint(&format!("member-inst-1#{service_name}-0"), keys::MasterKind::Member)
            .await
            .unwrap();
        substrate::derive_did_key(&master.public_key())
    }

    /// A plan naming one placed member by its real master DID, with the
    /// given rotation policy.
    fn plan_json_with_master(
        service_name: &str,
        master_did: &str,
        rotation_policy: &str,
    ) -> String {
        serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": master_did,
                "logical_ref": format!("inst-1/{service_name}"),
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": rotation_policy,
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            }]
        })
        .to_string()
    }

    /// One member's health, carrying a certificate window. `elapsed_ratio`
    /// is how far through its lifetime the certificate is at `NOW`.
    const NOW: u64 = 1_000_000;

    fn health_with_cert(
        service_name: &str,
        service_id: &str,
        substrate_did: &str,
        issued_at: u64,
        expires_at: u64,
    ) -> health::ServiceHealth {
        health::ServiceHealth {
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(service_name),
            },
            service_id: service_id.to_string(),
            alias: Some(SubstrateAlias::new("edge-1")),
            substrate_did: substrate_did.to_string(),
            signal: Signal::Healthy,
            instance_certificate_issued_at: Some(issued_at),
            instance_certificate_expires_at: Some(expires_at),
            binding_epochs: Vec::new(),
            member_index: 0,
        }
    }

    /// 90% through a 4-hour lifetime: inside `is_near_expiry_parts`'s
    /// 25%-remaining window.
    fn near_expiry_health(service_name: &str, service_id: &str) -> health::ServiceHealth {
        health_with_cert(service_name, service_id, "did:key:zEdge1", NOW - 12_960, NOW + 1_440)
    }

    /// Freshly issued: 0% elapsed, comfortably outside the window.
    fn fresh_health(service_name: &str, service_id: &str) -> health::ServiceHealth {
        health_with_cert(service_name, service_id, "did:key:zEdge1", NOW, NOW + 14_400)
    }

    fn report_of(services: Vec<health::ServiceHealth>) -> health::HealthReport {
        health::HealthReport { substrates: Vec::new(), services }
    }

    fn edge_1_actor(actor: Arc<RenewalActor>) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
        BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))])
    }

    fn edge_1_alias() -> BTreeMap<String, String> {
        BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())])
    }

    #[tokio::test]
    async fn a_pass_renews_a_member_within_the_near_expiry_window() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();
        let report = report_of(vec![near_expiry_health("backend", &master_did)]);

        let candidates = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        assert_eq!(candidates.len(), 1, "{candidates:?}");

        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();
        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &candidates,
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            7,
            NOW,
            &mut opened,
        )
        .await;

        let renewed = actor.renewed.lock().unwrap();
        assert_eq!(renewed.len(), 1, "the member must have had a certificate installed");
        assert_eq!(renewed[0].0, master_did);
        assert_eq!(renewed[0].1, 7, "the install must carry this supervisor's generation");
        let cert = DelegationCertificate::from_json(&renewed[0].2).unwrap();
        assert_eq!(cert.master_did, master_did);
        assert!(opened.is_empty(), "a successful renewal raises no alert: {opened:?}");
    }

    #[tokio::test]
    async fn a_pass_does_not_renew_a_member_outside_the_near_expiry_window() {
        let master_did = "did:key:hBackend";
        let report = report_of(vec![fresh_health("backend", master_did)]);
        let candidates = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        assert!(candidates.is_empty(), "{candidates:?}");
    }

    /// D-A5d-12: a service already in `needs_work` is about to be
    /// re-certified by `apply_plan` this same pass, so renewing it here
    /// would mint it a second certificate for no reason.
    #[test]
    fn a_member_in_needs_work_is_not_also_renewed_this_pass() {
        let report = report_of(vec![near_expiry_health("backend", "did:key:hBackend")]);
        let needs_work = BTreeSet::from(["inst-1/backend#0".to_string()]);
        let candidates =
            SupervisorService::renewal_candidates(&report, &needs_work, &BTreeSet::new(), NOW, 5);
        assert!(candidates.is_empty(), "{candidates:?}");
    }

    /// The other half of D-A5d-12: a restart reloads the running instance
    /// and touches no certificate, so a member under remediation still
    /// needs its own, independent renewal check. `restart_candidates` is
    /// therefore not an input to this decision at all.
    #[test]
    fn a_member_under_restart_remediation_is_still_checked_for_renewal() {
        let mut unhealthy = near_expiry_health("backend", "did:key:hBackend");
        unhealthy.signal = Signal::InstanceNotRunning("down".to_string());
        let report = report_of(vec![unhealthy]);
        let candidates = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        assert_eq!(candidates.len(), 1, "{candidates:?}");
    }

    /// D-A5d-4: a locked vault skips the renewal work-list and nothing
    /// else -- health, remediation, and the anchor check all continue,
    /// since none of them opens the vault.
    #[tokio::test]
    async fn a_locked_vault_skips_renewal_but_not_health_or_remediation_this_pass() {
        let s = service_with_locked_vault();
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            "did:key:hBackend",
            "none",
        ))
        .unwrap();
        let candidates = SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", "did:key:hBackend")]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &candidates,
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert!(
            actor.renewed.lock().unwrap().is_empty(),
            "a locked vault must not reach the substrate at all"
        );
        assert_eq!(opened, vec![(AlertKind::VaultLocked, "inst-1/backend#0".to_string())]);

        // The rest of the pass is unaffected: a restart candidate on the
        // same instance still records its attempt.
        let restart_actor = Arc::new(CountingActor::default());
        let dyn_actor: Arc<dyn SubstrateActor> = restart_actor.clone();
        let mut opened2 = Vec::new();
        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            NOW,
            &mut opened2,
        )
        .await;
        assert_eq!(*restart_actor.restart_calls.lock().unwrap(), 1);
    }

    /// One root cause, one row per affected member -- the same fan-out
    /// `SubstrateUnreachable` already uses, and what an operator reading
    /// `alerts <instance>` needs to see.
    #[tokio::test]
    async fn a_locked_vault_raises_vault_locked_for_every_near_expiry_member() {
        let s = service_with_locked_vault();
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            "did:key:hBackend",
            "none",
        ))
        .unwrap();
        let report = report_of(vec![
            near_expiry_health("backend", "did:key:hBackend"),
            near_expiry_health("frontend", "did:key:hFrontend"),
            fresh_health("worker", "did:key:hWorker"),
        ]);
        let candidates = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &candidates,
            &edge_1_alias(),
            &edge_1_actor(Arc::new(RenewalActor::default())),
            0,
            NOW,
            &mut opened,
        )
        .await;

        let instance_id = AppInstanceId::new("inst-1");
        let locked: Vec<_> = s
            .store
            .alerts
            .active(&instance_id)
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == AlertKind::VaultLocked)
            .collect();
        assert_eq!(locked.len(), 2, "one row per affected member, not one per fact: {locked:?}");
        let refs: BTreeSet<_> = locked.iter().filter_map(|a| a.logical_ref.clone()).collect();
        assert_eq!(
            refs,
            BTreeSet::from(["inst-1/backend#0".to_string(), "inst-1/frontend#0".to_string()]),
            "the member whose certificate is nowhere near expiry must not be alerted on"
        );
        assert!(
            locked.iter().all(|a| a.detail.contains("inject-kek")),
            "the alert must name the operator action that fixes it"
        );
    }

    /// D-A5d-6: `RotationPolicy` is read from the supervisor's own stored
    /// plan, after the new certificate has installed successfully. The
    /// substrate never sees it.
    #[tokio::test]
    async fn restart_on_rotation_follows_a_successful_install_with_a_restart_call() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert_eq!(actor.renewed.lock().unwrap().len(), 1);
        assert_eq!(*actor.restarted.lock().unwrap(), vec![master_did]);
    }

    #[tokio::test]
    async fn rotation_policy_none_installs_without_restarting() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();
        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert_eq!(actor.renewed.lock().unwrap().len(), 1);
        assert!(actor.restarted.lock().unwrap().is_empty());
    }

    /// D-A5d-13, first step: a mint that fails must not go on to install
    /// or restart. `CertificateNearExpiry` names the step, and the member
    /// is retried next pass rather than failing the whole instance.
    #[tokio::test]
    async fn a_failed_mint_does_not_attempt_install_or_restart_for_that_member() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let actor = Arc::new(RenewalActor {
            instance_identity_error: Some("substrate refused the identity query".to_string()),
            ..RenewalActor::default()
        });
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert!(actor.renewed.lock().unwrap().is_empty());
        assert!(actor.restarted.lock().unwrap().is_empty());
        assert_eq!(
            opened,
            vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]
        );
        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(alerts.iter().any(|a| a.detail.contains("mint")), "{alerts:?}");
    }

    /// D-A5d-13, second step: restarting a service whose new certificate
    /// never landed serves nothing and spends a lifecycle action for no
    /// gain.
    #[tokio::test]
    async fn a_failed_install_does_not_attempt_restart_for_that_member() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let actor = Arc::new(RenewalActor {
            renew_error: Some("substrate unreachable".to_string()),
            ..RenewalActor::default()
        });
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert!(actor.restarted.lock().unwrap().is_empty());
        assert_eq!(
            opened,
            vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]
        );
        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(alerts.iter().any(|a| a.detail.contains("install")), "{alerts:?}");
    }

    /// Mint and install both landed, only the
    /// `restart-on-rotation` restart failed. This must not be reported as
    /// a stalled renewal (the certificate is fine, and the very next
    /// health poll would clear that kind out from under the real
    /// problem) -- it gets its own alert kind and a persisted marker that
    /// survives the certificate's own alert lifecycle.
    #[tokio::test]
    async fn a_failed_rotation_restart_raises_its_own_alert_and_is_not_cleared_by_a_fresh_cert() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let actor = Arc::new(RenewalActor {
            restart_error: Some("substrate refused the restart".to_string()),
            ..RenewalActor::default()
        });
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        // The certificate itself landed.
        assert_eq!(actor.renewed.lock().unwrap().len(), 1);
        assert_eq!(
            opened,
            vec![(AlertKind::RotationRestartPending, "inst-1/backend#0".to_string())]
        );
        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(
            !alerts.iter().any(|a| a.kind == AlertKind::CertificateNearExpiry),
            "a landed renewal must not also read as a stalled one: {alerts:?}"
        );
        assert!(
            s.store.pending_rotation_restarts("inst-1").unwrap().contains("inst-1/backend#0"),
            "the owed restart must be persisted, not just alerted"
        );

        // A fresh certificate window alone must not clear it.
        s.clear_settled_renewal_alerts(
            &AppInstanceId::new("inst-1"),
            &report_of(vec![fresh_health("backend", &master_did)]),
            NOW,
        );
        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(
            alerts.iter().any(|a| a.kind == AlertKind::RotationRestartPending),
            "only a successful retry clears it: {alerts:?}"
        );
    }

    /// The retry half of the fix: once persisted, the owed restart is
    /// retried on a later pass, independent of the renewal work-list that
    /// no longer names this member (its certificate is no longer near
    /// expiry) -- `retry_pending_rotation_restarts` is `renew_due_members`'
    /// own sibling call inside `apply_write_phase`, tested the same
    /// direct way (see the "wiring" test below for the
    /// `plan -> did_to_alias -> clients` lookup itself).
    #[tokio::test]
    async fn a_pending_rotation_restart_is_retried_and_cleared_on_success() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend#0", NOW as i64).unwrap();
        s.store
            .alerts
            .raise(
                &instance_id,
                Some("inst-1/backend#0"),
                None,
                "did:key:zEdge1",
                AlertKind::RotationRestartPending,
                "owed",
            )
            .unwrap();
        let actor = Arc::new(RenewalActor::default());
        let pending: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();
        let mut opened = Vec::new();

        s.retry_pending_rotation_restarts(
            &instance_id,
            "inst-1",
            &plan,
            &pending,
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            &mut opened,
        )
        .await;

        assert_eq!(actor.restarted.lock().unwrap().len(), 1);
        assert!(s.store.pending_rotation_restarts("inst-1").unwrap().is_empty());
        assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
    }

    /// The failure half: a still-failing restart leaves the marker in
    /// place for the next pass, rather than clearing it or forgetting it.
    #[tokio::test]
    async fn a_still_failing_rotation_restart_stays_pending() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            &master_did,
            "restart-on-rotation",
        ))
        .unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend#0", NOW as i64).unwrap();
        let actor = Arc::new(RenewalActor {
            restart_error: Some("still refusing".to_string()),
            ..RenewalActor::default()
        });
        let pending: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();
        let mut opened = Vec::new();

        s.retry_pending_rotation_restarts(
            &instance_id,
            "inst-1",
            &plan,
            &pending,
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            &mut opened,
        )
        .await;

        assert!(s.store.pending_rotation_restarts("inst-1").unwrap().contains("inst-1/backend#0"));
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::RotationRestartPending)
        );
    }

    /// A member dropped from the plan by a
    /// resubmit is never reached by this loop again -- it is keyed off
    /// `plan.services` -- so unlike an unreachable-this-pass member,
    /// there is no future retry to defer to. Leaving the marker and the
    /// alert in place would make both permanent, with nothing left that
    /// could ever clear either.
    #[tokio::test]
    async fn a_member_dropped_from_the_plan_has_its_owed_restart_and_alert_cleared() {
        let s = service();
        // A plan that no longer names `inst-1/backend` at all.
        let plan = DeploymentPlan::from_json(&plan_json_no_services("inst-1")).unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend", NOW as i64).unwrap();
        s.store
            .alerts
            .raise(
                &instance_id,
                Some("inst-1/backend"),
                None,
                "did:key:zEdge1",
                AlertKind::RotationRestartPending,
                "owed",
            )
            .unwrap();
        let pending: BTreeSet<String> = ["inst-1/backend".to_string()].into_iter().collect();
        let mut opened = Vec::new();

        s.retry_pending_rotation_restarts(
            &instance_id,
            "inst-1",
            &plan,
            &pending,
            &BTreeMap::new(),
            &BTreeMap::new(),
            0,
            &mut opened,
        )
        .await;

        assert!(s.store.pending_rotation_restarts("inst-1").unwrap().is_empty());
        assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
    }

    /// D-A5d-9's clearing rule: raised alerts with no path back to cleared
    /// are exactly the bug §19.20 exists to prevent. Recomputed from the
    /// substrate's own answer, not tracked as a flag -- so a renewal that
    /// succeeded out of band clears these too.
    #[tokio::test]
    async fn certificate_near_expiry_clears_on_the_next_passs_healthy_read() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        for kind in [
            AlertKind::CertificateNearExpiry,
            AlertKind::CertificateExpired,
            AlertKind::VaultLocked,
        ] {
            s.store
                .alerts
                .raise(
                    &instance_id,
                    Some("inst-1/backend#0"),
                    None,
                    "did:key:zEdge1",
                    kind,
                    "stalled",
                )
                .unwrap();
        }

        // Still near expiry: nothing clears.
        s.clear_settled_renewal_alerts(
            &instance_id,
            &report_of(vec![near_expiry_health("backend", "did:key:hBackend")]),
            NOW,
        );
        assert_eq!(s.store.alerts.active(&instance_id).unwrap().len(), 3);

        // A healthy certificate window clears all three at once.
        s.clear_settled_renewal_alerts(
            &instance_id,
            &report_of(vec![fresh_health("backend", "did:key:hBackend")]),
            NOW,
        );
        assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
    }

    /// D-A5d-16: the supervisor mints at its own, short lifetime -- not
    /// the attended posture's 24-hour deploy default, which serves an
    /// operator with no renewal loop behind them.
    #[tokio::test]
    async fn renewal_mints_at_renewed_cert_expires_hours_not_the_deploy_default() {
        let s = service();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();
        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &SupervisorService::renewal_candidates(
                &report_of(vec![near_expiry_health("backend", &master_did)]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                NOW,
                5,
            ),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        let renewed = actor.renewed.lock().unwrap();
        let cert = DelegationCertificate::from_json(&renewed[0].2).unwrap();
        let lifetime = cert.expires_at_secs - cert.issued_at_secs;
        assert_eq!(lifetime, s.renewed_cert_expires_hours * 3600);
        assert!(
            lifetime < deploy::DEFAULT_INSTANCE_CERT_EXPIRES_HOURS * 3600,
            "a renewed certificate must be strictly shorter-lived than the attended default"
        );
    }

    /// D-A5d-17: the same root cause must surface under the same alert
    /// kind whichever of the two checks catches it. This is the defensive
    /// per-call path -- `kek_is_loaded` said unlocked, and the vault read
    /// itself then failed -- distinct from the up-front check's own test
    /// above.
    #[tokio::test]
    async fn a_vault_error_locked_race_during_mint_raises_vault_locked_not_certificate_near_expiry()
    {
        // An encrypted vault, open at construction so the cheap up-front
        // check passes, then closed again before the mint -- the exact
        // ordering the carve-out exists for, produced directly rather than
        // raced.
        let (s, key_store) =
            Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
                .build_with_key_store();
        assert!(s.vault.kek_is_loaded());

        let plan = DeploymentPlan::from_json(&plan_json_with_master(
            "backend",
            "did:key:hBackend",
            "none",
        ))
        .unwrap();
        let candidate = RenewalCandidate {
            member_ref: "inst-1/backend#0".to_string(),
            service_name: "backend".to_string(),
            service_id: "did:key:hBackend".to_string(),
            substrate_did: "did:key:zEdge1".to_string(),
            expires_at: NOW + 1_440,
            member_index: 0,
        };
        let actor = Arc::new(RenewalActor::default());
        let mut opened = Vec::new();
        key_store.clear_kek();

        s.renew_due_members(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            std::slice::from_ref(&candidate),
            &edge_1_alias(),
            &edge_1_actor(actor.clone()),
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert!(actor.renewed.lock().unwrap().is_empty());
        assert_eq!(
            opened,
            vec![(AlertKind::VaultLocked, "inst-1/backend#0".to_string())],
            "a vault lock found mid-mint is the same condition as one found up front, and must \
             not surface under a different alert kind"
        );
    }

    /// D-A5d-21: renewal is the one work-list whose arrivals are
    /// correlated by construction -- every member of an instance is minted
    /// in the same call at the same lifetime, so a whole instance reaches
    /// its near-expiry window in the same pass, every cycle. The cap
    /// bounds how long one pass holds the instance lock; the remainder
    /// rolls to the next pass, recomputed from live health data rather
    /// than queued.
    #[test]
    fn a_pass_renews_at_most_max_renewals_per_pass_candidates_and_defers_the_rest() {
        let names = ["a", "b", "c", "d", "e", "f", "g"];
        let report = report_of(
            names
                .iter()
                .map(|n| near_expiry_health(n, &format!("did:key:h{n}")))
                .collect::<Vec<_>>(),
        );

        let first = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        assert_eq!(first.len(), 5, "the cap must hold: {first:?}");

        // The next pass recomputes from live health. The five that landed
        // now report fresh certificates; the deferred two are still due
        // and are picked up.
        let taken: BTreeSet<String> = first.iter().map(|c| c.member_ref.clone()).collect();
        let next_report = report_of(
            names
                .iter()
                .map(|n| {
                    let id = format!("did:key:h{n}");
                    if taken.contains(&format!("inst-1/{n}#0")) {
                        fresh_health(n, &id)
                    } else {
                        near_expiry_health(n, &id)
                    }
                })
                .collect::<Vec<_>>(),
        );
        let second = SupervisorService::renewal_candidates(
            &next_report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        );
        let deferred: BTreeSet<String> = second.iter().map(|c| c.member_ref.clone()).collect();
        assert_eq!(deferred.len(), 2, "{second:?}");
        assert!(deferred.is_disjoint(&taken));
    }

    /// Without a sort, report order alone decided who kept
    /// the cap's slots, which a persistently-failing member (still
    /// near-expiry every pass, since its renewal never lands) could hold
    /// forever if it happened to sort first -- starving every member past
    /// the cap even though they are genuinely more urgent. `b` is one
    /// second from expiring; `a`, `c`, and `d` have a full hour left, but
    /// `a` sorts first alphabetically. The cap must still pick `b`.
    #[test]
    fn the_cap_keeps_the_most_urgent_candidates_not_whichever_sort_first() {
        // Same 4-hour lifetime `near_expiry_health`/`fresh_health` use;
        // only how much of it remains differs per member. 3,600s
        // remaining sits exactly on the 25%-of-lifetime boundary (still
        // near-expiry, inclusive); 1s remaining is far past it.
        let report = report_of(vec![
            health_with_cert("a", "did:key:ha", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
            health_with_cert("b", "did:key:hb", "did:key:zEdge1", NOW - 14_399, NOW + 1),
            health_with_cert("c", "did:key:hc", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
            health_with_cert("d", "did:key:hd", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
        ]);

        let candidates = SupervisorService::renewal_candidates(
            &report,
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            1,
        );

        assert_eq!(
            candidates.iter().map(|c| c.member_ref.as_str()).collect::<Vec<_>>(),
            vec!["inst-1/b#0"],
            "the one slot must go to the member closest to expiring: {candidates:?}"
        );
    }

    /// `.take(0)` silently disables renewal for the whole
    /// node, with no warning and nothing rejecting the config. The
    /// existing config-level test only pins the *default* at 1, which
    /// says nothing about a configured 0 -- clamped at construction
    /// instead, so every caller gets the guard regardless of how it built
    /// the config.
    #[test]
    fn a_configured_zero_max_renewals_per_pass_is_clamped_to_one() {
        let s = Fixture { max_renewals_per_pass: Some(0), ..Fixture::default() }.build();
        assert_eq!(s.max_renewals_per_pass, 1);
    }

    /// A `0` here would make every signed topology document born expired.
    #[test]
    fn a_configured_zero_topology_document_not_after_secs_is_clamped() {
        let s = Fixture { topology_document_not_after_secs: Some(0), ..Fixture::default() }.build();
        assert_eq!(s.topology_document_not_after_secs, 3_600);
    }

    /// A `cache_ttl` at or above half of `not_after` breaks the property
    /// that a served copy always outlives the caller's own cache TTL.
    #[test]
    fn a_cache_ttl_at_least_half_of_not_after_is_clamped() {
        let s = Fixture {
            topology_document_not_after_secs: Some(100),
            topology_document_cache_ttl_secs: Some(50),
            ..Fixture::default()
        }
        .build();
        assert_eq!(s.topology_document_cache_ttl_secs, 25);
    }

    /// `not_after_secs / 4` alone would clamp to `0` for any `not_after`
    /// under 4 seconds, and a reader that takes `0` as its cache TTL gets
    /// `Duration::ZERO`, which never registers a cache hit at all.
    #[test]
    fn the_cache_ttl_clamp_never_produces_zero() {
        let s = Fixture {
            topology_document_not_after_secs: Some(2),
            topology_document_cache_ttl_secs: Some(2),
            ..Fixture::default()
        }
        .build();
        assert_eq!(s.topology_document_cache_ttl_secs, 1);
    }

    // ── Phase 4: master-anchor refresh on the existing tick ──────────────

    #[tokio::test]
    async fn master_anchor_refresh_is_skipped_when_not_yet_overdue() {
        let writer = Arc::new(RecordingAnchorWriter::default());
        let s = Fixture {
            anchor_writer: Some(writer.clone()),
            master_anchor_refresh_interval_secs: Some(43_200),
            ..Fixture::default()
        }
        .build();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();
        s.store.record_master_anchor_refresh(&master_did, NOW as i64 - 100).unwrap();

        s.refresh_due_master_anchors(&plan, NOW).await;

        assert!(writer.refreshed.lock().unwrap().is_empty());
        assert_eq!(
            s.store.last_master_anchor_refresh(&master_did).unwrap(),
            Some(NOW as i64 - 100),
            "a skipped refresh must not move the stamp"
        );
    }

    #[tokio::test]
    async fn master_anchor_refresh_fires_once_the_interval_elapses() {
        let writer = Arc::new(RecordingAnchorWriter::default());
        let s = Fixture {
            anchor_writer: Some(writer.clone()),
            master_anchor_refresh_interval_secs: Some(43_200),
            ..Fixture::default()
        }
        .build();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();
        s.store.record_master_anchor_refresh(&master_did, NOW as i64 - 43_201).unwrap();

        s.refresh_due_master_anchors(&plan, NOW).await;

        assert_eq!(*writer.refreshed.lock().unwrap(), vec![master_did]);
    }

    /// M05A A5e §33.5/D-A5e-5, test 63: `refresh_due_master_anchors` reads
    /// `svc.member_index` to resolve which master to sign with
    /// (`keys::master_for_member`) -- a regression to a hardcoded `0`
    /// would silently sign member 1's anchor with member 0's key instead
    /// of failing loudly, the fail-closed outage the plan calls this
    /// slice's single most consequential fix. Asserts the *key the writer
    /// actually received*, not merely that a call happened.
    #[tokio::test]
    async fn master_anchor_refresh_republishes_each_members_own_anchor_and_stamps_its_own_row() {
        let writer = Arc::new(RecordingAnchorWriter::default());
        let s = Fixture { anchor_writer: Some(writer.clone()), ..Fixture::default() }.build();
        let master0 =
            s.vault.get_or_mint("member-inst-1#backend-0", keys::MasterKind::Member).await.unwrap();
        let master0_did = substrate::derive_did_key(&master0.public_key());
        let master1 =
            s.vault.get_or_mint("member-inst-1#backend-1", keys::MasterKind::Member).await.unwrap();
        let master1_did = substrate::derive_did_key(&master1.public_key());
        assert_ne!(master0_did, master1_did);

        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [
                {
                    "service_id": master0_did,
                    "logical_ref": "inst-1/backend",
                    "substrate": "edge-1",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "redundant",
                    "member_index": 0
                },
                {
                    "service_id": master1_did,
                    "logical_ref": "inst-1/backend",
                    "substrate": "edge-1",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": "redundant",
                    "member_index": 1
                }
            ]
        })
        .to_string();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();

        s.refresh_due_master_anchors(&plan, NOW).await;

        let refreshed = writer.refreshed.lock().unwrap();
        assert_eq!(
            BTreeSet::from_iter(refreshed.iter().cloned()),
            BTreeSet::from([master0_did.clone(), master1_did.clone()]),
            "each member's own anchor must be republished, signed with its own key: {refreshed:?}"
        );
        drop(refreshed);
        assert_eq!(s.store.last_master_anchor_refresh(&master0_did).unwrap(), Some(NOW as i64));
        assert_eq!(
            s.store.last_master_anchor_refresh(&master1_did).unwrap(),
            Some(NOW as i64),
            "member 1's own row must be stamped, not silently folded into member 0's"
        );
    }

    /// The stamp moves only on success: a failed publish must leave the
    /// previous one alone so the next pass retries rather than waiting out
    /// another whole interval.
    #[tokio::test]
    async fn master_anchor_refresh_updates_last_refreshed_at_on_success() {
        let s = Fixture {
            anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
            ..Fixture::default()
        }
        .build();
        let master_did = seeded_member(&s, "backend").await;
        let plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none"))
                .unwrap();

        // Never published before, so overdue on the first pass.
        assert_eq!(s.store.last_master_anchor_refresh(&master_did).unwrap(), None);
        s.refresh_due_master_anchors(&plan, NOW).await;
        assert_eq!(s.store.last_master_anchor_refresh(&master_did).unwrap(), Some(NOW as i64));

        let failing = Fixture {
            anchor_writer: Some(Arc::new(RecordingAnchorWriter {
                fail: true,
                ..RecordingAnchorWriter::default()
            })),
            ..Fixture::default()
        }
        .build();
        let failing_master = seeded_member(&failing, "backend").await;
        let failing_plan =
            DeploymentPlan::from_json(&plan_json_with_master("backend", &failing_master, "none"))
                .unwrap();
        failing.refresh_due_master_anchors(&failing_plan, NOW).await;
        assert_eq!(
            failing.store.last_master_anchor_refresh(&failing_master).unwrap(),
            None,
            "a failed publish must not be stamped as a success"
        );
    }

    // ── Tier 1: the app-DID registry record (ADR-0022 §2) ────────────────

    fn desired_state_with_app_master(app_instance_id: &str, app_master_did: &str) -> DesiredState {
        DesiredState {
            app_instance_id: app_instance_id.to_string(),
            plan_json: plan_json_no_services(app_instance_id),
            inventory_json: "{}".to_string(),
            owner_did: "did:key:zOwner".to_string(),
            generation: 0,
            paused: false,
            retired: false,
            submitted_at: 0,
            updated_at: 0,
            app_master_did: app_master_did.to_string(),
        }
    }

    /// D-S1-6, both halves: a row with no app master DID yet (D-A7-7,
    /// before this instance's first `adopt` under A7) is skipped without
    /// even opening the vault, and nothing gets minted to fill the gap.
    #[tokio::test]
    async fn an_instance_with_no_app_master_did_is_skipped_and_nothing_is_minted() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
        let state = desired_state_with_app_master("inst-1", "");
        let mut opened = Vec::new();

        s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened)
            .await;

        assert!(
            writer.published.lock().unwrap().is_empty(),
            "no app master DID recorded must never reach the writer"
        );
        assert!(
            s.vault.get("app-inst-1").await.unwrap().is_none(),
            "the skip must never mint an app master"
        );
        assert!(opened.is_empty());
    }

    /// D-S1-8: a supervisor with no registry configured holds no writer at
    /// all (mirroring `RegistryAnchorWriter::from_registry_client`), and a
    /// pass with none configured is a quiet no-op, never a panic. The
    /// `RegistryTier1Writer::from_registry_client` half of D-S1-8's own
    /// warning is asserted directly here; the log line itself is written
    /// once at supervisor init (`runtime.rs`), outside what a
    /// `SupervisorService` unit test can reach.
    #[tokio::test]
    async fn no_configured_registry_holds_no_writer_and_the_supervisor_keeps_running() {
        assert!(
            RegistryTier1Writer::from_registry_client(None).is_none(),
            "no substrate.registry_url must mean no writer, not one that quietly does nothing"
        );

        let s = Fixture::default().build();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        let state = desired_state_with_app_master("inst-1", &app_did);
        let mut opened = Vec::new();

        s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened)
            .await;

        assert_eq!(
            s.store.last_tier1_refresh(&app_did).unwrap(),
            None,
            "with no writer configured there is nothing to stamp"
        );
    }

    /// D-S1-5's real property is not the interval, it is that a failure
    /// never gives up: every pass this many seconds after the last
    /// success retries again, with nothing here counting attempts toward
    /// a cap. `tier1_refresh_survives_sixty_consecutive_failures_against_
    /// the_default_interval` (`syneroym-core`) pins the number this
    /// buys against `EndpointInfo`'s 30-day `not_after`.
    #[tokio::test]
    async fn a_failed_tier1_publish_is_retried_every_interval_with_no_internal_cap() {
        let failing =
            Arc::new(RecordingTier1Writer { fail: true, ..RecordingTier1Writer::default() });
        let s = Fixture {
            tier1_writer: Some(failing.clone()),
            master_anchor_refresh_interval_secs: Some(43_200),
            ..Fixture::default()
        }
        .build();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        let state = desired_state_with_app_master("inst-1", &app_did);
        let instance_id = AppInstanceId::new("inst-1");

        for tick in 0..3u64 {
            let mut opened = Vec::new();
            s.refresh_due_app_tier1_record(&instance_id, &state, NOW + tick * 43_200, &mut opened)
                .await;
        }

        assert_eq!(
            *failing.calls.lock().unwrap(),
            3,
            "every interval-elapsed pass must retry -- nothing here counts attempts and stops"
        );
        assert_eq!(
            s.store.last_tier1_refresh(&app_did).unwrap(),
            None,
            "a failed publish is never stamped as a success"
        );
    }

    /// The gate itself, exercised across a real success: the failure-only
    /// test above cannot regress this property, since a stamp that never
    /// advances reads as "due" on every single tick regardless of what
    /// the interval comparison does.
    #[tokio::test]
    async fn the_interval_gate_holds_after_a_successful_publish() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture {
            tier1_writer: Some(writer.clone()),
            master_anchor_refresh_interval_secs: Some(43_200),
            ..Fixture::default()
        }
        .build();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        let state = desired_state_with_app_master("inst-1", &app_did);
        let instance_id = AppInstanceId::new("inst-1");

        let mut opened = Vec::new();
        s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;
        assert_eq!(writer.published.lock().unwrap().len(), 1);

        s.refresh_due_app_tier1_record(&instance_id, &state, NOW + 100, &mut opened).await;
        assert_eq!(
            writer.published.lock().unwrap().len(),
            1,
            "a tick inside the interval must not re-publish"
        );

        s.refresh_due_app_tier1_record(&instance_id, &state, NOW + 43_200, &mut opened).await;
        assert_eq!(
            writer.published.lock().unwrap().len(),
            2,
            "a tick past the interval must publish again"
        );
    }

    /// The evaluation happens on the ordinary per-instance pass, against a
    /// persisted fact -- not on a timer of its own -- so a pass with
    /// otherwise nothing to do still reaches it purely because a
    /// `tier1_writer` is configured (the same gate `anchor_writer` uses).
    /// The published record also carries this pass's real generation, not
    /// a hardcoded one -- `generation` is the entire mechanism behind
    /// failure-matrix row 2 (two supervisors, one app DID).
    #[tokio::test]
    async fn a_refresh_runs_on_the_ordinary_pass_tick_and_carries_the_instances_generation() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0)
            .unwrap();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        s.store.record_adopt("inst-1", 7, &app_did).unwrap();

        s.reconcile_instance_pass("inst-1").await;

        let published = writer.published.lock().unwrap();
        assert_eq!(
            published.len(),
            1,
            "a configured writer must be exercised even when every other work list is empty"
        );
        assert_eq!(
            published[0].info.generation, 7,
            "the published record must carry this instance's real generation"
        );
        drop(published);
        assert!(
            s.store.last_tier1_refresh(&app_did).unwrap().is_some(),
            "a successful publish through the ordinary pass must stamp the fact"
        );
    }

    /// S1-8: the property that matters is one level up from the signer
    /// (`tier1::tests::a_locked_vault_fails_the_refresh_without_touching_
    /// the_registry`, which takes no writer at all and so is true by
    /// construction) -- with a writer actually configured, a locked vault
    /// must stop before that writer is ever reached, and must raise
    /// `VaultLocked` rather than only logging.
    #[tokio::test]
    async fn a_locked_vault_never_reaches_a_configured_tier1_writer() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture {
            locked_vault: true,
            tier1_writer: Some(writer.clone()),
            ..Fixture::default()
        }
        .build();
        let state = desired_state_with_app_master("inst-1", "did:key:zPlaceholderApp");
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

        assert_eq!(*writer.calls.lock().unwrap(), 0, "a locked vault must never reach the writer");
        assert_eq!(opened, vec![(AlertKind::VaultLocked, "inst-1".to_string())]);
    }

    /// The regression `kek_is_loaded()` cannot describe: on a node with
    /// `storage.encryption = false`, every vault read succeeds, but
    /// `kek_is_loaded()` -- a `KeyStore`-only check -- still answers
    /// `false`, since no KEK is ever injected on such a node. A pre-check
    /// on that answer (A5d's shape, which this refresh briefly copied)
    /// would skip this instance's Tier-1 publish forever and raise a
    /// `VaultLocked` alert that is never true. Reading the real attempt's
    /// own `VaultError::Locked` instead (A7's D-A7-1 shape) must reach the
    /// writer here, where the vault is merely unencrypted, not locked.
    #[tokio::test]
    async fn an_unencrypted_vault_with_no_kek_still_reaches_the_tier1_writer() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture {
            skip_kek_injection: true,
            tier1_writer: Some(writer.clone()),
            ..Fixture::default()
        }
        .build();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        let state = desired_state_with_app_master("inst-1", &app_did);
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

        assert_eq!(
            writer.published.lock().unwrap().len(),
            1,
            "an unencrypted vault with no KEK must not be treated as locked"
        );
        assert!(opened.is_empty(), "a genuinely reachable vault must raise no VaultLocked alert");
    }

    /// S1-3: the vault's own key must match the DID the row recorded at
    /// the last `adopt` -- a mismatch (an `import-master` not yet
    /// followed by an `adopt`, A7 §0.5) must refuse and raise, never
    /// publish under whichever key the vault happens to hold.
    #[tokio::test]
    async fn a_mismatched_vault_key_raises_an_alert_and_never_publishes() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
        // The vault genuinely holds an app master for "inst-1"...
        keys::app_master(&s.vault, "inst-1").await.unwrap();
        // ...but the row claims a different DID, the stale-handover case.
        let state = desired_state_with_app_master("inst-1", "did:key:zStaleRowClaim");
        let instance_id = AppInstanceId::new("inst-1");
        let mut opened = Vec::new();

        s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

        assert!(
            writer.published.lock().unwrap().is_empty(),
            "a mismatched identity must never be published"
        );
        assert_eq!(opened, vec![(AlertKind::AppIdentityMismatch, "inst-1".to_string())]);
    }

    /// Companion to A5c's own two paused-instance tests: `pause` excludes
    /// an instance from the write phase entirely, and the Tier-1 refresh
    /// is no exception (D-S1-4 keeps that skip rather than reopening it).
    #[tokio::test]
    async fn a_paused_instance_gets_no_tier1_refresh() {
        let writer = Arc::new(RecordingTier1Writer::default());
        let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0)
            .unwrap();
        let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
        s.store.record_adopt("inst-1", 0, &app_did).unwrap();
        s.store.pause("inst-1").unwrap();

        s.reconcile_instance_pass("inst-1").await;

        assert!(writer.published.lock().unwrap().is_empty(), "a paused instance gets zero work");
    }

    // ── Phase 5: revocation ──────────────────────────────────────────────

    /// D-A5d-15: a revoked placement is skipped by `apply_with_clients`
    /// itself, which is the one path every certificate-minting caller
    /// passes through -- the loop, `submit`, and `force-reconcile` alike.
    /// Without that, an ordinary resubmit silently re-mints the very key
    /// the operator revoked.
    #[tokio::test]
    async fn a_submit_of_the_same_plan_does_not_recertify_a_revoked_placement() {
        let s = service();
        let plan =
            DeploymentPlan::from_json(&plan_json_two_services("inst-1", "backend", "frontend"))
                .unwrap();
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        // No clients are built for either service, so `certify_placed_
        // members` fails on whichever service actually reaches it -- and
        // the error names it. A revoked service that reached it would show
        // up here by name.
        let err = s
            .apply_with_clients(&plan, &plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
            .await
            .unwrap_err();
        assert!(
            !err.contains("hFabricatedA"),
            "the revoked member must never reach the certify step: {err}"
        );
        assert!(
            err.contains("hFabricatedB"),
            "the rest of the plan must still be attempted: {err}"
        );

        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        let revoked: Vec<_> =
            alerts.iter().filter(|a| a.kind == AlertKind::InstanceRevoked).collect();
        assert_eq!(revoked.len(), 1, "{alerts:?}");
        assert_eq!(revoked[0].logical_ref.as_deref(), Some("inst-1/backend#0"));
    }

    /// `force-reconcile` reaches the same gate by the same route: its own
    /// doc already notes it bypasses several checks `submit` applies, so
    /// putting the exclusion anywhere upstream of `apply_with_clients`
    /// would have left this path open.
    #[tokio::test]
    async fn a_force_reconcile_does_not_recertify_a_revoked_placement_and_raises_instance_revoked_for_the_rest_of_the_plan()
     {
        let s = service();
        let plan_json = plan_json_two_services("inst-1", "backend", "frontend");
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        let err = s
            .apply_with_clients(&plan, &plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
            .await
            .unwrap_err();
        assert!(!err.contains("hFabricatedA"), "{err}");

        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        let revoked = alerts
            .iter()
            .find(|a| a.kind == AlertKind::InstanceRevoked)
            .unwrap_or_else(|| panic!("no InstanceRevoked alert among {alerts:?}"));
        assert!(
            revoked.detail.contains("Undeploy it separately"),
            "the alert must say revocation is not a teardown: {}",
            revoked.detail
        );
    }

    /// The raise used to pass the *alias* for both the
    /// alias and DID arguments, so `substrate_did` on the stored row held
    /// e.g. `edge-1` instead of a real DID -- inconsistent with every
    /// other alert kind's rows. Resolved through this pass's own
    /// connected clients, so the column holds what it is supposed to.
    #[tokio::test]
    async fn instance_revoked_records_a_real_substrate_did_not_the_alias() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        let identity = Identity::generate().unwrap();
        let client = Arc::new(SyneroymClient::new_with_identity(
            "did:key:zEdge1".to_string(),
            String::new(),
            identity,
        ));
        let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);

        // The plan's one service is entirely revoked, so nothing remains
        // to certify once it is filtered out -- no live connection is
        // needed for the call to succeed.
        s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new())
            .await
            .unwrap();

        let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        let revoked = alerts
            .iter()
            .find(|a| a.kind == AlertKind::InstanceRevoked)
            .unwrap_or_else(|| panic!("no InstanceRevoked alert among {alerts:?}"));
        assert_eq!(revoked.substrate_did, "did:key:zEdge1");
        assert_ne!(revoked.substrate_did, "edge-1", "must be the DID, not the alias");
    }

    /// Every existing revocation test drove one `apply_with_clients` call
    /// in isolation and stopped, so nothing proved the *next* pass stays
    /// quiet. The trigger: an ordinary `submit`
    /// or `force-reconcile` after a revocation, which reaches
    /// `apply_with_clients` with `record_plan == plan` -- followed by a
    /// real resident-loop pass reading back what that call journaled.
    #[tokio::test]
    async fn a_revoked_placement_does_not_reappear_as_an_add_on_the_next_pass() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

        let identity = Identity::generate().unwrap();
        let client = Arc::new(SyneroymClient::new_with_identity(
            "did:key:zEdge1".to_string(),
            String::new(),
            identity,
        ));
        let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);

        // The ordinary `submit`/`force-reconcile` route: `record_plan ==
        // plan`, the shape that used to drop the revoked member from the
        // journaled baseline.
        s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new())
            .await
            .unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        let after_first_apply = s.store.journal.get_latest(&instance_id).unwrap().unwrap();

        // Checked directly against what the
        // next pass's own diff would read: the revoked member must not
        // show up as a fresh `Add` against the baseline the call above
        // just journaled.
        let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
        assert!(
            diff.actions.is_empty(),
            "a revoked member must not read back as a change against its own just-journaled \
             baseline: {:?}",
            diff.actions
        );

        // A real resident-loop pass, reading exactly that baseline back.
        // Unbounded regrowth would show up here as a second journal
        // entry -- the ~2,880-rows-a-day shape the review measured.
        s.reconcile_instance_pass("inst-1").await;
        let after_second_pass = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
        assert_eq!(
            after_second_pass.id, after_first_apply.id,
            "a quiet pass must not append a new journal record"
        );
    }

    /// The renewal work-list's own half of the same exclusion.
    #[test]
    fn a_renewal_pass_skips_a_revoked_placement_even_when_near_expiry() {
        let report = report_of(vec![
            near_expiry_health("backend", "did:key:hBackend"),
            near_expiry_health("frontend", "did:key:hFrontend"),
        ]);
        let revoked = BTreeSet::from(["inst-1/backend#0".to_string()]);
        let candidates =
            SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &revoked, NOW, 5);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].member_ref, "inst-1/frontend#0");
    }

    /// D-A5d-14: without the lock, an operator's `revoke-instance` and a
    /// resident pass's renewal of the same member race -- the pass could
    /// mint and install a fresh certificate in the gap between the anchor
    /// write and the exclusion write landing, which is the window this
    /// verb exists to close.
    #[tokio::test]
    async fn revoke_instance_takes_the_instance_lock_for_the_whole_call() {
        let s = Arc::new(
            Fixture {
                anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
                ..Fixture::default()
            }
            .build(),
        );
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let held = s.instance_lock("inst-1");
        let guard = held.lock().await;

        let s2 = s.clone();
        let call = tokio::spawn(async move {
            dispatch(
                &s2,
                admin_caller("did:key:zSupervisorNode"),
                "revoke-instance",
                serde_json::json!(["inst-1", "inst-1/backend"]),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!call.is_finished(), "revoke-instance must block on the instance lock");
        drop(guard);

        // Now it proceeds -- and refuses, because the stored plan names no
        // such member. What matters here is that it got that far only
        // after the lock was released.
        let err = call.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("inst-1/backend"), "{err}");
    }

    /// The line that actually decides which DID
    /// gets revoked had no direct test, since `handle_revoke_instance`
    /// needs a live client. Hoisted into a pure function so both branches
    /// are assertable with no substrate at all.
    #[test]
    fn select_revocation_did_prefers_the_installed_did_over_the_derived_one() {
        let installed = syneroym_sdk::InstanceIdentity {
            instance_did: "did:key:zDerivedForThisCaller".to_string(),
            pubkey_hex: "aa".to_string(),
            installed_temporary_did: Some("did:key:zActuallyInstalled".to_string()),
        };
        assert_eq!(
            SupervisorService::select_revocation_did(installed),
            "did:key:zActuallyInstalled"
        );

        let nothing_installed = syneroym_sdk::InstanceIdentity {
            instance_did: "did:key:zDerivedForThisCaller".to_string(),
            pubkey_hex: "aa".to_string(),
            installed_temporary_did: None,
        };
        assert_eq!(
            SupervisorService::select_revocation_did(nothing_installed),
            "did:key:zDerivedForThisCaller"
        );
    }

    /// The anchor half: the *derived instance* DID goes into the master's
    /// revoked list, never the master's own -- revoking the master would
    /// repudiate every instance it has ever certified.
    #[tokio::test]
    async fn revoke_instance_appends_the_derived_instance_did_to_revoked_keys() {
        let writer = Arc::new(RecordingAnchorWriter::default());
        let s = Fixture { anchor_writer: Some(writer.clone()), ..Fixture::default() }.build();
        let master_did = seeded_member(&s, "backend").await;

        s.record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
            .await
            .unwrap();

        assert_eq!(
            *writer.revoked.lock().unwrap(),
            vec![(master_did.clone(), "did:key:zInstanceKey".to_string())]
        );
        assert_ne!(
            writer.revoked.lock().unwrap()[0].1,
            master_did,
            "the revoked entry must be the instance key, not the member master"
        );
    }

    /// The local half, and the ordering between the two: the anchor
    /// publish comes first, and the exclusion is written only after it
    /// succeeds. A failed publish must leave the placement under ordinary
    /// management rather than half-revoked -- excluded from renewal here
    /// while still fully trusted by every consumer, which would let it age
    /// out quietly instead of failing closed.
    #[tokio::test]
    async fn revoke_instance_writes_a_revoked_placements_row() {
        let s = Fixture {
            anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
            ..Fixture::default()
        }
        .build();
        seeded_member(&s, "backend").await;

        s.record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
            .await
            .unwrap();
        assert_eq!(
            s.store.revoked_placements("inst-1").unwrap(),
            BTreeSet::from(["inst-1/backend".to_string()])
        );

        let failing_writer =
            Arc::new(RecordingAnchorWriter { fail: true, ..RecordingAnchorWriter::default() });
        let failing = Fixture { anchor_writer: Some(failing_writer), ..Fixture::default() }.build();
        seeded_member(&failing, "backend").await;

        let err = failing
            .record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
            .await
            .unwrap_err();
        assert!(err.contains("failed to publish"), "{err}");
        assert!(
            failing.store.revoked_placements("inst-1").unwrap().is_empty(),
            "a revocation that did not publish must not have written a local exclusion"
        );
    }

    /// A node with no registry configured cannot publish a revocation at
    /// all, and must say so rather than writing a local exclusion that no
    /// consumer can see.
    #[tokio::test]
    async fn revoke_instance_is_refused_when_the_node_has_no_registry_configured() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let err = s
            .handle_revoke_instance(
                &admin_caller("did:key:zSupervisorNode"),
                serde_json::json!(["inst-1", "inst-1/backend"]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("registry"), "{err}");
        assert!(s.store.revoked_placements("inst-1").unwrap().is_empty());
    }

    // ── M05A A7: the app-instance master identity ───────────────────────

    fn adopt_field<'a>(res: &'a NativeResponse, field: &str) -> Option<&'a Value> {
        res.payload.get(field)
    }

    /// D-A7-1/D-A7-4/D-A7-8: the ordinary path, over a services-less plan
    /// so no substrate is involved (§0.12) -- `adopt` mints an app master
    /// and both the vault and the instance row carry it afterwards.
    #[tokio::test]
    async fn adopt_mints_an_app_master_and_records_it_on_the_instance_row() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
        let vault_name = adopt_field(&res, "vault_name").and_then(Value::as_str).unwrap();
        assert!(did.starts_with("did:key:"), "{did}");
        assert_eq!(vault_name, "app-inst-1");
        assert_eq!(adopt_field(&res, "generation").and_then(Value::as_u64), Some(1));

        let row_did = s.store.get("inst-1").unwrap().unwrap().app_master_did;
        assert_eq!(row_did, did);
        let vault_entry = s.vault.get("app-inst-1").await.unwrap().unwrap();
        assert_eq!(substrate::derive_did_key(&vault_entry.public_key()), did);
    }

    /// D-A7-1: a locked vault refuses the whole call, before a generation
    /// is claimed and before any key is minted -- not through a
    /// `kek_is_loaded` pre-check. The locked fixture (encryption on, no
    /// KEK) is the only shape that proves anything about locking.
    #[tokio::test]
    async fn adopt_on_a_locked_vault_refuses_before_it_claims_a_generation() {
        let s = Fixture { locked_vault: true, ..Fixture::default() }.build();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("inject-kek"), "{err}");

        let row = s.store.get("inst-1").unwrap().unwrap();
        assert_eq!(row.generation, 0, "a refused adopt must not claim a generation");
        assert_eq!(row.app_master_did, "", "a refused adopt must not record a DID");
    }

    /// D-A7-7's "and nowhere else" (M05A A7 review finding 7a): `adopt` is
    /// the only mint point, stated as a decision rather than merely true
    /// of the paths tested so far. Test 94 shows the field absent right
    /// after `submit`; this covers the two paths most likely to grow a
    /// mint by accident later, since both re-run the same apply pipeline
    /// `adopt` does over the identical plan -- `force-reconcile` and one
    /// resident-loop pass.
    #[tokio::test]
    async fn app_master_did_stays_empty_through_force_reconcile_and_a_loop_pass_without_adopt() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "force-reconcile",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");

        s.run_pass().await;
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");
    }

    /// D-A7-5: the DID stays stable across two `adopt`s -- resolving, not
    /// minting, on the second call. Over a services-less plan (§0.12), the
    /// generation itself stays `1` on both calls too: `claim_next_
    /// generation` reads the held maximum only from the substrates the
    /// plan places services on, and an empty plan has none to remember a
    /// prior claim, so this in-process shape cannot demonstrate the
    /// generation actually advancing -- that needs a real substrate, which
    /// is what the e2e (test 98, step d) proves alongside DID stability.
    /// Named for what it actually asserts (M05A A7 review finding 9,
    /// renamed from `…_at_the_next_generation`, which promised an
    /// increment this test cannot produce).
    #[tokio::test]
    async fn a_second_adopt_reports_the_same_app_master_did() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let first = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let second = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        assert_eq!(
            adopt_field(&first, "app_master_did").and_then(Value::as_str),
            adopt_field(&second, "app_master_did").and_then(Value::as_str)
        );
        assert_eq!(adopt_field(&first, "generation").and_then(Value::as_u64), Some(1));
        assert_eq!(adopt_field(&second, "generation").and_then(Value::as_u64), Some(1));
    }

    /// D-A7-6: `status` reports the same DID `adopt` minted.
    #[tokio::test]
    async fn status_reports_the_app_master_did_of_an_adopted_instance() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        let adopted = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let minted_did = adopt_field(&adopted, "app_master_did").and_then(Value::as_str).unwrap();

        let status = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(status.payload.get("app_master_did").and_then(Value::as_str), Some(minted_did));
    }

    /// D-A7-6/§0.7: absent, not an empty string -- an instance that has
    /// never been adopted under A7 must not read as though it holds a DID
    /// of `""`.
    #[tokio::test]
    async fn status_reports_no_app_master_for_an_instance_that_was_never_adopted() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let status = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert!(
            status.payload.get("app_master_did").is_none_or(Value::is_null),
            "absent means null once serialized, not an empty string: {:?}",
            status.payload.get("app_master_did")
        );
    }

    /// D-C-2: `status` reports the currently-published Tier-1 record's
    /// expiry, derived from the last successful refresh this supervisor
    /// stamped -- not from a fresh registry lookup.
    #[tokio::test]
    async fn status_reports_the_tier_one_record_expiry() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        s.store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
        s.store.record_tier1_refresh("did:key:zAppMaster", 1_000).unwrap();

        let status = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        assert_eq!(
            status.payload.get("app_record_expires_at").and_then(Value::as_u64),
            Some(1_000u64 + DEFAULT_ENDPOINT_NOT_AFTER_SECS)
        );
    }

    /// The absent case: no successful publish is on record (never
    /// adopted, no registry configured, or a vault locked since before
    /// the first refresh), so there is no expiry to report.
    #[tokio::test]
    async fn status_reports_no_tier_one_expiry_when_never_published() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let status = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        assert!(status.payload.get("app_record_expires_at").is_none_or(Value::is_null));
    }

    /// D-S1-4: `pause`'s response reports the date its own write-phase skip
    /// (`a_paused_instance_gets_no_tier1_refresh`, above) will let the
    /// currently-published record decay to. Named for the response field
    /// this actually asserts, not the `tracing::warn!` alongside it --
    /// that log line is real (`handle_pause`) but outside what a
    /// dispatch-level RPC test can capture without the same
    /// `run_capturing_logs`-shaped machinery `keys.rs` uses.
    #[tokio::test]
    async fn pause_reports_the_records_expiry_date() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        s.store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
        s.store.record_tier1_refresh("did:key:zAppMaster", 1_000).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "pause",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        assert_eq!(
            res.payload.get("app_record_expires_at").and_then(Value::as_u64),
            Some(1_000u64 + DEFAULT_ENDPOINT_NOT_AFTER_SECS)
        );
        assert!(s.store.get("inst-1").unwrap().unwrap().paused, "pause must still take effect");
    }

    /// Nothing published yet means nothing to warn about -- `pause` must
    /// not invent an expiry for a record that was never signed.
    #[tokio::test]
    async fn pause_has_nothing_to_warn_about_when_never_published() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "pause",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        assert!(res.payload.get("app_record_expires_at").is_none());
    }

    /// D-A7-5/§0.5: the handover-order repair inside one vault -- mint by
    /// adopting, import a different key under the same name (simulating an
    /// operator-carried backup replacing this vault's own key), adopt
    /// again, and the row follows the vault rather than keeping the
    /// replaced DID.
    #[tokio::test]
    async fn adopt_after_an_import_records_the_imported_did_not_the_one_it_replaced() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        let first = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let original_did =
            adopt_field(&first, "app_master_did").and_then(Value::as_str).unwrap().to_string();

        let replacement = Identity::generate().unwrap();
        s.vault.import("app-inst-1", &replacement.to_bytes()).await.unwrap();
        let replacement_did = substrate::derive_did_key(&replacement.public_key());
        assert_ne!(original_did, replacement_did);

        let second = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(
            adopt_field(&second, "app_master_did").and_then(Value::as_str),
            Some(replacement_did.as_str())
        );
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, replacement_did);
    }

    /// D-A7-7: an instance whose row predates A7 -- generation already
    /// claimed, `app_master_did` empty -- gains one at its *next* `adopt`,
    /// never anywhere else. Simulated by writing the pre-A7 state directly
    /// rather than going through `adopt` to reach it.
    #[tokio::test]
    async fn an_instance_row_with_no_app_master_gains_one_on_its_next_adopt() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        s.store.set_generation("inst-1", 2).unwrap();
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        // The generation bump itself is D-A7-7's named cost -- not
        // asserted at a specific number here, since a services-less plan
        // (§0.12) has no substrate to remember `2` was already claimed and
        // `claim_next_generation` always computes fresh from what the plan
        // places, which is nothing.
        let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
        assert!(did.starts_with("did:key:"));
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, did);
    }

    /// D-A7-8/§0.8: the A5b S1 failure, asserted directly -- the returned
    /// name must be one `export-master` actually accepts, not the bare
    /// logical name.
    #[tokio::test]
    async fn adopt_returns_the_vault_name_export_master_accepts() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let vault_name =
            adopt_field(&res, "vault_name").and_then(Value::as_str).unwrap().to_string();

        let export = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "export-master",
            serde_json::json!([vault_name]),
        )
        .await
        .unwrap();
        assert!(export.payload.as_str().unwrap().contains("app-inst-1"));
    }

    /// D-A7-4, added in review (§0.4/test 99): `status` must stay readable
    /// through a genuinely locked vault -- the column exists precisely so
    /// the app's identity is visible while the vault is shut, and this is
    /// the only test in this file that reaches that state over one held
    /// service rather than a fresh, empty rebuild. Locked *in place*
    /// (A5d's own recipe): unlocked at construction so `adopt` can mint,
    /// then the KEK is cleared afterward.
    #[tokio::test]
    async fn status_reports_the_app_master_did_while_the_vault_is_locked() {
        let (s, key_store) =
            Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
                .build_with_key_store();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        let adopted = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let minted_did =
            adopt_field(&adopted, "app_master_did").and_then(Value::as_str).unwrap().to_string();

        key_store.clear_kek();
        assert!(
            s.vault.get("app-inst-1").await.is_err(),
            "the vault must genuinely be locked for this test to prove anything"
        );

        let status = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(
            status.payload.get("app_master_did").and_then(Value::as_str),
            Some(minted_did.as_str())
        );
    }

    /// D-A7-5, the slice's most important new case (§0.5/test 100): a
    /// *second* supervisor, which has never adopted this instance,
    /// imports the app master another supervisor already exported, and
    /// its first `adopt` reports the imported DID rather than minting a
    /// fresh one. Two independent fixture-built services sharing one
    /// test-owned backup directory -- the stand-in for the file an
    /// operator carries between two real supervisors during a handover.
    #[tokio::test]
    async fn a_second_supervisor_that_imports_the_app_master_adopts_without_minting_a_new_one() {
        let backup_dir = tempfile::tempdir().unwrap();
        let supervisor_a =
            Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }
                .build();
        let supervisor_b =
            Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }
                .build();

        supervisor_a
            .store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        let adopted_a = dispatch(
            &supervisor_a,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let a_did =
            adopt_field(&adopted_a, "app_master_did").and_then(Value::as_str).unwrap().to_string();
        let vault_name =
            adopt_field(&adopted_a, "vault_name").and_then(Value::as_str).unwrap().to_string();

        dispatch(
            &supervisor_a,
            admin_caller("did:key:zSupervisorNode"),
            "export-master",
            serde_json::json!([vault_name.clone()]),
        )
        .await
        .unwrap();

        // Supervisor B has never adopted this instance -- it must still
        // hold its own desired-state row before `adopt` can act on it.
        supervisor_b
            .store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        dispatch(
            &supervisor_b,
            admin_caller("did:key:zSupervisorNode"),
            "import-master",
            serde_json::json!([vault_name]),
        )
        .await
        .unwrap();

        let adopted_b = dispatch(
            &supervisor_b,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        assert_eq!(
            adopt_field(&adopted_b, "app_master_did").and_then(Value::as_str),
            Some(a_did.as_str()),
            "B's first adopt must resolve A's imported DID, not mint a second identity"
        );
        assert_eq!(supervisor_b.store.get("inst-1").unwrap().unwrap().app_master_did, a_did);
    }

    /// D-A7-1/D-A7-5, optional (§0.12/test 101): the mint-before-claim,
    /// record-after-claim asymmetry's failing direction -- a claim that
    /// fails after the mint already landed must not mint a *second* key on
    /// the retry, since a vault key with no row is meant to be
    /// recoverable. Needs a placed service on an unreachable substrate, so
    /// the services-less shortcut every other test in this section uses
    /// does not apply.
    #[tokio::test]
    async fn a_failed_claim_after_a_successful_mint_reuses_the_same_app_master() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
        let inventory_json =
            serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
                .to_string();
        s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:owner", 0).unwrap();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap_err();
        // The claim failed against the unreachable substrate, not the
        // mint -- the row must show no generation was claimed, but the
        // vault must already hold a key, since the mint runs first.
        // `.expect` here, not `.map`: a plain `Option` comparison at the
        // bottom of this test would pass on `None == None` if the mint
        // ever stopped running before the claim (M05A A7 review finding
        // 3) -- the exact regression this test exists to catch -- since
        // both reads would then find nothing rather than the same key.
        assert_eq!(s.store.get("inst-1").unwrap().unwrap().generation, 0, "{err}");
        let minted = s
            .vault
            .get("app-inst-1")
            .await
            .unwrap()
            .expect("the mint runs before the claim, so the vault must already hold a key");
        let minted_did = substrate::derive_did_key(&minted.public_key());

        // A real deployment would fix the substrate before retrying;
        // here the same unreachable alias still fails the claim, so the
        // only thing left to prove is that the vault key from the first
        // attempt was reused, not replaced.
        let _ = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap_err();
        let second_minted = s
            .vault
            .get("app-inst-1")
            .await
            .unwrap()
            .expect("the retried mint must also resolve a key, not find nothing");
        let second_did = substrate::derive_did_key(&second_minted.public_key());
        assert_eq!(
            minted_did, second_did,
            "a retried mint must resolve the same key, not a new one"
        );
    }

    /// M05A A7 review finding 7c: `adopt`'s un-retire and its app-master
    /// write now land in the same `record_adopt` call (finding 6), but
    /// nothing at the service level had exercised them running back to
    /// back on a genuinely retired instance -- store-level coverage
    /// (`store.rs`'s `record_adopt_writes_generation_retired_and_
    /// app_master_did_together`) proves the store method alone, not that
    /// `handle_adopt` actually reaches it starting from `retired`.
    #[tokio::test]
    async fn adopt_on_a_retired_instance_un_retires_and_records_the_app_master_together() {
        let s = service();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        s.store.retire("inst-1").unwrap();
        assert!(s.store.get("inst-1").unwrap().unwrap().retired);

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "adopt",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();

        let row = s.store.get("inst-1").unwrap().unwrap();
        assert!(!row.retired, "adopt must un-retire the instance");
        let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
        assert_eq!(row.app_master_did, did);
    }

    // ── M05B B1: the durable outbox worker ──────────────────────────────

    /// A fake standing in for a connected substrate on the queue worker's
    /// replay path: one scripted delivery per DID, consumed in FIFO order.
    /// `ConnectFails` simulates the substrate still being unreachable;
    /// `Attempt` simulates a reconnect that succeeds, with `write_bindings`
    /// itself returning whatever the test scripts.
    #[derive(Debug, Default)]
    struct FakeQueueConnector {
        scripted: Mutex<BTreeMap<String, VecDeque<FakeDelivery>>>,
    }

    #[derive(Debug)]
    enum FakeDelivery {
        /// The reconnect itself fails -- a transport failure.
        ConnectFails,
        /// Reconnected; `write_bindings` returns this outcome.
        Attempt(Result<Vec<BindingWriteOutcome>, String>),
        /// Reconnected, and the substrate answered with a `JsonRpcError` --
        /// the callee-error shape `deploy::is_callee_error` classifies as
        /// terminal, distinct from a plain transport failure (which
        /// `Attempt(Err(_))` alone cannot express: an ordinary string error
        /// is not downcastable to `JsonRpcError`, so it would be classified
        /// as transport, same as `ConnectFails`).
        CalleeError(String),
        /// The reconnect never resolves -- stands in for a substrate that
        /// never answers, so a test can prove something is genuinely
        /// in-flight when shutdown is asked for (M05B B1 review finding 5).
        /// Distinct from an absent scripted entry (`None` below), which
        /// returns an error immediately and proves nothing about
        /// abandoning in-flight work.
        Blocks,
    }

    impl FakeQueueConnector {
        fn script(&self, did: &str, delivery: FakeDelivery) {
            self.scripted.lock().unwrap().entry(did.to_string()).or_default().push_back(delivery);
        }
    }

    #[derive(Debug)]
    struct ScriptedAttempt {
        result: Mutex<Option<anyhow::Result<Vec<BindingWriteOutcome>>>>,
    }

    #[async_trait::async_trait]
    impl WriteBindingsAttempt for ScriptedAttempt {
        async fn attempt_write_bindings(
            &self,
            _write: BindingWrite,
        ) -> anyhow::Result<Vec<BindingWriteOutcome>> {
            self.result.lock().unwrap().take().expect("scripted result already consumed")
        }
    }

    #[async_trait::async_trait]
    impl QueueConnector for FakeQueueConnector {
        async fn connect(
            &self,
            entry: &SupervisorInventoryEntry,
        ) -> anyhow::Result<Arc<dyn WriteBindingsAttempt>> {
            let next =
                self.scripted.lock().unwrap().get_mut(&entry.did).and_then(VecDeque::pop_front);
            let result: anyhow::Result<Vec<BindingWriteOutcome>> = match next {
                Some(FakeDelivery::ConnectFails) => {
                    return Err(anyhow::anyhow!("simulated connect failure"));
                }
                Some(FakeDelivery::Attempt(Ok(outcomes))) => Ok(outcomes),
                Some(FakeDelivery::Attempt(Err(msg))) => Err(anyhow::anyhow!(msg)),
                Some(FakeDelivery::CalleeError(msg)) => {
                    Err(syneroym_rpc::JsonRpcError { code: -32010, message: msg, data: None }
                        .into())
                }
                Some(FakeDelivery::Blocks) => future::pending().await,
                None => return Err(anyhow::anyhow!("no scripted delivery for {}", entry.did)),
            };
            Ok(Arc::new(ScriptedAttempt { result: Mutex::new(Some(result)) }))
        }
    }

    /// Every real fake in this crate implements the full `SubstrateActor`
    /// trait, not just `WriteBindingsAttempt` -- `deploy::build_durable_actor`
    /// requires both, since a durable actor still answers every other
    /// action synchronously (D-B1-3).
    #[derive(Debug, Default)]
    struct FakeSubstrateClient {
        write_bindings_outcome: Mutex<Option<Result<Vec<BindingWriteOutcome>, String>>>,
    }

    #[async_trait::async_trait]
    impl WriteBindingsAttempt for FakeSubstrateClient {
        async fn attempt_write_bindings(
            &self,
            _write: BindingWrite,
        ) -> anyhow::Result<Vec<BindingWriteOutcome>> {
            match self
                .write_bindings_outcome
                .lock()
                .unwrap()
                .take()
                .expect("outcome not set for test")
            {
                Ok(outcomes) => Ok(outcomes),
                Err(msg) => Err(anyhow::anyhow!(msg)),
            }
        }
    }

    #[async_trait::async_trait]
    impl SubstrateActor for FakeSubstrateClient {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by the queue worker tests")
        }

        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("DurableActor never calls the inner T::write_bindings directly")
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("not exercised by the queue worker tests")
        }

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by the queue worker tests")
        }

        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("not exercised by the queue worker tests")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by the queue worker tests")
        }
    }

    fn test_binding_write(service_id: &str, app_instance_id: &str) -> BindingWrite {
        BindingWrite {
            service_id: service_id.to_string(),
            app_instance_id: app_instance_id.to_string(),
            bindings: vec![],
            generation: 0,
        }
    }

    /// Submits minimal desired state whose inventory names one substrate
    /// alias/DID pair -- enough for `deliver_queued_item` to resolve a
    /// queued item's target, without a real plan or a real deploy.
    fn seed_inventory(store: &SupervisorStore, app_instance_id: &str, substrate_did: &str) {
        let inventory_json =
            serde_json::json!({"edge-1": {"did": substrate_did, "api_url": "http://127.0.0.1:1"}})
                .to_string();
        store
            .submit(
                app_instance_id,
                &plan_json_no_services(app_instance_id),
                &inventory_json,
                "did:key:owner",
                0,
            )
            .unwrap();
    }

    fn enqueue_test_item(
        store: &SupervisorStore,
        app_instance_id: &str,
        logical_ref: &str,
        substrate_did: &str,
        write: BindingWrite,
    ) -> i64 {
        let key = QueueKey {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_did: substrate_did.to_string(),
        };
        let payload =
            outbox::QueuedBindingWrite { substrate_did: substrate_did.to_string(), write };
        store
            .queue
            .enqueue(
                app_instance_id,
                &key.to_string(),
                &serde_json::to_vec(&payload).unwrap(),
                outbox::now_ms(),
            )
            .unwrap()
    }

    /// Test 13: the load-bearing budget. A reachable substrate's
    /// `write_bindings` call must never touch the real, wired-up
    /// supervisor outbox -- asserted as "the queue is untouched", not as a
    /// timing, so it cannot pass by being fast.
    #[tokio::test]
    async fn a_reachable_substrate_never_touches_the_queue() {
        let s = service();
        let fake = Arc::new(FakeSubstrateClient {
            write_bindings_outcome: Mutex::new(Some(Ok(vec![BindingWriteOutcome::Applied]))),
        });
        let outbox = Arc::new(SupervisorOutbox::new(s.store.queue.clone()));
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let actor =
            deploy::build_durable_actor(fake, "did:key:zB".to_string(), key.to_string(), outbox);

        let outcomes =
            actor.write_bindings(test_binding_write("did:key:zSvc", "inst-1")).await.unwrap();
        assert_eq!(outcomes, vec![BindingWriteOutcome::Applied]);
        assert!(s.store.queue.all().unwrap().is_empty());
        assert!(s.store.queue.dead_letters().unwrap().is_empty());
    }

    /// Test 19: failure-matrix row 2. The worker's replay of an
    /// already-applied write must be a no-op from the substrate's own
    /// epoch guard's perspective -- scripted here as the fake simply
    /// reporting `NoOp` on delivery, which the worker must complete
    /// exactly like `Applied`.
    #[tokio::test]
    async fn applying_a_queued_write_bindings_twice_is_a_no_op() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::NoOp])));
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        let id = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        assert!(
            s.store.queue.all().unwrap().is_empty(),
            "the item {id} must be gone from the outbox"
        );
        assert!(s.store.queue.dead_letters().unwrap().is_empty());
    }

    /// Test 20: D-B1-15. A `stale` delivery means a newer epoch already
    /// landed -- convergence, not loss -- so it must complete and must not
    /// dead-letter.
    #[tokio::test]
    async fn a_queued_write_delivered_stale_completes_and_does_not_dead_letter() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Stale(7)])));
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        assert!(s.store.queue.all().unwrap().is_empty());
        assert!(
            s.store.queue.dead_letters().unwrap().is_empty(),
            "a stale delivery is convergence, not a failure"
        );
    }

    /// M05B B1 review follow-on 4: a queued item whose instance was
    /// retired between enqueue and delivery must be quietly completed --
    /// no delivery attempt (it would resurrect a binding the operator just
    /// released) and no `DeliveryExhausted` alert (noise against an
    /// instance nobody is going to act on). Unscripted `FakeQueueConnector`
    /// is deliberate: reaching `connect` at all would fail the test with
    /// "no scripted delivery", so this also proves the retired branch
    /// returns before ever attempting one.
    #[tokio::test]
    async fn a_queued_item_for_a_retired_instance_completes_quietly() {
        let mut s = service();
        s.queue_connector = Arc::new(FakeQueueConnector::default());
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );
        s.store.retire("inst-1").unwrap();

        s.queue_worker_tick().await;

        assert!(s.store.queue.all().unwrap().is_empty(), "the item must be gone from the outbox");
        assert!(
            s.store.queue.dead_letters().unwrap().is_empty(),
            "a retired instance's stale intent is moot, not a failure"
        );
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        assert!(
            !s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::DeliveryExhausted),
            "a retired instance must not gain a fresh DeliveryExhausted alert"
        );
    }

    /// Test 21: D-B1-15's `conflict` row -- the case test 14's synchronous
    /// coverage misses. Completes **and** raises the same `BindingConflict`
    /// alert the synchronous path does.
    #[tokio::test]
    async fn a_queued_write_delivered_conflicting_raises_the_same_alert_as_the_synchronous_path() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script(
            "did:key:zB",
            FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Conflict(9)])),
        );
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        assert!(s.store.queue.all().unwrap().is_empty(), "a conflict still completes the item");
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(
            active.iter().any(|a| a.kind == AlertKind::BindingConflict
                && a.logical_ref.as_deref() == Some("inst-1/backend")),
            "{active:?}"
        );
    }

    /// A transport failure on replay must return the item to the outbox
    /// rather than dead-lettering it outright -- the queue's own retry
    /// budget governs when it finally gives up, not one failed replay.
    #[tokio::test]
    async fn a_transport_failure_on_replay_retries_rather_than_dead_lettering() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script("did:key:zB", FakeDelivery::ConnectFails);
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        let remaining = s.store.queue.all().unwrap();
        assert_eq!(remaining.len(), 1, "a transport failure must stay in the outbox");
        assert!(s.store.queue.dead_letters().unwrap().is_empty());
    }

    /// M05B B1 review finding 10: a callee error that does *not* name this
    /// write's own target as gone -- a transient, reached-and-answered
    /// refusal the wire protocol cannot currently distinguish from "gone"
    /// by error code -- must stay retryable on the queued path, not
    /// dead-letter on its first delivery. Otherwise a queued item that
    /// survived a restart specifically to be retried would be given up on
    /// by a hiccup a later attempt would have cleared.
    #[tokio::test]
    async fn an_ambiguous_callee_error_on_replay_retries_rather_than_dead_lettering() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script(
            "did:key:zB",
            FakeDelivery::CalleeError("database is locked, try again".to_string()),
        );
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        let remaining = s.store.queue.all().unwrap();
        assert_eq!(remaining.len(), 1, "an ambiguous callee error must stay in the outbox");
        assert!(s.store.queue.dead_letters().unwrap().is_empty());
    }

    /// Failure-matrix row 9: a callee error on replay naming this write's
    /// own target as gone (the exact wording `control_plane`'s
    /// `write-bindings` dispatch uses for that specific refusal, M05B B1
    /// review finding 10) is terminal -- straight to the DLQ, not retried.
    #[tokio::test]
    async fn a_callee_error_on_replay_dead_letters_immediately() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script(
            "did:key:zB",
            FakeDelivery::CalleeError(
                "'did:key:zSvc' has no app context on this substrate".to_string(),
            ),
        );
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        s.queue_worker_tick().await;

        assert!(s.store.queue.all().unwrap().is_empty());
        let dead = s.store.queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].attempts, 1, "a terminal failure dead-letters on its first attempt");
    }

    /// Test 22: D-B1-14. Without taking `instance_lock`, the worker could
    /// interleave with a live pass write for the same instance and race
    /// this supervisor into a spurious `BindingConflict`. Proven by
    /// holding the lock externally (as a live pass would) and asserting
    /// the worker's own attempt to take it blocks until released.
    #[tokio::test]
    async fn the_worker_and_a_loop_pass_never_write_one_instance_concurrently() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        let held = s.instance_lock("inst-1");
        let guard = held.lock().await;

        let s = Arc::new(s);
        let s_clone = s.clone();
        let mut tick = tokio::spawn(async move { s_clone.queue_worker_tick().await });

        tokio::select! {
            _ = &mut tick => panic!("the worker must block on instance_lock while a pass holds it"),
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }

        drop(guard);
        tick.await.unwrap();
        assert!(
            s.store.queue.all().unwrap().is_empty(),
            "the worker delivers once the lock frees up"
        );
    }

    /// Test 23: D-B1-8. Cancelling the token must not wait for a delivery
    /// *genuinely in flight* -- `FakeDelivery::Blocks` makes `connect`
    /// never resolve, so if `shutdown` returned promptly here it is
    /// because cancellation actually interrupted a real, ongoing delivery
    /// (M05B B1 review finding 5: an earlier version of this test left the
    /// substrate unscripted, which fails `connect` immediately and
    /// proves nothing about abandoning work in flight -- its own comment
    /// also claimed `queue_tick_secs` was 1s here when it was actually set
    /// to 3600, so the tick this test depended on never fired at all
    /// inside its own real-clock sleep).
    #[tokio::test]
    async fn shutdown_abandons_in_flight_work_rather_than_draining() {
        let mut s = Fixture { queue_tick_secs: Some(1), ..Fixture::default() }.build();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script("did:key:zB", FakeDelivery::Blocks);
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        let s = Arc::new(s);
        let run_handle = {
            let s = s.clone();
            tokio::spawn(async move { s.run_queue_worker().await })
        };
        // Give the 1s interval its first tick a chance to fire, claim the
        // item, and reach `connect` -- which then blocks forever, so by
        // the time this returns a delivery is genuinely in flight.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        let start = Instant::now();
        s.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), run_handle)
            .await
            .expect(
                "run_queue_worker must return promptly on shutdown, not wait for the delivery in \
                 flight",
            )
            .unwrap()
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));

        // The abandoned item is still claimed (invisible) right after
        // shutdown -- dropped mid-delivery, not completed or failed.
        assert!(
            s.store.queue.claim_due(outbox::now_ms(), 10).unwrap().is_empty(),
            "the item must still be invisible, not silently completed or requeued"
        );
    }

    /// Test 24: the in-process analogue of e2e step 5 -- a queued item
    /// survives a supervisor restart (a fresh `SupervisorStore` opened
    /// against the same database file) and the new process's worker
    /// resumes it.
    #[tokio::test]
    async fn the_worker_resumes_a_queued_item_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
            seed_inventory(&store, "inst-1", "did:key:zB");
            enqueue_test_item(
                &store,
                "inst-1",
                "inst-1/backend",
                "did:key:zB",
                test_binding_write("did:key:zSvc", "inst-1"),
            );
        }

        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        let mut s = Fixture::default().build();
        s.store = store;
        let connector = Arc::new(FakeQueueConnector::default());
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
        s.queue_connector = connector;

        s.queue_worker_tick().await;

        assert!(
            s.store.queue.all().unwrap().is_empty(),
            "the resumed item must have been delivered"
        );
    }

    /// Test 25: the budget the plan's first draft asserted nowhere.
    /// Recovery must complete within one `queue_tick_secs`, not one
    /// `poll_interval_secs` -- driven against a paused clock with
    /// `poll_interval_secs` set far above the worker tick, so passing by
    /// accident (both loops running for real) is impossible.
    #[tokio::test(start_paused = true)]
    async fn recovery_completes_within_one_worker_tick_and_not_one_poll_interval() {
        let mut s = Fixture {
            queue_tick_secs: Some(5),
            poll_interval_secs: Some(3600),
            ..Fixture::default()
        }
        .build();
        let connector = Arc::new(FakeQueueConnector::default());
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );

        let s = Arc::new(s);
        let run_handle = {
            let s = s.clone();
            tokio::spawn(async move { s.run_queue_worker().await })
        };

        tokio::time::advance(Duration::from_secs(6)).await;
        // Yield so the worker's now-elapsed tick actually runs.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert!(
            s.store.queue.all().unwrap().is_empty(),
            "recovery must land within one 5s worker tick, well under the 3600s poll interval"
        );

        s.shutdown().await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(1), run_handle).await;
    }

    // ── M05B B1: the DLQ surface ─────────────────────────────────────────

    /// Test 27: D-B1-6. `AlertStore`'s unique index cannot express one row
    /// per dead letter, and an operator wants the standing fact anyway --
    /// a second dead letter for the same key must refresh the existing
    /// alert's count, not open a second row.
    #[tokio::test]
    async fn a_second_dead_letter_for_one_member_refreshes_the_alert_and_raises_its_count() {
        let s = service();
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id1 = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc1", "inst-1"),
        );
        let id2 = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc2", "inst-1"),
        );

        s.fail_queued_item(&instance_id, &key, id1, outbox::now_ms(), "boom", true).await;
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert_eq!(active.iter().filter(|a| a.kind == AlertKind::DeliveryExhausted).count(), 1);
        assert!(
            active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted && a.detail.contains('1'))
        );

        s.fail_queued_item(&instance_id, &key, id2, outbox::now_ms(), "boom again", true).await;
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert_eq!(
            active.iter().filter(|a| a.kind == AlertKind::DeliveryExhausted).count(),
            1,
            "a second dead letter must refresh the existing row, not open a second one"
        );
        assert!(
            active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted && a.detail.contains('2'))
        );
    }

    /// Test 28: D-B1-6's clear path -- the thing `RemediationExhausted`
    /// already documents and the draft omitted. Replaying every dead
    /// letter for a key clears its alert; an earlier replay leaving one
    /// behind must not.
    #[tokio::test]
    async fn the_alert_clears_when_the_last_dead_letter_for_that_key_is_gone() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
        connector
            .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id1 = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc1", "inst-1"),
        );
        let id2 = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc2", "inst-1"),
        );
        s.fail_queued_item(&instance_id, &key, id1, outbox::now_ms(), "boom", true).await;
        s.fail_queued_item(&instance_id, &key, id2, outbox::now_ms(), "boom again", true).await;
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::DeliveryExhausted)
        );

        let dead = s.store.queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 2);

        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "replay",
            serde_json::json!(["inst-1", dead[0].id as u64]),
        )
        .await
        .unwrap();
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::DeliveryExhausted),
            "one dead letter for this key still remains; the alert must stay active"
        );

        // The first replay's own row must resolve before the second dead
        // letter for the identical key can be replayed too -- `Queue::
        // replay` refuses a second pending row for one key (M05B B1 review
        // finding 2), so this drives the worker to deliver (and complete)
        // the first replay's item before trying the second.
        s.queue_worker_tick().await;
        assert!(s.store.queue.all().unwrap().is_empty(), "the first replay must have landed");

        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "replay",
            serde_json::json!(["inst-1", dead[1].id as u64]),
        )
        .await
        .unwrap();
        assert!(
            !s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::DeliveryExhausted),
            "the last dead letter for this key is gone; the alert must clear"
        );
    }

    /// M05B B1 review finding 3: a *pruned* dead letter (the DLQ cap
    /// evicting the oldest row on write, D-B1-9) must clear its own
    /// standing alert exactly the same way an explicit `replay` does --
    /// not just when a human happens to replay it. Distinct from the test
    /// above, which only exercises the replay path.
    #[tokio::test]
    async fn a_pruned_dead_letter_clears_its_own_alert_too() {
        let mut s = service();
        // A cap of 1 so the second key's own dead letter immediately
        // evicts the first key's -- both keys share one group (the app
        // instance), so the cap applies across them (M05B B1 review
        // finding 4).
        s.store.queue = Queue::open_in_memory(QueueConfig {
            dlq_max_rows: 1,
            ..QueueConfig::from(&SupervisorRole::default())
        })
        .unwrap();

        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key_a = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend-a".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let key_b = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend-b".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id_a = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend-a",
            "did:key:zB",
            test_binding_write("did:key:zSvcA", "inst-1"),
        );
        s.fail_queued_item(&instance_id, &key_a, id_a, outbox::now_ms(), "boom", true).await;
        assert!(
            s.store
                .alerts
                .active(&instance_id)
                .unwrap()
                .iter()
                .any(|a| a.kind == AlertKind::DeliveryExhausted
                    && a.logical_ref.as_deref() == Some("inst-1/backend-a")),
            "key a's dead letter must raise its own alert"
        );

        let id_b = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend-b",
            "did:key:zB",
            test_binding_write("did:key:zSvcB", "inst-1"),
        );
        // This dead-letters key b, which prunes key a's now-oldest row
        // past the cap of 1 -- with no `replay` in sight.
        s.fail_queued_item(&instance_id, &key_b, id_b, outbox::now_ms(), "boom too", true).await;

        assert_eq!(s.store.queue.dead_letters().unwrap().len(), 1, "the cap must still hold");
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(
            !active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted
                && a.logical_ref.as_deref() == Some("inst-1/backend-a")),
            "key a's dead letter was pruned; its alert must clear, not linger forever: {active:?}"
        );
        assert!(
            active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted
                && a.logical_ref.as_deref() == Some("inst-1/backend-b")),
            "key b's own alert must still be active: {active:?}"
        );
    }

    /// M05B B1 review finding 14: row 12's outbox growth bound -- "at most
    /// one row per `(instance, logical_ref, substrate)`" -- is enforced by
    /// `SupervisorOutbox::already_pending`, but nothing asserted it as the
    /// bound directly.
    #[tokio::test]
    async fn the_outbox_holds_at_most_one_row_per_key_regardless_of_how_many_enqueue_attempts() {
        let queue = Queue::open_in_memory(QueueConfig::from(&SupervisorRole::default())).unwrap();
        let outbox = SupervisorOutbox::new(queue.clone());
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let write = test_binding_write("did:key:zSvc", "inst-1");

        for generation in 0..50u64 {
            outbox
                .enqueue(
                    &key.to_string(),
                    "did:key:zB",
                    &BindingWrite { generation, ..write.clone() },
                )
                .await;
        }

        assert_eq!(
            queue.all().unwrap().len(),
            1,
            "fifty enqueue attempts for one key must still leave exactly one outbox row"
        );
    }

    /// Test 29: `dead-letters` lists exactly what the store holds, mapped
    /// onto the WIT shape (logical ref and substrate DID pulled back out
    /// of the opaque queue key).
    #[tokio::test]
    async fn dead_letters_lists_what_the_store_holds() {
        let s = service();
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );
        s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "unreachable", true).await;

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "dead-letters",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let rows: Vec<DeadLetter> = serde_json::from_value(res.payload).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].logical_ref, "inst-1/backend");
        assert_eq!(rows[0].substrate_did, "did:key:zB");
        assert_eq!(rows[0].last_error, "unreachable");
        assert_eq!(rows[0].attempts, 1);

        // A different instance's dead letters must not leak through.
        let other = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "dead-letters",
            serde_json::json!(["inst-2"]),
        )
        .await
        .unwrap();
        let other_rows: Vec<DeadLetter> = serde_json::from_value(other.payload).unwrap();
        assert!(other_rows.is_empty());
    }

    /// Test 30: D-B1-7. `replay` re-enqueues through the ordinary worker
    /// path and does not execute inline -- proven by scripting no delivery
    /// at all for the target DID and asserting the RPC call still
    /// succeeds, since replay itself never calls the connector.
    #[tokio::test]
    async fn replay_re_enqueues_and_does_not_execute_inline() {
        let mut s = service();
        s.queue_connector = Arc::new(FakeQueueConnector::default());
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );
        s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "unreachable", true).await;
        let dead_id = s.store.queue.dead_letters().unwrap()[0].id as u64;

        // No `FakeDelivery` scripted for "did:key:zB" -- if `replay`
        // executed inline it would have to reach the connector and this
        // call would fail closed (`FakeQueueConnector::connect` errors
        // with nothing scripted).
        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "replay",
            serde_json::json!(["inst-1", dead_id]),
        )
        .await
        .unwrap();

        assert!(s.store.queue.dead_letters().unwrap().is_empty());
        let requeued = s.store.queue.all().unwrap();
        assert_eq!(requeued.len(), 1);
    }

    /// Test 31: failure-matrix row 5, over the RPC surface -- a replayed
    /// item that fails again returns to the DLQ with its attempt history
    /// intact, listable the same way the first one was.
    #[tokio::test]
    async fn a_replayed_item_that_fails_again_returns_to_the_dlq_with_its_history() {
        let mut s = service();
        let connector = Arc::new(FakeQueueConnector::default());
        connector.script(
            "did:key:zB",
            FakeDelivery::CalleeError(
                "'did:key:zSvc' has no app context on this substrate".to_string(),
            ),
        );
        s.queue_connector = connector;
        seed_inventory(&s.store, "inst-1", "did:key:zB");
        let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
        let key = QueueKey {
            app_instance_id: "inst-1".to_string(),
            logical_ref: "inst-1/backend".to_string(),
            substrate_did: "did:key:zB".to_string(),
        };
        let id = enqueue_test_item(
            &s.store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );
        s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "first refusal", true).await;
        let dead_id = s.store.queue.dead_letters().unwrap()[0].id as u64;

        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "replay",
            serde_json::json!(["inst-1", dead_id]),
        )
        .await
        .unwrap();
        s.queue_worker_tick().await;

        let dead = s.store.queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 1, "the replayed item must be back in the DLQ after failing again");
        assert_eq!(dead[0].attempts, 2, "the attempt count must carry over, not reset");
    }

    // ── Scheduled tasks ──────────────────────────────────────────────────

    fn scheduled_service(name: &str, member_index: u32, cron: &str) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(format!("did:key:h{name}{member_index}")),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(name),
            },
            substrate: Some(SubstrateAlias::new("edge-1")),
            config: dummy_config(),
            resolved_dependencies: BTreeMap::new(),
            topology_mode: if member_index > 0 {
                TopologyMode::Redundant
            } else {
                TopologyMode::Singleton
            },
            member_index,
            schedule: Some(ScheduleSpec {
                cron: cron.to_string(),
                interface: InterfaceName::new("scheduled-driver"),
                method: "tick".to_string(),
                params: None,
                timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
            }),
            sharding_strategy: None,
            topology_visibility: Default::default(),
        }
    }

    fn plan_with_schedule(members: Vec<PlannedService>) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: members,
        }
    }

    fn scheduled_health(
        name: &str,
        member_index: u32,
        substrate_did: &str,
        signal: Signal,
    ) -> health::ServiceHealth {
        health::ServiceHealth {
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(name),
            },
            service_id: format!("did:key:h{name}{member_index}"),
            alias: Some(SubstrateAlias::new("edge-1")),
            substrate_did: substrate_did.to_string(),
            signal,
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            binding_epochs: Vec::new(),
            member_index,
        }
    }

    /// A timestamp exactly on a cron minute boundary, offset by
    /// `offset_secs` -- lets the watermark/grace tests reason about
    /// `"* * * * *"` occurrences in exact whole seconds rather than
    /// hoping `NOW` happens to align.
    fn minute(offset_secs: i64) -> u64 {
        use chrono::{TimeZone, Utc};
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 5, 0).unwrap().timestamp();
        u64::try_from(base + offset_secs).unwrap()
    }

    #[test]
    fn a_schedule_seen_for_the_first_time_does_not_fire_for_the_past() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        let decisions =
            SupervisorService::schedule_decisions(&plan, &BTreeMap::new(), &report, NOW, 60);
        assert_eq!(
            decisions,
            vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
            "a schedule with no state row must not fire on the pass that first sees it"
        );
    }

    #[test]
    fn a_due_schedule_runs_exactly_one_member() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (NOW - 3600) as i64, ..Default::default() },
        );
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
        assert_eq!(decisions.len(), 1);
        match &decisions[0] {
            ScheduleDecision::Run {
                logical_ref, member_index, service_id, substrate_did, ..
            } => {
                assert_eq!(logical_ref, "inst-1/worker");
                assert_eq!(*member_index, 0);
                assert_eq!(service_id, "did:key:hworker0");
                assert_eq!(substrate_did, "did:key:zEdge1");
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn a_schedule_evaluated_twice_inside_one_cron_minute_runs_once() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);

        // First pass, exactly on the minute boundary: due.
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (minute(0) - 30) as i64, ..Default::default() },
        );
        let first = SupervisorService::schedule_decisions(&plan, &states, &report, minute(0), 60);
        assert!(matches!(first[0], ScheduleDecision::Run { .. }));

        // A second pass ten seconds later, inside the same cron minute,
        // with the state a real `record_schedule_started` would have left.
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState {
                evaluated_at: minute(0) as i64,
                last_run_at: Some(minute(0) as i64),
                last_member_index: Some(0),
                last_error: None,
            },
        );
        let second =
            SupervisorService::schedule_decisions(&plan, &states, &report, minute(0) + 10, 60);
        assert_eq!(
            second,
            vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
            "a second look inside the same cron minute must not run again"
        );
    }

    #[test]
    fn a_tick_missed_while_the_supervisor_was_down_is_skipped_not_run_late() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        let mut states = BTreeMap::new();
        // Down for an hour before minute(0)'s own occurrence.
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (minute(0) - 3600) as i64, ..Default::default() },
        );
        // Comes back 40s after the boundary, with a grace window smaller
        // than the gap -- the boundary itself has already fallen outside
        // the window this pass computes.
        let now = minute(0) + 40;
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, now, 30);
        assert_eq!(
            decisions,
            vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
            "an hour-long gap must not fire a burst of catch-up runs"
        );
    }

    #[test]
    fn a_pass_delayed_by_less_than_the_grace_window_still_runs_its_tick() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (minute(0) - 3600) as i64, ..Default::default() },
        );
        // 40s late, but the grace window (60s) still covers minute(0)'s
        // own occurrence.
        let now = minute(0) + 40;
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, now, 60);
        assert!(
            matches!(&decisions[0], ScheduleDecision::Run { .. }),
            "ordinary jitter under the grace window must not silently drop the tick: {decisions:?}"
        );
    }

    /// Before the loop has completed a second sweep there is no observed
    /// gap to read, so the window is the configured floor -- which is also
    /// what a fresh process gets, so downtime can never widen it.
    #[test]
    fn the_grace_window_is_two_poll_intervals_until_a_sweep_has_been_timed() {
        let s = Fixture { poll_interval_secs: Some(30), ..Fixture::default() }.build();
        assert_eq!(s.schedule_grace_secs(NOW), 60);
    }

    /// The defect this rule exists for: a sweep that takes longer than two
    /// poll intervals -- routine, since every pass rebuilds an iroh client
    /// per substrate -- used to leave a hole between the last evaluation
    /// and the start of the window, and every occurrence inside that hole
    /// was dropped while the supervisor was awake the whole time.
    #[test]
    fn a_sweep_slower_than_two_poll_intervals_widens_the_grace_window_to_match() {
        let s = Fixture { poll_interval_secs: Some(30), ..Fixture::default() }.build();
        s.previous_pass_started_at.store(NOW - 300, Ordering::Relaxed);
        assert_eq!(s.schedule_grace_secs(NOW), 300);
    }

    /// The same defect, end to end through the decision it feeds: the
    /// watermark is 100s old because the previous sweep was 100s ago, and
    /// the tick in between must still fire even though the *configured*
    /// interval says a pass should have happened four times over.
    #[test]
    fn a_sweep_slower_than_two_poll_intervals_still_fires_the_tick_it_covered() {
        let s = Fixture { poll_interval_secs: Some(10), ..Fixture::default() }.build();
        let now = minute(0) + 40;
        s.previous_pass_started_at.store(now - 100, Ordering::Relaxed);

        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        let states = BTreeMap::from([(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (now - 100) as i64, ..Default::default() },
        )]);

        let decisions = SupervisorService::schedule_decisions(
            &plan,
            &states,
            &report,
            now,
            s.schedule_grace_secs(now),
        );
        assert!(
            matches!(&decisions[0], ScheduleDecision::Run { .. }),
            "a tick inside the real gap between two sweeps must not be dropped: {decisions:?}"
        );
    }

    /// The other side of the same rule. A paused instance is skipped before
    /// the health sweep, so its watermark goes stale -- but the loop keeps
    /// sweeping the whole time, so the observed gap stays one poll interval
    /// and the window on resume is still the floor. Nothing catches up.
    #[tokio::test]
    async fn a_paused_instance_fires_no_backlog_when_it_resumes() {
        let s = Fixture { poll_interval_secs: Some(10), ..Fixture::default() }.build();
        s.store
            .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
            .unwrap();
        s.store.pause("inst-1").unwrap();

        // A sweep over nothing but paused instances still times itself:
        // the liveness signal belongs to the loop, not to any instance.
        s.run_pass().await;
        assert_ne!(s.previous_pass_started_at.load(Ordering::Relaxed), 0);
        s.store.resume("inst-1").unwrap();

        // So on resume the observed gap is one sweep, however long the
        // pause was.
        let now = minute(0) + 40;
        s.previous_pass_started_at.store(now - 10, Ordering::Relaxed);
        assert_eq!(s.schedule_grace_secs(now), 20, "a pause must not widen the window");

        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
        let report =
            report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
        // The watermark an hour-long pause leaves behind.
        let states = BTreeMap::from([(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: (now - 3600) as i64, ..Default::default() },
        )]);

        let decisions = SupervisorService::schedule_decisions(
            &plan,
            &states,
            &report,
            now,
            s.schedule_grace_secs(now),
        );
        assert_eq!(
            decisions,
            vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
            "resuming must not fire the ticks that fell inside the pause: {decisions:?}"
        );
    }

    /// The write phase's own re-read (D-A5c-14) covers scheduled work too:
    /// a `pause` that lands between the health sweep and the write phase
    /// must stop the tick, not merely the deploys -- the run is dispatched
    /// from inside that phase, after the re-read.
    #[tokio::test]
    async fn a_pause_landing_mid_pass_stops_that_passs_scheduled_run() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "worker", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        let plan = DeploymentPlan::from_json(&plan_json).unwrap();
        s.store.pause("inst-1").unwrap();

        s.apply_write_phase(WritePhase {
            instance_id: &AppInstanceId::new("inst-1"),
            app_instance_id: "inst-1",
            plan: &plan,
            needs_work: &BTreeSet::new(),
            restart_candidates: &[],
            renewal_candidates: &[],
            pending_rotation_restarts: &BTreeSet::new(),
            push_candidates: &[],
            schedule_decisions: &[ScheduleDecision::Watermark {
                logical_ref: "inst-1/worker".to_string(),
            }],
            did_to_alias: &BTreeMap::new(),
            clients: &BTreeMap::new(),
            now: 100,
        })
        .await;

        assert!(
            s.store.schedule_states("inst-1").unwrap().is_empty(),
            "a paused instance must not even advance a watermark"
        );
    }

    /// A schedule dropped from a resubmitted manifest leaves a row behind
    /// that nothing short of retiring the instance would ever delete, and
    /// that `schedules` cannot show, since it reads the plan. The pass that
    /// knows the declared set reclaims it.
    #[tokio::test]
    async fn a_schedule_the_plan_no_longer_declares_loses_its_state_row() {
        let s = service();
        let plan_json = plan_json_one_service("inst-1", "worker", None);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        s.store.record_schedule_started("inst-1", "inst-1/worker", 100, 0).unwrap();

        s.reconcile_instance_pass("inst-1").await;

        assert!(
            s.store.schedule_states("inst-1").unwrap().is_empty(),
            "the plan declares no schedule, so no schedule state may survive the pass"
        );
    }

    /// `last_member_index` is absent, not 0, until a run has happened --
    /// otherwise the round-robin reads a fresh row as "member 0 already
    /// ran" and sends the very first tick of a multi-member service to
    /// member 1.
    #[test]
    fn the_first_tick_of_a_multi_member_service_runs_member_zero() {
        let plan = plan_with_schedule(vec![
            scheduled_service("worker", 0, "* * * * * *"),
            scheduled_service("worker", 1, "* * * * * *"),
        ]);
        let report = report_of(vec![
            scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
            scheduled_health("worker", 1, "did:key:zEdgeB", Signal::Healthy),
        ]);
        let states = BTreeMap::from([(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: 0, ..Default::default() },
        )]);

        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
        match &decisions[0] {
            ScheduleDecision::Run { member_index, .. } => assert_eq!(*member_index, 0),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn selection_rotates_across_healthy_members_on_consecutive_ticks() {
        let plan = plan_with_schedule(vec![
            scheduled_service("worker", 0, "* * * * * *"),
            scheduled_service("worker", 1, "* * * * * *"),
            scheduled_service("worker", 2, "* * * * * *"),
        ]);
        let report = report_of(vec![
            scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
            scheduled_health("worker", 1, "did:key:zEdgeB", Signal::Healthy),
            scheduled_health("worker", 2, "did:key:zEdgeC", Signal::Healthy),
        ]);
        for (last_index, expected_index) in [(0u32, 1u32), (1, 2), (2, 0)] {
            let mut states = BTreeMap::new();
            states.insert(
                "inst-1/worker".to_string(),
                ScheduleState {
                    evaluated_at: 0,
                    last_member_index: Some(last_index),
                    ..Default::default()
                },
            );
            let decisions =
                SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
            match &decisions[0] {
                ScheduleDecision::Run { member_index, .. } => {
                    assert_eq!(*member_index, expected_index, "after member {last_index}");
                }
                other => panic!("expected Run, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_unhealthy_member_is_never_selected_and_does_not_block_the_schedule() {
        let plan = plan_with_schedule(vec![
            scheduled_service("worker", 0, "* * * * * *"),
            scheduled_service("worker", 1, "* * * * * *"),
        ]);
        let report = report_of(vec![
            scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
            scheduled_health(
                "worker",
                1,
                "did:key:zEdgeB",
                Signal::ProbeFailing("down".to_string()),
            ),
        ]);
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: 0, ..Default::default() },
        );
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
        match &decisions[0] {
            ScheduleDecision::Run { member_index, substrate_did, .. } => {
                assert_eq!(*member_index, 0);
                assert_eq!(substrate_did, "did:key:zEdgeA");
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn a_schedule_with_no_healthy_member_advances_its_watermark_and_skips() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * * *")]);
        let report = report_of(vec![scheduled_health(
            "worker",
            0,
            "did:key:zEdgeA",
            Signal::SubstrateUnreachable("down".to_string()),
        )]);
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState { evaluated_at: 0, ..Default::default() },
        );
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
        assert_eq!(
            decisions,
            vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }]
        );
    }

    #[test]
    fn a_schedule_only_update_is_excluded_from_redeploy_but_is_not_a_push_candidate() {
        let old = scheduled_service("worker", 0, "* * * * *");
        let mut new = old.clone();
        new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
        let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
        let (redeploy_exclusions, push_candidates) =
            SupervisorService::classify_update_actions(&[], &actions);
        assert!(redeploy_exclusions.contains("inst-1/worker#0"));
        assert!(push_candidates.is_empty());
    }

    /// The classifier's first call site: the loop's own work list. Testing
    /// the classifier alone says nothing about whether either caller
    /// honours it -- fixing one path and not the other is the exact gap an
    /// earlier review round found.
    #[test]
    fn a_schedule_only_edit_does_not_redeploy_the_service() {
        let old = scheduled_service("worker", 0, "* * * * *");
        let mut new = old.clone();
        new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
        let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
        let (redeploy_exclusions, _) = SupervisorService::classify_update_actions(&[], &actions);

        let needs_work =
            SupervisorService::redeploy_work_list(&BTreeSet::new(), &actions, &redeploy_exclusions);
        assert!(needs_work.is_empty(), "a schedule-only edit is not redeploy work: {needs_work:?}");
    }

    /// Same edit, the other call site: `submit`/`force-reconcile`. The plan
    /// applied must exclude the member, while the plan *journaled* still
    /// carries the whole thing -- a baseline narrowed to this call's own
    /// subset is what makes later passes redeploy everything it left out.
    #[tokio::test]
    async fn a_schedule_only_edit_on_submit_does_not_redeploy_the_service() {
        let s = service();
        let old = scheduled_service("worker", 0, "* * * * *");
        let old_plan = plan_with_schedule(vec![old.clone()]);
        let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
        s.store
            .journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/worker#0",
                Some("edge-1"),
                "did:key:zEdge1",
                ActionState::Completed,
            )
            .unwrap();

        let mut new = old.clone();
        new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
        let plan = plan_with_schedule(vec![new]);

        // No clients: a member that reached `apply_plan` would fail for
        // want of a target and journal `Degraded`, so an `Active` record
        // with no new action row is the direct evidence it was excluded.
        s.apply_with_membership_pushes(&plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
            .await
            .expect("a schedule-only resubmit must not fail for want of a substrate");

        let latest = s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
        assert_eq!(latest.state, DeploymentState::Active);
        assert_eq!(
            latest.plan.services[0].schedule.as_ref().unwrap().cron,
            "0 3 * * *",
            "the journaled baseline must still carry the whole plan, new schedule included"
        );
        let actions = s
            .store
            .journal
            .get_completed_actions_for_instance(&AppInstanceId::new("inst-1"))
            .unwrap();
        assert_eq!(actions.len(), 1, "no second placement action: {actions:?}");
    }

    #[test]
    fn a_simultaneous_schedule_and_membership_edit_is_not_classified_as_membership_only() {
        let old = dependent_service("frontend", "backend");
        let mut new = old.clone();
        new.resolved_dependencies = BTreeMap::from([(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hDepMember2")],
        )]);
        new.schedule = Some(ScheduleSpec {
            cron: "* * * * *".to_string(),
            interface: InterfaceName::new("scheduled-driver"),
            method: "tick".to_string(),
            params: None,
            timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
        });
        assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
        assert!(!SupervisorService::only_schedule_changed(&old, &new));

        let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
        let (redeploy_exclusions, push_candidates) =
            SupervisorService::classify_update_actions(&[], &actions);
        assert!(
            redeploy_exclusions.is_empty(),
            "a schedule change alongside a membership change must not be excluded from redeploy"
        );
        assert!(push_candidates.is_empty());
    }

    #[test]
    fn refuse_unrunnable_schedules_refuses_a_plan_naming_more_scheduled_services_than_the_cap() {
        let services: Vec<PlannedService> = (0..=MAX_SCHEDULED_SERVICES)
            .map(|i| scheduled_service(&format!("worker-{i}"), 0, "* * * * *"))
            .collect();
        let plan = plan_with_schedule(services);
        let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
        assert!(err.contains(&format!("above the cap of {MAX_SCHEDULED_SERVICES}")), "{err}");
    }

    #[test]
    fn refuse_unrunnable_schedules_allows_a_plan_exactly_at_the_cap() {
        let services: Vec<PlannedService> = (0..MAX_SCHEDULED_SERVICES)
            .map(|i| scheduled_service(&format!("worker-{i}"), 0, "* * * * *"))
            .collect();
        let plan = plan_with_schedule(services);
        assert!(SupervisorService::refuse_unrunnable_schedules(&plan).is_ok());
    }

    /// The manifest's own bound is compile-time only, and `submit` takes an
    /// already-compiled plan -- so without this check a hand-edited plan
    /// reproduces the original defect exactly: the runtime clamp is a
    /// `min`, which a zero survives, and the tick is then consumed by a
    /// timeout that elapses before the call starts.
    #[test]
    fn refuse_unrunnable_schedules_refuses_a_submitted_plan_with_a_zero_timeout() {
        let mut svc = scheduled_service("worker", 0, "* * * * *");
        svc.schedule.as_mut().unwrap().timeout_ms = 0;
        let plan = plan_with_schedule(vec![svc]);
        let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
        assert!(err.contains("must be between 1"), "{err}");
    }

    #[test]
    fn refuse_unrunnable_schedules_refuses_a_submitted_plan_above_the_timeout_ceiling() {
        let mut svc = scheduled_service("worker", 0, "* * * * *");
        svc.schedule.as_mut().unwrap().timeout_ms = MAX_SCHEDULE_TIMEOUT_MS + 1;
        let plan = plan_with_schedule(vec![svc]);
        let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
        assert!(err.contains(&format!("{MAX_SCHEDULE_TIMEOUT_MS}ms")), "{err}");
    }

    /// An unparseable cron is deliberately *not* a submission-level
    /// refusal: it degrades to the watermark branch, which skips that one
    /// schedule and leaves the rest of the instance reconciling. Pinned so
    /// the asymmetry with the budget above is a decision, not a gap.
    #[test]
    fn refuse_unrunnable_schedules_allows_a_plan_whose_cron_does_not_parse() {
        let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "not a cron")]);
        assert!(SupervisorService::refuse_unrunnable_schedules(&plan).is_ok());
    }

    /// A fake `SubstrateActor` exercising only `run_scheduled`, for
    /// `run_due_schedules`'s own tests. `error`/`delay` are behind a
    /// `Mutex` so a test can flip the outcome between two calls in the
    /// same actor, the same shape `DurableTestActor` uses.
    #[derive(Debug, Default)]
    struct ScheduledActor {
        error: Mutex<Option<String>>,
        delay: Option<Duration>,
    }

    #[async_trait::async_trait]
    impl SubstrateActor for ScheduledActor {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by schedule tests")
        }
        async fn run_scheduled(
            &self,
            _service_id: String,
            _generation: u64,
            _interface: String,
            _method: String,
            _params_json: Option<String>,
        ) -> Result<(), String> {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if let Some(e) = self.error.lock().unwrap().clone() {
                return Err(e);
            }
            Ok(())
        }
    }

    fn run_decision(logical_ref: &str, service_id: &str, substrate_did: &str) -> ScheduleDecision {
        ScheduleDecision::Run {
            logical_ref: logical_ref.to_string(),
            service_id: service_id.to_string(),
            substrate_did: substrate_did.to_string(),
            member_index: 0,
            schedule: ScheduleSpec {
                cron: "* * * * *".to_string(),
                interface: InterfaceName::new("scheduled-driver"),
                method: "tick".to_string(),
                params: None,
                timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
            },
        }
    }

    #[tokio::test]
    async fn a_failed_run_raises_scheduled_run_failed_and_the_next_success_clears_it() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        let actor = Arc::new(ScheduledActor::default());
        *actor.error.lock().unwrap() = Some("boom".to_string());
        let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor.clone()))]);
        let did_to_alias = edge_1_alias();
        let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
        let mut opened = Vec::new();

        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions,
            &did_to_alias,
            &actors,
            0,
            NOW,
            &mut opened,
        )
        .await;

        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(
            active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed
                && a.substrate_did == SCHEDULE_SUBSTRATE_DID),
            "a failed run must raise ScheduledRunFailed under the sentinel, not the member's own \
             substrate: {active:?}"
        );
        assert_eq!(opened, vec![(AlertKind::ScheduledRunFailed, "inst-1/worker".to_string())]);

        *actor.error.lock().unwrap() = None;
        let mut opened2 = Vec::new();
        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions,
            &did_to_alias,
            &actors,
            0,
            NOW + 60,
            &mut opened2,
        )
        .await;
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(
            !active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed),
            "the next successful run must clear the alert: {active:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_run_is_never_enqueued_onto_the_outbox() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        let actor = Arc::new(ScheduledActor::default());
        *actor.error.lock().unwrap() = Some("boom".to_string());
        let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
        let did_to_alias = edge_1_alias();
        let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
        let mut opened = Vec::new();

        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions,
            &did_to_alias,
            &actors,
            0,
            NOW,
            &mut opened,
        )
        .await;

        assert_eq!(s.store.queue.pending_count().unwrap(), 0);
        assert!(s.store.queue.dead_letters().unwrap().is_empty());
    }

    /// A direct proof of the watermark's ordering: `record_schedule_started`
    /// must have already landed by the time the target is called, not after --
    /// checked from *inside* the call itself, before its own outcome is
    /// known.
    #[derive(Debug)]
    struct AssertsStartedBeforeCallActor {
        store: SupervisorStore,
        expected_run_at: i64,
    }

    #[async_trait::async_trait]
    impl SubstrateActor for AssertsStartedBeforeCallActor {
        async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
            unimplemented!("not exercised by this test")
        }
        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("not exercised by this test")
        }
        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("not exercised by this test")
        }
        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by this test")
        }
        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("not exercised by this test")
        }
        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by this test")
        }
        async fn run_scheduled(
            &self,
            _service_id: String,
            _generation: u64,
            _interface: String,
            _method: String,
            _params_json: Option<String>,
        ) -> Result<(), String> {
            let states = self.store.schedule_states("inst-1").unwrap();
            let state = states
                .get("inst-1/worker")
                .expect("record_schedule_started must have written before the call");
            assert_eq!(state.last_run_at, Some(self.expected_run_at));
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_run_is_recorded_before_the_call_so_a_crash_mid_run_skips_the_tick() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        let actor = Arc::new(AssertsStartedBeforeCallActor {
            store: s.store.clone(),
            expected_run_at: NOW as i64,
        });
        let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
        let did_to_alias = edge_1_alias();
        let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
        let mut opened = Vec::new();

        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions,
            &did_to_alias,
            &actors,
            0,
            NOW,
            &mut opened,
        )
        .await;
    }

    #[tokio::test]
    async fn a_failure_on_one_member_is_cleared_by_a_success_on_another_members_substrate() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        let failing_actor = Arc::new(ScheduledActor::default());
        *failing_actor.error.lock().unwrap() = Some("boom".to_string());
        let succeeding_actor = Arc::new(ScheduledActor::default());
        let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> = BTreeMap::from([
            (SubstrateAlias::new("edge-a"), deploy::build_actor(failing_actor)),
            (SubstrateAlias::new("edge-b"), deploy::build_actor(succeeding_actor)),
        ]);
        let did_to_alias = BTreeMap::from([
            ("did:key:zEdgeA".to_string(), "edge-a".to_string()),
            ("did:key:zEdgeB".to_string(), "edge-b".to_string()),
        ]);
        let mut opened = Vec::new();

        let decisions_a = vec![run_decision("inst-1/worker", "did:key:hworkerA", "did:key:zEdgeA")];
        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions_a,
            &did_to_alias,
            &actors,
            0,
            NOW,
            &mut opened,
        )
        .await;
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed));

        let decisions_b = vec![run_decision("inst-1/worker", "did:key:hworkerB", "did:key:zEdgeB")];
        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions_b,
            &did_to_alias,
            &actors,
            0,
            NOW + 60,
            &mut opened,
        )
        .await;
        let active = s.store.alerts.active(&instance_id).unwrap();
        assert!(
            !active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed),
            "a success on a different member's substrate must clear the sentinel-keyed alert: \
             {active:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_schedule_timeout_is_clamped_to_the_ceiling() {
        let s = service();
        let instance_id = AppInstanceId::new("inst-1");
        let actor =
            Arc::new(ScheduledActor { delay: Some(Duration::from_secs(60)), ..Default::default() });
        let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
            BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
        let did_to_alias = edge_1_alias();
        let decisions = vec![ScheduleDecision::Run {
            logical_ref: "inst-1/worker".to_string(),
            service_id: "did:key:hworker0".to_string(),
            substrate_did: "did:key:zEdge1".to_string(),
            member_index: 0,
            schedule: ScheduleSpec {
                cron: "* * * * *".to_string(),
                interface: InterfaceName::new("scheduled-driver"),
                method: "tick".to_string(),
                params: None,
                // Above the ceiling on purpose -- the ceiling must win.
                timeout_ms: 100_000,
            },
        }];
        let mut opened = Vec::new();

        let start = tokio::time::Instant::now();
        s.run_due_schedules(
            &instance_id,
            "inst-1",
            &decisions,
            &did_to_alias,
            &actors,
            0,
            NOW,
            &mut opened,
        )
        .await;
        let elapsed = start.elapsed();

        assert!(
            elapsed >= SCHEDULED_RUN_CEILING && elapsed < Duration::from_secs(60),
            "the run must time out at the 30s ceiling, not the configured 100s or the actor's own \
             60s delay: {elapsed:?}"
        );
        let states = s.store.schedule_states("inst-1").unwrap();
        assert!(
            states
                .get("inst-1/worker")
                .unwrap()
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("timed out"),
            "{:?}",
            states.get("inst-1/worker")
        );
    }

    // ── The `schedules` operator verb ────────────────────────────────────

    fn plan_json_with_schedule(service_name: &str, master_did: &str) -> String {
        serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": master_did,
                "logical_ref": format!("inst-1/{service_name}"),
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "resolved_dependencies": {},
                "topology_mode": "singleton",
                "schedule": {
                    "cron": "* * * * *",
                    "interface": "scheduled-driver",
                    "method": "tick"
                }
            }]
        })
        .to_string()
    }

    #[tokio::test]
    async fn schedules_lists_a_declared_schedule_that_has_never_run() {
        let s = service();
        s.store
            .submit(
                "inst-1",
                &plan_json_with_schedule("worker", "did:key:hworker0"),
                "{}",
                "did:key:owner",
                0,
            )
            .unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "schedules",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let tasks: Vec<ScheduledTask> = serde_json::from_value(res.payload).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].logical_ref, "inst-1/worker");
        assert_eq!(tasks[0].cron, "* * * * *");
        assert_eq!(tasks[0].interface, "scheduled-driver");
        assert_eq!(tasks[0].method, "tick");
        assert_eq!(tasks[0].evaluated_at, 0, "a schedule never evaluated must read as 0");
        assert_eq!(tasks[0].last_run_at, None);
        assert_eq!(tasks[0].last_member_index, None);
        assert_eq!(tasks[0].last_error, None);
    }

    #[tokio::test]
    async fn schedules_reports_the_member_and_time_of_the_last_run() {
        let s = service();
        s.store
            .submit(
                "inst-1",
                &plan_json_with_schedule("worker", "did:key:hworker0"),
                "{}",
                "did:key:owner",
                0,
            )
            .unwrap();
        s.store.record_schedule_started("inst-1", "inst-1/worker", 12_345, 2).unwrap();

        let res = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "schedules",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
        let tasks: Vec<ScheduledTask> = serde_json::from_value(res.payload).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].evaluated_at, 12_345);
        assert_eq!(tasks[0].last_run_at, Some(12_345));
        assert_eq!(tasks[0].last_member_index, Some(2));
    }

    // ── Tier 2 `resolve` ──────────────────────────────────────

    fn plan_json_n_member_service_with_vis(
        instance: &str,
        service_name: &str,
        mode: &str,
        n: u32,
        visibility: &str,
        topology_visibility: &str,
    ) -> String {
        let services: Vec<_> = (0..n)
            .map(|i| {
                serde_json::json!({
                    "service_id": format!("did:key:hMember{i}"),
                    "logical_ref": format!("{instance}/{service_name}"),
                    "substrate": "edge-1",
                    "service_type": "tcp", "source": "127.0.0.1:9000",
                    "rotation_policy": "none",
                    "resolved_dependencies": {},
                    "topology_mode": mode,
                    "member_index": i,
                    "visibility": visibility,
                    "topology_visibility": topology_visibility,
                })
            })
            .collect();
        serde_json::json!({
            "app_instance_id": instance,
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": services,
        })
        .to_string()
    }

    fn plan_json_n_member_service(
        instance: &str,
        service_name: &str,
        mode: &str,
        n: u32,
    ) -> String {
        plan_json_n_member_service_with_vis(
            instance,
            service_name,
            mode,
            n,
            "private",
            "restricted",
        )
    }

    /// Directly submits desired state and adopts an app master, bypassing
    /// the RPC `submit`/`adopt` pipeline's mint/apply machinery -- this
    /// slice's `resolve` reads only the store and the vault, so a fully
    /// applied plan (which needs a live substrate connection) is not
    /// needed to exercise it. Returns the app master DID `resolve` is
    /// looked up by.
    async fn adopted_instance(s: &SupervisorService, instance_id: &str, plan_json: &str) -> String {
        s.store.submit(instance_id, plan_json, "{}", "did:key:owner", 0).unwrap();
        let (app_did, _) = keys::app_master(&s.vault, instance_id).await.unwrap();
        s.store.record_adopt(instance_id, 1, &app_did).unwrap();
        app_did
    }

    /// A caller holding `supervisor/resolve` on exactly `synapp:<app_did>`
    /// -- not `substrate/admin`, so this is an honest reading of the
    /// reference scenario's "a caller outside the app instance".
    fn resolve_grant(caller_did: &str, app_did: &str) -> CallerContext {
        use syneroym_rpc::{Capability, SessionContext};
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: caller_did.to_string(),
                capabilities: vec![Capability {
                    with: ResourceUri(format!("synapp:{app_did}")),
                    can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        }
    }

    fn caller_with_no_capabilities(caller_did: &str) -> CallerContext {
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: Default::default(),
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    fn decode_signed_document(payload: Value) -> SignedTopologyDocument {
        serde_json::from_value(payload).unwrap()
    }

    /// The document names every member in member-index order.
    #[tokio::test]
    async fn resolve_returns_a_document_naming_every_member_master_did_in_index_order() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "redundant", 3);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let resp = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap();
        let signed = decode_signed_document(resp.payload);

        assert_eq!(
            signed.document.members,
            vec![
                ServiceId::new("did:key:hMember0"),
                ServiceId::new("did:key:hMember1"),
                ServiceId::new("did:key:hMember2"),
            ]
        );
        assert_eq!(signed.document.mode, TopologyMode::Redundant);
        assert!(signed.verify(&AppDid::new(app_did)).is_ok());
    }

    /// Test 78: the property phase 3's D-S3-5 check depends on -- the
    /// signed document never echoes the hash back, always the real name,
    /// even when the caller supplied the hash. Also pins matrix row 10
    /// (S3's second named test): the epoch is still carried and preserved
    /// on a hashed request.
    #[tokio::test]
    async fn resolve_answers_a_hashed_service_name_with_a_document_naming_the_real_name() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
        let name_hash = util::short_hash("backend");

        let by_name = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap();
        let by_hash = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, name_hash]),
        )
        .await
        .unwrap();

        let signed_by_hash = decode_signed_document(by_hash.payload);
        let signed_by_name = decode_signed_document(by_name.payload);
        assert_eq!(signed_by_hash.document.service_name.as_str(), "backend");
        assert_eq!(signed_by_hash.document.epoch, signed_by_name.document.epoch);
        assert!(signed_by_hash.verify(&AppDid::new(app_did)).is_ok());
    }

    /// Matrix row 7: a caller holding no grant for this app is refused.
    #[tokio::test]
    async fn resolve_refuses_a_caller_holding_no_grant_for_this_app() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let err = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zOutsideCaller"),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE);
    }

    /// Test 24: An ungranted caller resolving an `open` service receives the
    /// signed document.
    #[tokio::test]
    async fn resolve_open_service_answers_ungranted_caller() {
        let s = service();
        let plan_json = plan_json_n_member_service_with_vis(
            "inst-1",
            "backend",
            "singleton",
            1,
            "public",
            "open",
        );
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let resp = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zOutsideCaller"),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap();
        let signed = decode_signed_document(resp.payload);
        assert_eq!(signed.document.members, vec![ServiceId::new("did:key:hMember0")]);
        assert_eq!(signed.document.mode, TopologyMode::Singleton);
        assert!(signed.verify(&AppDid::new(app_did)).is_ok());
    }

    /// Test 26: An ungranted caller naming a non-existent service gets the same
    /// refusal as an unauthorized call.
    #[tokio::test]
    async fn resolve_open_service_nonexistent_service_refuses_identically_for_ungranted_caller() {
        let s = service();
        let plan_json = plan_json_n_member_service_with_vis(
            "inst-1",
            "backend",
            "singleton",
            1,
            "public",
            "open",
        );
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let err = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zOutsideCaller"),
            "resolve",
            serde_json::json!([app_did, "nonexistent"]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE);
    }

    /// Test 27: An ungranted caller resolving an `open` service on a retired
    /// instance is refused.
    #[tokio::test]
    async fn resolve_open_service_on_retired_instance_is_refused() {
        let s = service();
        let plan_json = plan_json_n_member_service_with_vis(
            "inst-1",
            "backend",
            "singleton",
            1,
            "public",
            "open",
        );
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
        s.store.retire("inst-1").unwrap();

        let err = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zOutsideCaller"),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE);
    }

    /// Test 28: A granted caller naming a non-existent service gets
    /// InvalidParams, not denied.
    #[tokio::test]
    async fn resolve_granted_caller_naming_nonexistent_service_returns_invalid_params() {
        let s = service();
        let plan_json = plan_json_n_member_service_with_vis(
            "inst-1",
            "backend",
            "singleton",
            1,
            "public",
            "open",
        );
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let err = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, "nonexistent"]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RpcError::InvalidParams(_)), "expected InvalidParams, got {err:?}");
    }

    /// Test 29: The document served to an ungranted `open` caller is
    /// byte-identical to the one served to a granted caller.
    #[tokio::test]
    async fn resolve_open_service_served_to_ungranted_caller_is_identical_to_granted_caller() {
        let s = service();
        let plan_json = plan_json_n_member_service_with_vis(
            "inst-1",
            "backend",
            "singleton",
            1,
            "public",
            "open",
        );
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let granted_resp = dispatch(
            &s,
            resolve_grant("did:key:zGrantedCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap();
        let ungranted_resp = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zUngrantedCaller"),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap();

        let granted_signed = decode_signed_document(granted_resp.payload);
        let ungranted_signed = decode_signed_document(ungranted_resp.payload);

        assert_eq!(granted_signed.document, ungranted_signed.document);
        assert_eq!(granted_signed.signature, ungranted_signed.signature);
    }

    /// The probing guard: an unknown app and an unauthorized caller are
    /// reported identically, asserted on the exact error string.
    /// `record_adopt` writes the row's `app_master_did`
    /// directly (bypassing the vault) so both branches can share one
    /// literal DID and one literal caller DID, making the two error
    /// strings byte-comparable.
    #[tokio::test]
    async fn resolve_reports_an_unknown_app_and_an_unauthorized_caller_identically() {
        const SHARED_DID: &str = "did:key:zSharedForComparison";
        const SHARED_CALLER: &str = "did:key:zOutsideCaller";

        // Branch 1: no instance anywhere claims this DID.
        let unknown_app = service();
        let err_unknown = dispatch(
            &unknown_app,
            resolve_grant(SHARED_CALLER, SHARED_DID),
            "resolve",
            serde_json::json!([SHARED_DID, "backend"]),
        )
        .await
        .unwrap_err();

        // Branch 2: the app exists, under the same DID, but the caller
        // holds no grant for it.
        let unauthorized = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        unauthorized.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        unauthorized.store.record_adopt("inst-1", 1, SHARED_DID).unwrap();
        let err_unauthorized = dispatch(
            &unauthorized,
            caller_with_no_capabilities(SHARED_CALLER),
            "resolve",
            serde_json::json!([SHARED_DID, "backend"]),
        )
        .await
        .unwrap_err();

        assert_eq!(err_unknown.to_string(), err_unauthorized.to_string());
        assert!(err_unknown.to_string().contains(SHARED_DID));
        assert!(err_unknown.to_string().contains(SHARED_CALLER));
    }

    /// A refusal carries no member DIDs at all -- the document is built
    /// whole or not at all, and the authorization check runs before it is
    /// built.
    #[tokio::test]
    async fn a_refused_resolve_carries_no_member_dids_at_all() {
        let s = service();
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hVerySecretMember",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton",
            }],
        })
        .to_string();
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let err = dispatch(
            &s,
            caller_with_no_capabilities("did:key:zOutsideCaller"),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap_err();
        assert!(!err.to_string().contains("hVerySecretMember"), "{err}");
    }

    /// A locked vault fails loudly and names `inject-kek`, never a silent
    /// empty answer.
    #[tokio::test]
    async fn resolve_on_a_locked_vault_fails_loudly_and_names_inject_kek() {
        let s = service_with_locked_vault();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
        s.store.record_adopt("inst-1", 1, "did:key:zPlaceholderAppMaster").unwrap();

        let err = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", "did:key:zPlaceholderAppMaster"),
            "resolve",
            serde_json::json!(["did:key:zPlaceholderAppMaster", "backend"]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("inject-kek"), "{err}");
        let active = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
        assert!(active.iter().any(|a| a.kind == AlertKind::VaultLocked));
    }

    /// The supervisor signs once per `(service, epoch)` and serves the
    /// cached document afterwards -- asserted on the signature bytes being
    /// identical across two calls.
    #[tokio::test]
    async fn resolve_signs_once_per_epoch_and_serves_the_cached_document_afterwards() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
        let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

        let first = decode_signed_document(
            dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        let second = decode_signed_document(
            dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_eq!(first.signature, second.signature, "a cache hit must not re-sign");
        assert_eq!(first.document.issued_at, second.document.issued_at);
    }

    /// D-S2-6: a cached document is re-signed once less than half its
    /// validity remains, so a served copy always outlives a caller's own
    /// cache TTL rather than being served right up to the moment it
    /// expires.
    #[tokio::test]
    async fn a_nearly_expired_cached_document_is_re_signed_rather_than_served() {
        let s = Fixture { topology_document_not_after_secs: Some(2), ..Fixture::default() }.build();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
        let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

        let first = decode_signed_document(
            dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );

        // Past half of the 2s validity, with no membership change.
        tokio::time::sleep(Duration::from_millis(1_100)).await;

        let second = decode_signed_document(
            dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_ne!(first.signature, second.signature, "a nearly-expired document must re-sign");
        assert!(second.document.issued_at >= first.document.issued_at);
        assert!(second.document.not_after > first.document.not_after);
        assert_eq!(first.document.epoch, second.document.epoch, "membership did not change");
    }

    /// A membership change re-signs the document at the new epoch. The
    /// store's own epoch/fingerprint update (which
    /// `handle_submit` performs after a real resubmit) is driven directly
    /// here, since a real resubmit needs a live substrate connection this
    /// unit test has no reason to stand up.
    #[tokio::test]
    async fn a_membership_change_re_signs_the_document_at_the_new_epoch() {
        let s = service();
        let one_member = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &one_member).await;
        let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

        let first = decode_signed_document(
            dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_eq!(first.document.epoch, TopologyEpoch(1));

        // A scale-out: the stored plan now names two members, and the
        // fingerprint is advanced the way `handle_submit` would after a
        // real resubmit.
        let two_members = plan_json_n_member_service("inst-1", "backend", "redundant", 2);
        s.store.submit("inst-1", &two_members, "{}", "did:key:owner", 1).unwrap();
        let plan = DeploymentPlan::from_json(&two_members).unwrap();
        let topo = topology::service_topology(&plan, &LogicalServiceName::new("backend")).unwrap();
        let fp = topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref());
        s.store.record_topology_fingerprint("inst-1", "backend", &fp).unwrap();

        let second = decode_signed_document(
            dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_eq!(second.document.epoch, TopologyEpoch(2));
        assert_eq!(second.document.members.len(), 2);
        assert_ne!(first.signature, second.signature);
    }

    /// F1: the document cache is keyed only by `(app_instance_id,
    /// service_name)`, so a handover that reassigns `app_master_did`
    /// (`import-master` + `adopt`, simulated here at the store level --
    /// the same shortcut the test above uses for a resubmit) must not let
    /// a caller asking for the *new* DID be served a document cached
    /// under the *old* one, which would carry the wrong `app_did` and
    /// fail every caller's `verify`.
    #[tokio::test]
    async fn a_handover_to_a_different_app_did_does_not_serve_the_previous_masters_cached_document()
    {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let old_app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        let first = decode_signed_document(
            dispatch(
                &s,
                resolve_grant("did:key:zOutsideCaller", &old_app_did),
                "resolve",
                serde_json::json!([old_app_did, "backend"]),
            )
            .await
            .unwrap()
            .payload,
        );
        assert_eq!(first.document.app_did, AppDid::new(old_app_did.clone()));

        // The row-level effect of `import-master` + `adopt`: the same
        // instance, a different recorded app master DID, no vault key
        // rotation (a real handover also imports a new vault key -- the
        // resulting mismatch, and its alert, are `resolve_on_a_locked_
        // vault_fails_loudly_and_names_inject_kek`'s sibling test, not
        // this one's concern). Generation is held at `1`, unchanged from
        // `adopted_instance`'s own call, so this isolates the app DID as
        // the only thing that moved -- `record_adopt` is a plain `UPDATE`
        // with no monotonic guard on generation, so re-recording the same
        // one is accepted. Bumping it here as well would let the
        // `generation` clause alone force the cache miss this test means
        // to pin on `app_did`.
        let new_app_did = "did:key:zHandedOverMaster";
        s.store.record_adopt("inst-1", 1, new_app_did).unwrap();

        let err = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", new_app_did),
            "resolve",
            serde_json::json!([new_app_did, "backend"]),
        )
        .await
        .unwrap_err();
        // Proves the stale document was not served: a cache hit would
        // have returned `Ok` with `first`'s bytes. Instead the fresh-sign
        // path ran and correctly refused, since the vault's real key
        // still derives `old_app_did`, not `new_app_did`.
        assert!(err.to_string().contains("does not match its recorded app master DID"), "{err}");
    }

    /// F4: the cache hit condition did not compare `generation`, so a
    /// second `adopt` of the same app master -- no membership change, no
    /// `AppDid` change -- could serve a document carrying the *previous*
    /// generation, the field ADR-0022 §2 gives a reader to tell two
    /// supervisors' documents apart.
    #[tokio::test]
    async fn a_generation_bump_with_no_membership_change_is_not_served_from_a_stale_cache() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
        let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

        let first = decode_signed_document(
            dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_eq!(first.document.generation, 1);

        // Same app master, same plan -- only the generation moves, the
        // way a second `adopt` of an already-adopted instance would.
        s.store.record_adopt("inst-1", 2, &app_did).unwrap();

        let second = decode_signed_document(
            dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
                .await
                .unwrap()
                .payload,
        );
        assert_eq!(second.document.generation, 2);
        assert_ne!(first.signature, second.signature, "a stale generation must not be served");
        assert_eq!(first.document.epoch, second.document.epoch, "membership did not change");
    }

    /// F2 (and F6, the same fix): `initialise_topology_epoch`'s
    /// insert-only form can never correct an existing row's fingerprint,
    /// so a row left holding one that disagrees with the real plan --
    /// whether from an earlier `submit`'s fingerprint write landing
    /// wrong, or a genuine concurrent `submit` -- used to exhaust both
    /// lock-free attempts and fail permanently. The locked repair path
    /// settles it instead.
    #[tokio::test]
    async fn resolve_repairs_a_topology_epoch_row_stuck_on_the_wrong_fingerprint() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

        // A row that disagrees with what `service_topology` actually
        // computes for the stored plan -- the state a lock-free
        // `resolve` can never fix on its own.
        s.store.record_topology_fingerprint("inst-1", "backend", "garbage-fingerprint").unwrap();

        let doc = decode_signed_document(
            dispatch(
                &s,
                resolve_grant("did:key:zOutsideCaller", &app_did),
                "resolve",
                serde_json::json!([app_did, "backend"]),
            )
            .await
            .unwrap()
            .payload,
        );
        // The repair path's advancing write bumps the epoch past the
        // garbage row's `1`, since the stored fingerprint did not match.
        assert_eq!(doc.document.epoch, TopologyEpoch(2));
    }

    /// The WIT record and the serde struct are two descriptions of one
    /// wire format, and nothing else stops them drifting.
    #[test]
    fn the_resolve_payloads_json_keys_match_the_wit_records_field_names() {
        let (resolve, iface_id) = supervisor_interface();
        let iface = &resolve.interfaces[iface_id];
        let record_ty = *iface.types.get("topology-document").expect("no topology-document type");
        let wit_parser::TypeDefKind::Record(record) = &resolve.types[record_ty].kind else {
            panic!("topology-document is not a record");
        };
        let wit_fields: BTreeSet<String> =
            record.fields.iter().map(|f| f.name.replace('-', "_")).collect();

        // The fixture must declare a sharding_strategy: it is
        // skip_serializing_if = "Option::is_none", so a fixture without
        // one omits the key and the comparison would read a real name
        // match as a mismatch.
        let doc = TopologyDocument {
            app_instance_id: AppInstanceId::new("inst-1"),
            app_did: AppDid::new("did:key:zApp"),
            service_name: LogicalServiceName::new("backend"),
            mode: TopologyMode::Sharded,
            members: vec![ServiceId::new("did:key:zM0")],
            sharding_strategy: Some(ShardingStrategy::HashSharding),
            epoch: TopologyEpoch(1),
            generation: 0,
            issued_at: 0,
            not_after: 0,
            cache_ttl_ms: 0,
        };
        let value = serde_json::to_value(&doc).unwrap();
        let json_keys: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();

        assert_eq!(wit_fields, json_keys);
    }

    /// An instance with no app master DID yet, the same skip
    /// `refresh_due_app_tier1_record` makes.
    #[tokio::test]
    async fn resolve_is_refused_for_an_instance_that_has_no_app_master_did() {
        let s = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        // Submitted, never adopted -- `app_master_did` stays empty.
        s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

        let err = dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", "did:key:zNeverAssigned"),
            "resolve",
            serde_json::json!(["did:key:zNeverAssigned", "backend"]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE);
    }

    /// `resolve` answers for a paused instance and refuses for a retired
    /// one, in one test because the decision is the contrast.
    #[tokio::test]
    async fn resolve_answers_for_a_paused_instance_and_refuses_for_a_retired_one() {
        let paused = service();
        let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
        let paused_did = adopted_instance(&paused, "inst-1", &plan_json).await;
        paused.store.pause("inst-1").unwrap();
        let resp = dispatch(
            &paused,
            resolve_grant("did:key:zOutsideCaller", &paused_did),
            "resolve",
            serde_json::json!([paused_did, "backend"]),
        )
        .await;
        assert!(resp.is_ok(), "pause stops the resident loop, not the members");

        let retired = service();
        let retired_did = adopted_instance(&retired, "inst-1", &plan_json).await;
        retired.store.retire("inst-1").unwrap();
        let err = dispatch(
            &retired,
            resolve_grant("did:key:zOutsideCaller", &retired_did),
            "resolve",
            serde_json::json!([retired_did, "backend"]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE);
    }

    /// Following `refuse_replicas_above_cap`'s existing refuse/allow pair
    /// exactly.
    #[test]
    fn refuse_unshardable_plan_refuses_a_hand_authored_range_sharding_plan() {
        use syneroym_app_orchestration::resolver::{RangeChunk, RangeRoutingTable};

        let mut svc = dependent_service("backend", "unrelated");
        svc.topology_mode = TopologyMode::Sharded;
        svc.sharding_strategy = Some(ShardingStrategy::RangeSharding(RangeRoutingTable {
            chunks: vec![RangeChunk {
                start_key: None,
                end_key: None,
                target: ServiceId::new("did:key:hShard0"),
            }],
        }));
        let mut svc2 = svc.clone();
        svc2.member_index = 1;
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![svc, svc2],
        };
        let err = SupervisorService::refuse_unshardable_plan(&plan).unwrap_err();
        assert!(err.contains("range_sharding"), "{err}");
    }

    #[test]
    fn refuse_unshardable_plan_refuses_a_strategy_over_a_single_member() {
        let mut svc = dependent_service("backend", "unrelated");
        svc.topology_mode = TopologyMode::Sharded;
        svc.sharding_strategy = Some(ShardingStrategy::HashSharding);
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![svc],
        };
        let err = SupervisorService::refuse_unshardable_plan(&plan).unwrap_err();
        assert!(err.contains("one member"), "{err}");
    }

    #[test]
    fn refuse_unshardable_plan_allows_a_plan_with_no_strategy() {
        let svc = dependent_service("backend", "unrelated");
        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services: vec![svc],
        };
        assert!(SupervisorService::refuse_unshardable_plan(&plan).is_ok());
    }

    /// The refusal runs beside its two siblings, ahead of `store.submit`,
    /// so nothing durable is written -- the test that makes the WIT's
    /// `option<string>` a checked property rather than a comment.
    #[tokio::test]
    async fn a_submitted_plan_declaring_range_sharding_is_refused_before_anything_is_stored() {
        let s = service();
        let plan_json = serde_json::json!({
            "app_instance_id": "inst-1",
            "blueprint_id": "syneroym:test",
            "version": "1.0.0",
            "services": [{
                "service_id": "did:key:hFabricated0",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "sharded",
                "sharding_strategy": {"range_sharding": {"chunks": [
                    {"start_key": null, "end_key": null, "target": "did:key:hShard0"}
                ]}},
            }, {
                "service_id": "did:key:hFabricated1",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "sharded",
                "member_index": 1,
                "sharding_strategy": {"range_sharding": {"chunks": [
                    {"start_key": null, "end_key": null, "target": "did:key:hShard0"}
                ]}},
            }],
        })
        .to_string();

        let err = dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": plan_json,
                "inventory_json": "{}",
                "generation": 0,
            }]),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("range_sharding"), "{err}");
        assert!(s.store.get("inst-1").unwrap().is_none(), "nothing must be stored on refusal");
    }
}

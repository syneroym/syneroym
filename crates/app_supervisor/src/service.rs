//! The `supervisor` `NativeService`: dispatches every verb in
//! `supervisor.wit` (submit / adopt / release / pause / resume / retire /
//! force-reconcile / export-master / import-master / status / alerts).
//!
//! Every verb gates on `substrate/admin` on this supervisor's own node:
//! submitting desired state hands the supervisor deploy authority
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
    /// only `resolved_dependencies`: a membership
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
    /// This member's own `MemberRef` display string -- what every
    /// store/alert call this candidate feeds keys on.
    member_ref: String,
    service_name: String,
    /// The member master DID -- what the certificate names and what the
    /// substrate knows the service by.
    service_id: String,
    substrate_did: String,
    /// Carried so a failed renewal can tell "not yet expired" from "already
    /// expired" without re-reading the report.
    expires_at: u64,
    /// This member's ordinal -- what `keys::master_for_member` reads
    /// instead of hardcoding `0`, so member N's renewal signs with member
    /// N's own master.
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

/// How the queue worker reconnects to a claimed item's target substrate.
/// A trait rather than a direct call to
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
/// to an already-pending queue item". Two review passes deep: the first
/// found the epoch-skew this sentinel was meant to fix; a second found the
/// sentinel itself was ambiguous with a real, empty success, permanently
/// downgrading a service that had simply lost its last dependency.
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
    /// This node's shared broker, registered under
    /// `SUPERVISOR_DISPATCH_ID` (`runtime.rs`) so `record_report`'s caller
    /// can publish a newly-opened alert without a deployed service in the
    /// way.
    messaging_broker: Arc<MqttBroker>,
    /// `SupervisorRole.alert_topic` (default `supervisor/alerts`) -- the
    /// topic prefix, joined with the app instance id at publish time.
    alert_topic: String,
    /// `SupervisorRole.poll_interval_secs` (default 30) -- the resident
    /// loop's `tokio::interval` period.
    poll_interval_secs: u64,
    /// `SupervisorRole.max_restart_attempts` (default 3) -- the bounded
    /// restart-in-place ceiling.
    max_restart_attempts: u32,
    /// `SupervisorRole.restart_backoff_secs` (default 30) -- minimum wait
    /// between two restart attempts for one service.
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
    /// `SupervisorRole.queue_tick_secs` (default 5s) -- the durable outbox
    /// worker's own `tokio::interval` period, independent of
    /// `poll_interval_secs`: the recovery budget is one worker tick, not
    /// one poll interval.
    queue_tick_secs: u64,
    /// How the queue worker reconnects to a claimed item's target
    /// substrate. `LiveQueueConnector` in production; a test
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
    /// writes) or `status`/`alerts` (reads).
    /// Per-instance rather than global so one unreachable substrate cannot
    /// stall every other instance's loop pass.
    instance_locks: DashMap<String, Arc<AsyncMutex<()>>>,
    /// Unix-seconds timestamp of the last time the *resident loop*
    /// finished a reconcile pass for this instance, keyed by
    /// `app_instance_id` -- distinct from `status`'s own on-demand health
    /// sweep, which does not write here (this field used to be hardcoded
    /// `None` under a comment claiming no loop existed yet to fill it).
    /// In-memory only, not persisted: a
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
    /// dropped mid-pass. The `JoinHandle`
    /// itself is held by `RuntimeServices`, which awaits it after calling
    /// `shutdown` -- cancelling alone does not wait for the pass in
    /// flight to actually finish closing its clients.
    cancellation_token: CancellationToken,
    /// Test-only: keeps a fixture-built service's backing directory alive
    /// for exactly this service's own lifetime.
    /// `Fixture::build_with_key_store` needs the directory to survive past
    /// its own return for the vault's encrypted-mint path to work at all
    /// (an earlier fix that instead called `.keep()` on the `TempDir`,
    /// unconditionally leaking it, is the bug this replaced); tying its
    /// lifetime to the service it backs, rather than never dropping it,
    /// restores ordinary cleanup while keeping that fix.
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
        // per-request latency the re-sign-at-half-validity cache exists to
        // remove. Same clamp shape as `max_renewals_per_pass` above.
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
        // A `cache_ttl` at or above half of `not_after` breaks the property
        // that a served copy always outlives the caller's own cache TTL --
        // a reader that re-asks exactly on the advised TTL could then read
        // a document that already expired.
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
    /// shared by every later caller that names the same instance.
    /// `DashMap::entry` takes its own internal shard lock
    /// only for the duration of the lookup/insert, not for the mutex's
    /// own hold time -- what the caller does with the returned `Arc`
    /// afterward is independent of it.
    fn instance_lock(&self, app_instance_id: &str) -> Arc<AsyncMutex<()>> {
        self.instance_locks
            .entry(app_instance_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// The resident reconcile loop.
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

    /// `MissedTickBehavior::Skip`: a pass that outruns `poll_interval_secs`
    /// against a slow substrate drops the tick it overran instead of
    /// firing a queued burst once it finally returns. Its own function so
    /// this one configuration decision is directly testable under a paused
    /// clock, with no pass or network involved.
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

    /// The durable outbox worker: claims items due from this supervisor's
    /// own queue and replays each `write_bindings` against its target
    /// substrate. Spawned beside the resident loop
    /// (`RuntimeServices::run_until_shutdown`), on the same
    /// `queue_tick_secs` cadence and the same cancellation token -- and,
    /// like the resident loop, **not** drained on shutdown: work in flight
    /// is abandoned, and the visibility timeout returns it to `Pending` on
    /// the next start.
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
            // Shutdown abandons work in flight rather than draining it --
            // checked between every item, not only between
            // ticks, and raced into the delivery itself below, so
            // cancelling mid-tick does not wait out the rest of a claim
            // batch (up to `QUEUE_WORKER_CLAIM_LIMIT` items, each up to
            // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT`) against exactly the
            // unreachable substrates this is meant never to wait on. An
            // item not yet started this tick is left claimed; its own
            // visibility timeout returns it to `Pending` on the next
            // start, same as a crashed worker.
            if self.cancellation_token.is_cancelled() {
                return;
            }
            // An item claimed `max_attempts` times without ever reaching
            // `fail`/`complete` (a worker panic or crash on every delivery)
            // would otherwise be handed
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
    /// without `fail` ever being called for it -- dead-lettered through
    /// the same terminal path and alerting
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
    /// attempts the write again, and applies the outcome mapping:
    /// `applied`/`no-op`/`stale` complete and clear any `BindingConflict`
    /// the original transport failure raised; `conflict` completes **and**
    /// raises that same alert, exactly as the synchronous path does; a
    /// transport error retries; a callee error (the substrate reached and
    /// refused -- e.g. its target no longer exists) is terminal.
    ///
    /// Takes the same `instance_lock` the resident loop's own pass holds
    /// for the whole delivery -- without it, a queued write and a
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
            // is simply moot now, the same as `applied`/`no-op`/`stale`.
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
                // by `queue_tick_secs` instead of the configured curve.
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
                // write's own target is gone" case is terminal here --
                // `deploy::is_callee_error` alone would also dead-letter a
                // transient, reached-and-answered error (a locked database,
                // a service still starting) that the wire protocol cannot
                // currently distinguish from "gone" by error code, only by
                // message text. Treating every callee
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
    /// alert with the current dead-letter count for this key -- the DLQ's
    /// whole stated purpose fails unless something surfaces it.
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

        // The DLQ cap just pruned these other keys' oldest dead letters --
        // if that was their *last* one, their own standing alert must
        // clear too, or a prune leaves a permanent red mark nothing can
        // ever clear. A pruned key's group is always this same instance
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
    /// its dead letters remain -- `replay`'s own clearing path.
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
    /// the only place that holds it.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.cancellation_token.cancel();
        Ok(())
    }

    /// One tick of the resident loop: every non-retired, non-paused
    /// instance, in `all_active`'s order, sequentially -- a slow instance
    /// delays later ones in this same pass, accepted for A5c since the
    /// per-instance lock (not a global one) is what keeps that a latency
    /// property rather than a correctness one.
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

    /// One instance's share of a loop pass: a health sweep (shared by the
    /// alert pass and, unless superseded, the reconcile below), then --
    /// for a non-superseded instance -- a **filtered** redeploy of only
    /// the services `Reconciler::compute_diff` says changed since the last
    /// fully-landed plan, plus any service the current sweep finds with no
    /// completed placement at all (the `missing_placement` case, which a
    /// content-unchanged diff cannot see on its own). One client set for
    /// the whole pass, closed once at the end.
    async fn reconcile_instance_pass(&self, app_instance_id: &str) {
        // Each of these four reads used to fail silently -- no log, no
        // alert -- which drops the instance out
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
        // proves that call is gone: a second apply for this instance
        // cannot be in flight while we hold the lock, so a record still
        // reading `Applying` here was abandoned by a process that exited
        // between appending it and updating it. `Degraded` is the correct
        // resting state for "we do not know whether this landed" --
        // `handle_status` would otherwise report
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
        // These used to be discarded entirely. An unreachable substrate
        // is already visible another way (the
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
        // A dependent member whose diff against the last active plan
        // changed *only* `resolved_dependencies` is a
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

        // Landed services the sweep just found `InstanceNotRunning` are
        // restart
        // candidates -- distinct from `needs_work` above, which never-
        // landed or content-changed services feed into instead. A
        // healthy service's own remediation bookkeeping resets here too,
        // so the next fault starts counting from zero.
        let restart_candidates = Self::restart_candidates(&report);
        for svc in report.services.iter().filter(|s| s.signal == Signal::Healthy) {
            let _ = self.store.clear_remediation(app_instance_id, &svc.member_ref().to_string());
        }

        // The fourth work-list. Its input is this pass's own health poll
        // -- `ServiceHealth` already carries the certificate's
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

    /// The write half of a loop pass: mints, certifies, and applies only
    /// `needs_work`'s services, then attempts one bounded restart per
    /// `restart_candidates` entry. Extracted from `reconcile_instance_pass`
    /// so the re-read this opens with is directly testable against a
    /// `pause`/`retire` that lands between the health sweep and here --
    /// neither takes the per-instance lock a pass otherwise holds for its
    /// whole duration, so this is the one window that flag can still land
    /// in, and this fresh read is what closes it (a pause takes effect at
    /// the next write phase, not mid-write; this is that write phase's own
    /// boundary). Also picks up a generation `adopt` may have bumped since
    /// the pass's own early read.
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
            // block the service that *could* land. Only
            // services whose alias this pass actually connected to are
            // included; an unreachable one stays in `needs_work` (nothing
            // landed for it) and is picked up again next pass.
            filtered_plan.services.retain(|s| {
                needs_work.contains(&s.member_ref().to_string())
                    && s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
            });
            if !filtered_plan.services.is_empty() {
                // What gets *applied* this pass is deliberately narrowed
                // to `filtered_plan`, but what gets
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

        // Every member whose only change is which DIDs a dependency
        // resolves to gets a binding push instead of the
        // redeploy above -- an unreachable member this pass simply retries
        // next pass, since `resolved_dependencies` still disagrees with
        // what was last pushed.
        let mut any_push_failed = false;
        for (svc, substrate_did) in push_candidates {
            // A dependent this pass could not even connect to used to be
            // dropped here with no alert and no `opened` entry, so
            // `BindingConflict` was never set and `Degraded` never derived
            // from it -- indistinguishable from "nothing to push". Raised
            // through the same alert `write_bindings_at_epoch` itself
            // failing would raise, so the operator sees the same row
            // either way.
            //
            // Raising the alert and moving on used to be the whole story
            // here, which left a substrate this pass could
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
            // count the same as an error here, or a redeploy landing in
            // the same pass would journal this member's new baseline as
            // converged while the queue still holds stale content for it.
            // Distinct from `Landed` with zero outcomes (every dependency
            // was just removed from this member's manifest), which is a
            // real, converged success, not deferred -- an earlier version
            // of this match used an empty `Vec` as the deferred sentinel,
            // which that case collided with.
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
            // `kek_is_loaded()` first: that check reads the `KeyStore`,
            // not whether the storage
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
    /// deploys. Recording only the touched subset as `Active` made
    /// `Reconciler::compute_diff` -- which reads
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
    /// else about this member itself. `logical_ref` is
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
    /// this classification for one path and not the other is exactly the
    /// gap an earlier review round found.
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
                // A skipped tick, not a late one.
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
                    // reachable, which is a documented cost, not a
                    // delivery to retry.
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
                            // The sentinel, never `substrate_did` -- see
                            // `SCHEDULE_SUBSTRATE_DID`'s own doc.
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
    /// service: refuses if this service's remediation is already terminal,
    /// or if it is still inside `restart_backoff_secs` of the last
    /// attempt; otherwise calls `SubstrateActor::restart`, records the
    /// attempt regardless of the call's own outcome (an attempt is an
    /// attempt -- the next sweep is what determines whether it worked),
    /// and raises `RemediationExhausted`, naming `force-reconcile` as the
    /// escape hatch, the moment `max_restart_attempts` is reached.
    /// Takes `Arc<dyn SubstrateActor>` so this is directly testable
    /// against a fake actor with no live substrate.
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

    /// Publishes every alert this pass newly opened:
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

    /// Every alias `handle_status` must connect to this pass, deduplicated:
    /// the union of every alias the plan
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

    /// Refuses a submission whose plan would move an already-landed
    /// service to a different substrate than the journal shows it running
    /// on. An early version of `submit` shipped with no such refusal at
    /// all -- `roymctl`'s own `check_no_placement_change` is private to
    /// that binary and reads a local identity file the supervisor cannot
    /// see, so this is the supervisor's own check, not a reuse. Without
    /// it, a re-submit that changes an alias silently deploys a second
    /// live copy of the same member: the two-publisher state another
    /// refusal exists to prevent, reachable here because nothing on this
    /// path called it.
    ///
    /// Reads only what this supervisor's own journal has recorded landed
    /// -- never `roymctl`'s `--dir` -- so it is safe to call from both
    /// `submit` and `force-reconcile`.
    ///
    /// A refusal here raises `AlertKind::PlacementChangeRefused` -- the
    /// variant existed, tested only in its own `Display`/`FromStr` round
    /// trip, with nothing in either caller ever raising it. Raised (and
    /// published, same as every other alert this file opens) before the
    /// refusal is returned, so a refused submission is visible on `alerts`
    /// even though it is otherwise indistinguishable from a plain RPC
    /// error to whatever received it.
    /// `SynAppManifest::validate()` enforces `MAX_REPLICAS` at
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
    /// the inventory or carrying no credential.
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
    /// undetectable.
    ///
    /// Returns `None`, not `Some(0)`, when not one placed substrate could
    /// be reached and queried -- every failure (no inventory entry,
    /// connect failure, RPC error) previously folded into the same "0" a
    /// substrate with a genuinely empty management row also produces, so
    /// a supervisor that had lost its own `orchestrator/status` grant
    /// reported "not superseded" indefinitely instead of "cannot tell".
    ///
    /// Takes already-connected clients, keyed by alias, rather than
    /// connecting itself -- `handle_status` used to connect to every
    /// substrate twice per call (once for the health sweep, once here),
    /// and this is now the same client set the sweep used.
    ///
    /// Takes `Arc<dyn SubstrateActor>` rather than a concrete
    /// `SyneroymClient` -- callers upcast their real,
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
    /// undurable constructor: every actor this builds is used for
    /// exactly one read, `held_generation`, and never for `write_bindings`
    /// -- durability would add a queue key with nothing meaningful to bind
    /// it to and no call that could ever use it.
    fn actors_from_clients(
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
        clients.iter().map(|(alias, c)| (alias.clone(), deploy::build_actor(c.clone()))).collect()
    }

    /// The durable constructor every other app-supervisor call site with a
    /// real client builds through: `write_bindings` on the returned actor
    /// attempts synchronously first and enqueues onto this supervisor's
    /// own outbox only on a transport failure. Every other action stays
    /// exactly as undurable as `build_actor` would make it -- that
    /// declaration lives inside `DurableActor` itself, not in which call
    /// sites choose this over `build_actor`.
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
    /// instance is currently superseded (ADR-0021 §4).
    /// Shared by `handle_status` and the loop's own pass so the two cannot
    /// read "superseded" two different ways.
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
    /// clean up on drop. Only closes a client this
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
    /// same plan for storing as desired state: one vault open and one
    /// `reveal_secret` per service for a value this call had already
    /// computed.
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
        // closed -- not just on the success path.
        let result =
            self.apply_with_membership_pushes(&plan, &masters, &clients, generation, minted).await;
        Self::shutdown_clients(clients.into_values()).await;
        result.map(|minted| (minted, plan))
    }

    /// Mints, certifies, and applies `plan`, except for whatever member the
    /// same classifier `reconcile_instance_pass` uses
    /// (`classify_update_actions`) would route to a binding push or
    /// exclude outright -- those get `push_bindings` after the redeploy of
    /// the rest, rather than a full `deploy_with_context` reinstall.
    /// Shared by `deploy_submission` (`force-reconcile`) and
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
                // Visible on `alerts`, the same as any other push failure,
                // not just returned to this call's own caller -- the
                // resident loop's next pass does not re-raise a fresh
                // alert for the same cause until this one clears. Also
                // queued, the same reason the resident loop's own
                // analogous branch is (`enqueue_unreachable_push`'s doc
                // comment) -- a fallback-
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
            // count the same as an error here too, so
            // `submit`/`force-reconcile` reports it and the downgrade
            // below fires, same as the resident loop's own call site.
            // `Landed` with zero outcomes (every dependency just removed)
            // is a real success, not deferred.
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
    /// deliberately wider than it for the loop's filtered pass: see
    /// `record_plan_for_pass`'s own doc for why the two must not be
    /// conflated.
    async fn apply_with_clients(
        &self,
        plan: &DeploymentPlan,
        record_plan: &DeploymentPlan,
        masters: &BTreeMap<ServiceId, Identity>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        minted: Vec<MintedMaster>,
    ) -> Result<Vec<MintedMaster>, String> {
        // The one place every certificate-minting caller passes through --
        // the resident loop, `submit`, and
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
        // Deliberately the plain, undurable constructor: `deploy::
        // apply_plan` only ever calls `actor.apply_plan(..)` on these
        // targets, never `write_bindings`, and `apply_plan` is never queued
        // regardless -- there is no per-service logical ref to
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

        // The counter always advances before a write -- a deploy is an
        // authoritative write like any other, so
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
                // Always true on the supervisor's apply path: the
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
    /// redeploy: reuses `map_deployment_plan_to_wit`'s own
    /// binding-construction logic (called the same way `apply_plan` calls
    /// it internally, over `&[svc]`) rather than duplicating it, so the
    /// two paths cannot drift apart on what a binding looks like on the
    /// wire. Its production caller is the membership-change classifier in
    /// `reconcile_instance_pass`/`apply_write_phase`.
    ///
    /// `Stale(held)` is retried exactly once, at `held + 1`: no re-read,
    /// since `Stale` already carries the number a second round trip would
    /// only relearn. `Conflict` is not retried -- a second writer exists,
    /// and retrying would only race it again. Either failure raises
    /// `BindingConflict`, folded into `opened` so the caller can publish
    /// it the same way every other alert this pass raised gets published.
    /// A push that lands cleanly clears it instead -- the clear site this
    /// alert kind never had, without which `Degraded` derived from it
    /// would be permanent.
    ///
    /// `substrate_did` is the member's real, already-landed substrate DID
    /// -- not `svc.substrate`, an operator-chosen
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

        // This call advances the binding epoch unconditionally, before
        // every attempt. `DurableActor::write_
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
        // never happened and was never durably queued either. Failing
        // *open* instead is safe specifically because the queue and every
        // other supervisor table share one connection -- a genuinely
        // broken connection surfaces a
        // proper `Err` on the very next line's `advance_binding_epoch`
        // instead of a silent no-op.
        if self.store.queue.has_pending(&queue_key.to_string()).unwrap_or(false) {
            return Ok(PushOutcome::Deferred);
        }

        let epoch = self
            .store
            .advance_binding_epoch(&app_instance_id, &l_ref)
            .map_err(|e| e.to_string())?;
        // A `write_bindings` call that fails outright (the dependent
        // unreachable) used to propagate with `?`, before the
        // alert-raising code below was ever reached
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

    /// The alert half of an unreachable dependent: a push that fails to
    /// reach the dependent at all (not a clean `Stale`/`Conflict` outcome)
    /// still
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
    /// a transport failure, but that only fires *inside* an
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
        // The resident loop's own retry (falling `compute_diff` back to
        // the previous baseline on Degraded) reclassifies this member as
        // a push
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
        // Held for the whole call, so a loop pass or another operator
        // write for this same instance cannot interleave with it -- a
        // read-then-write race otherwise.
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
        // never associates with the deployed services.
        if plan.app_instance_id.as_str() != s.app_instance_id {
            return Err(RpcError::InvalidParams(format!(
                "submission names app instance '{}' but its plan-json is compiled for '{}'",
                s.app_instance_id, plan.app_instance_id
            )));
        }

        // Checked before any deploy work runs, not only after:
        // `store.submit`'s own guards, below, live past the whole
        // mint/certify/apply pipeline. For `retired` that used to mean
        // only a late rejection. For `generation` it is worse:
        // `deploy_submission` already presents
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

        // Checked in the same pre-flight as `retired`/`generation` above,
        // before any deploy work runs -- a changed placement must be
        // refused, not silently applied.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // The manifest-time replica cap, re-checked at the interface that
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
        // compiler's fabricated ones.
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

        // Persisted here, before the deploy attempt below -- so a
        // substrate that is down or slow at this
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
    /// failure -- `?` inside either loop here used to return straight out
    /// of `handle_adopt` itself, leaking
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

        // Resolved or minted before any substrate connection is opened --
        // same ordering `submit`'s own mint uses
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
        // refusal on a retired instance points to. Idempotent when the
        // instance was never retired.
        //
        // The generation, the un-retired flag, and the resolved app
        // master DID land in one combined store write rather than three
        // separate ones -- a crash between them used to be able to leave a
        // claimed generation with no recorded app master, breaking the
        // invariant that the row always agrees with the vault.
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
        // A fresh generation is a fresh start, so a terminal
        // `InstanceNotRunning` service -- one nothing will ever
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
    /// instance is placed on. Connects what it can
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
            // `queue_worker_tick`'s per-item check, `run()`'s outer
            // `select!` only races cancellation against
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
    /// what they can.
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
        // redeploy every service indefinitely.
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
        // `force-reconcile` never calls `store.submit`, so nothing else on
        // this path checks placement either -- the identical reasoning as
        // the `retired` check above.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // The same replica-cap re-check `submit` runs -- a desired-state
        // row written before this check existed must not get a permanent
        // pass.
        Self::refuse_replicas_above_cap(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(&plan).map_err(RpcError::InternalError)?;
        // A directed reconcile is a fresh start, regardless of what this
        // call's own outcome turns out to be --
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

    /// The read half of binding convergence: per declared dependency of
    /// every dependent in the plan, what this supervisor last wrote
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
    /// landed (a real `substrate_did`) -- restart candidates. Deliberately
    /// excludes `ProbeFailing` (an author-declared assertion, not a
    /// substrate-verified fact -- alert only) and `SubstrateUnreachable`
    /// (restarting cannot fix a substrate that did not answer). Its own
    /// function so this filter is directly testable against a synthetic
    /// `HealthReport`, with no live substrate.
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

    /// Runs a fresh health sweep inside the RPC rather than reading rows
    /// nothing writes -- this read surface is not idle, it just isn't on a
    /// resident timer.
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

        // One client set for the whole call, shared by the health sweep
        // and the generation read below -- `handle_status`
        // used to connect to every substrate twice. The connected set is
        // the union of every alias the plan declares (needed for the
        // generation read, which must reach a substrate even before
        // anything has landed there) and every alias a landed placement
        // names (needed for the health sweep).
        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(&plan).unwrap_or_default().into_iter().collect();
        let connect_aliases = Self::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, &inventory).await;
        // These used to be discarded entirely. An unreachable substrate
        // is already visible another way (the
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

        // A planned service the journal has never recorded landed is a
        // deploy failure the sweep cannot see (it has no
        // `service_id`/`substrate_did` to probe at all, reported as
        // `NotDeployed`, deliberately not a fault).
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
        // clear it" cleanup -- which would otherwise clear this alert
        // every single call, for the identical reason in reverse.
        // `extra_live_pairs` is exactly the exemption that cleanup needs.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        // Same call, same constant, as the resident loop's own.
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

        // Publication happens here, in `record_report`'s caller, over the
        // newly-opened list above -- every store write
        // that could add to it has already committed, so a publish
        // failure below can never lose an alert by construction. Never
        // propagated with `?`: an unreachable/slow MQTT broker must not
        // fail the whole `status` call.
        self.publish_opened_alerts(&app_instance_id, &opened).await;

        // ADR-0021 §4: a substrate reporting a higher generation than this
        // supervisor holds means a second supervisor
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

        // Closed once, at the end, now that both the health sweep and the
        // generation read above are done with them.
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

        // A reconcile in flight is now observable -- `apply_with_clients`
        // writes `Applying` and this is
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

        // A binding push that has been attempted and did not land leaves
        // the instance `Degraded` -- reachable now that `push_bindings`
        // has a production caller. Read off the *active* alert set rather
        // than "any
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
        // A service the plan names but the journal has never recorded
        // landed is a deploy failure the sweep alone cannot see (a
        // `NotDeployed` signal is deliberately not a fault) -- the
        // supervisor adds the plan knowledge the poll does not have.
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
            // An earlier version ran no reconcile loop, so this used to be
            // permanently `None`. It is not `Some(now)` either -- that
            // reported every instance as having just reconciled, even one
            // that never has. The loop now stamps `last_reconciled` at
            // the end of every pass it actually runs for this instance;
            // `status`'s own on-demand sweep, right here, deliberately
            // does not count as one.
            last_reconciled_at: self.last_reconciled.get(&app_instance_id).map(|v| *v as u64),
            services,
            // Read off the store's own written epoch and this pass's
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
            // Read from the stored row only, never the vault -- a locked
            // vault is the ordinary state of a
            // freshly-booted supervisor, and this field must stay readable
            // through it. Empty means "never adopted under A7", mapped to
            // `None` here so a caller does not have to know `""` is a
            // sentinel.
            app_master_did: (!state.app_master_did.is_empty()).then_some(state.app_master_did),
            // ADR-0022 §2: derived from the last successful refresh this
            // supervisor stamped, not read back from the
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
    /// or claimed, not yet dead-lettered: `roymctl supervisor outbox`'s
    /// own listing, and what an end-to-end test needs to assert the item
    /// is actually queued rather than only inferring it from alerts or
    /// `is_converged`.
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

    /// Every dead letter belonging to this instance, oldest first --
    /// `roymctl supervisor dead-letters`'s own listing.
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

    /// Re-enqueues a dead letter -- it never executes inline.
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
        // parameter, not a server problem, so both answer `InvalidParams`.
        // Neither names which other instance
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
        // The supplied name is canonicalised (`resolve` accepts a logical
        // service name *or* its `short_hash`) **inside** the
        // two-attempt loop, since each attempt re-reads `state.plan_json`
        // and a `submit` landing between attempts can change the declared
        // names. `resolved_name` then replaces `service_name` at every
        // later use in this function -- the epoch key, the cache key, and
        // `TopologyDocument.service_name` -- so the document always names
        // the real service name, never the hash a caller sent. That
        // property is what lets the gateway's own check
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
mod tests;

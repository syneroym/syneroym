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

mod bindings;
mod clients;
mod queue_worker;
mod renewal;
mod resident_loop;
mod resolve;
mod schedules;
mod status;
mod verbs;

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
    pub(super) fn instance_lock(&self, app_instance_id: &str) -> Arc<AsyncMutex<()>> {
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
    pub(super) fn build_pass_interval(poll_interval_secs: u64) -> tokio::time::Interval {
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

    /// Cancels the loop's token -- the spawn site (`RuntimeServices`) is
    /// the one that awaits the `JoinHandle` this unblocks, since that is
    /// the only place that holds it.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.cancellation_token.cancel();
        Ok(())
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
    pub(super) async fn publish_opened_alerts(
        &self,
        app_instance_id: &str,
        opened: &[(AlertKind, String)],
    ) {
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

    pub(super) fn require_admin(&self, caller: &CallerContext) -> RpcResult<()> {
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

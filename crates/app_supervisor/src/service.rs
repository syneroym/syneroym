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
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use serde_json::Value;
use syneroym_app_orchestration::{
    AlertKind, DeploymentState, ReconcileAction, Reconciler,
    models::{
        AppInstanceId, DeploymentPlan, LogicalServiceRef, PlannedService, ServiceId, SubstrateAlias,
    },
};
use syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::{MqttBroker, namespace_topic_for_publish};
use syneroym_rpc::{
    Ability, CallerContext, NativeInvocation, NativeResponse, NativeService,
    PERMISSION_DENIED_CODE, ResourceUri, RpcError, RpcResult,
};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, SyneroymClient,
    deploy::{self, ApplyRequest, DeployTarget, SubstrateActor},
    health::{self, ExpectedService, HealthTarget, Signal, StatusQuery},
    mapper::map_deployment_plan_to_wit,
};
use syneroym_wit_interfaces::supervisor::exports::syneroym::supervisor::supervisor::{
    Alert, BindingConvergence, InstanceStatus, ManagedService, ManagedState,
    MintedMaster as WitMintedMaster, Submission, SubmitResult,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::{
    MasterVault, MintedMaster,
    inventory::{SupervisorInventory, SupervisorInventoryEntry},
    keys,
    store::{RemediationState, SupervisorStore},
};

const SUPERVISOR_INTERFACE: &str = "supervisor";
/// Ceiling for connecting to one managed substrate -- generous relative to
/// `PREFLIGHT_TIMEOUT`'s 5s in `roymctl`, since a supervisor call may fan
/// out to several substrates concurrently rather than being a single
/// operator-watched command.
const MANAGED_SUBSTRATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Matches `sdk::deploy::DEFAULT_INSTANCE_CERT_EXPIRES_HOURS` -- the
/// attended posture's default, since A5b mints and certifies but does not
/// yet renew (A5d).
const INSTANCE_CERT_EXPIRES_HOURS: u64 = deploy::DEFAULT_INSTANCE_CERT_EXPIRES_HOURS;
/// The `substrate_did` D-A5c-10's "planned but never landed"
/// `InstanceNotRunning` alert is keyed under -- deliberately not the
/// empty string `record_report`'s own per-service loop uses for a
/// `Signal::NotDeployed` service, which would otherwise clear this exact
/// alert on every pass right before it gets re-raised (see the call
/// site's own comment).
const NEVER_LANDED_SUBSTRATE_DID: &str = "supervisor:never-landed";

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
    /// Cancelled by `shutdown` -- the resident loop (`run`, spawned by
    /// `RuntimeServices::run_until_shutdown`, not pinned in its own
    /// `select!`) watches this to stop between passes rather than being
    /// dropped mid-pass (M05A A5c §19.8, D-A5c-8). The `JoinHandle`
    /// itself is held by `RuntimeServices`, which awaits it after calling
    /// `shutdown` -- cancelling alone does not wait for the pass in
    /// flight to actually finish closing its clients.
    cancellation_token: CancellationToken,
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
        messaging_broker: Arc<MqttBroker>,
        alert_topic: String,
        poll_interval_secs: u64,
        max_restart_attempts: u32,
        restart_backoff_secs: u64,
    ) -> Self {
        Self {
            node_did,
            store,
            vault,
            client_identity_bytes: client_identity.to_bytes(),
            messaging_broker,
            alert_topic,
            poll_interval_secs,
            max_restart_attempts,
            restart_backoff_secs,
            instance_locks: DashMap::new(),
            last_reconciled: DashMap::new(),
            cancellation_token: CancellationToken::new(),
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
            match deploy::current_placement(&landed, &svc.logical_ref.to_string()) {
                None => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                    });
                    missing_placement.insert(svc.logical_ref.to_string());
                }
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
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
        let mut opened = match health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
            now,
            &extra_live_pairs,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(app_instance_id, error = %e, "failed to record this pass's health report");
                Vec::new()
            }
        };
        for svc in &plan.services {
            let l_ref = svc.logical_ref.to_string();
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
        // D-A5c-2/D-A5c-3/D-A5c-21: the work list is `compute_diff`'s
        // Add/Update actions (a plan-level change) plus `missing_
        // placement` (a service the current sweep finds with no landed
        // placement at all, which a content-unchanged diff against an
        // older Active snapshot cannot see on its own -- D-A5c-10's own
        // gap). `Remove` actions are deliberately excluded from the work
        // list -- a plan-level removal is never undeployed here, and is
        // instead raised as `OrphanedService`, folded into this same
        // `opened`/publish pass since it is a local read, not a write to
        // any substrate.
        let diff = Reconciler::new(&self.store.journal).compute_diff(&plan);
        let mut needs_work: BTreeSet<String> = missing_placement.clone();
        if let Ok(diff) = &diff {
            for action in &diff.actions {
                match action {
                    ReconcileAction::Add(svc) => {
                        needs_work.insert(svc.logical_ref.to_string());
                    }
                    ReconcileAction::Update { new, .. } => {
                        needs_work.insert(new.logical_ref.to_string());
                    }
                    ReconcileAction::Remove(l_ref) => {
                        let l_ref_str = l_ref.to_string();
                        if let Some(row) = deploy::current_placement(&landed, &l_ref_str)
                            && let Ok(true) = self.store.alerts.raise(
                                &instance_id,
                                Some(&l_ref_str),
                                row.substrate_alias.as_deref(),
                                &row.substrate_did,
                                AlertKind::OrphanedService,
                                "dropped from the plan but still running on its substrate; not \
                                 undeployed -- remove it by hand (`svc remove`) if that is \
                                 intended",
                            )
                        {
                            opened.push((AlertKind::OrphanedService, l_ref_str));
                        }
                    }
                }
            }
        }
        // A service back in the current plan cannot be orphaned this
        // pass, regardless of what an older diff once said.
        for svc in &plan.services {
            let l_ref = svc.logical_ref.to_string();
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
            let _ = self.store.clear_remediation(app_instance_id, &svc.logical_ref.to_string());
        }

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

        if !needs_work.is_empty() || !restart_candidates.is_empty() {
            self.apply_write_phase(
                &instance_id,
                app_instance_id,
                &plan,
                &needs_work,
                &restart_candidates,
                &did_to_alias,
                &clients,
                now,
            )
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
    #[allow(clippy::too_many_arguments)]
    async fn apply_write_phase(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        needs_work: &BTreeSet<String>,
        restart_candidates: &[(String, String, String)],
        did_to_alias: &BTreeMap<String, String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        now: u64,
    ) {
        let Ok(Some(fresh_state)) = self.store.get(app_instance_id) else { return };
        if fresh_state.paused || fresh_state.retired {
            return;
        }

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
                needs_work.contains(&s.logical_ref.to_string())
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
                        if let Err(e) = self
                            .apply_with_clients(
                                &filtered_plan,
                                &record_plan,
                                &masters,
                                clients,
                                fresh_state.generation,
                                minted,
                            )
                            .await
                        {
                            tracing::warn!(
                                app_instance_id,
                                error = %e,
                                "this pass's redeploy did not fully land"
                            );
                        }
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
        for (logical_ref, service_id, substrate_did) in restart_candidates {
            let Some(alias) = did_to_alias.get(substrate_did) else { continue };
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else { continue };
            let actor = client.clone() as Arc<dyn SubstrateActor>;
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
        self.publish_opened_alerts(app_instance_id, &opened).await;
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
            !needs_work.contains(&s.logical_ref.to_string())
                || s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
        });
        record_plan
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
        );
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
            let l_ref = svc.logical_ref.to_string();
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
    /// `max_held_generation_from_clients` (and, in later phases, the
    /// remediation/push call sites) takes -- one place doing the upcast
    /// rather than each call site repeating it.
    fn actors_from_clients(
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
        clients
            .iter()
            .map(|(alias, c)| (alias.clone(), c.clone() as Arc<dyn SubstrateActor>))
            .collect()
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

        // However `apply_with_clients` below returns, every client this
        // call opened must be closed -- not just on the success path
        // (S6).
        let result =
            self.apply_with_clients(&plan, &plan, &masters, &clients, generation, minted).await;
        Self::shutdown_clients(clients.into_values()).await;
        result.map(|minted| (minted, plan))
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
        let (instance_certs, registry_certs) = deploy::certify_placed_members(
            plan,
            masters,
            clients,
            None,
            INSTANCE_CERT_EXPIRES_HOURS,
        )
        .await
        .map_err(|e| e.to_string())?;

        let deployment_id = self
            .store
            .journal
            .append(record_plan, DeploymentState::Applying)
            .map_err(|e| e.to_string())?;
        let targets: BTreeMap<SubstrateAlias, DeployTarget> = clients
            .iter()
            .map(|(alias, c)| {
                (
                    alias.clone(),
                    DeployTarget {
                        alias: Some(alias.clone()),
                        substrate_did: c.service_id().to_string(),
                        actor: c.clone() as Arc<dyn SubstrateActor>,
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
        let mut binding_epochs: BTreeMap<LogicalServiceRef, u64> = BTreeMap::new();
        for svc in &plan.services {
            if svc.resolved_dependencies.is_empty() {
                continue;
            }
            let epoch = self
                .store
                .advance_binding_epoch(
                    &plan.app_instance_id.to_string(),
                    &svc.logical_ref.to_string(),
                )
                .map_err(|e| e.to_string())?;
            binding_epochs.insert(svc.logical_ref.clone(), epoch);
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
                report.failures.iter().map(|f| format!("{}: {}", f.logical_ref, f.error)).collect();
            return Err(format!("deploy applied with failures: {}", failures.join("; ")));
        }

        Ok(minted)
    }

    /// One dependent service's bindings, at its next epoch, without a
    /// redeploy (M05A A5c phase 7, D-A5c-16): reuses `map_deployment_
    /// plan_to_wit`'s own binding-construction logic (called the same
    /// way `apply_plan` calls it internally, over `&[svc]`) rather than
    /// duplicating it, so the two paths cannot drift apart on what a
    /// binding looks like on the wire. A5c has no reachable membership
    /// change of its own (A5e's `replicas` is the real trigger), so
    /// nothing in this slice calls this automatically -- it exists,
    /// tested, for the loop to call once that trigger lands.
    ///
    /// `Stale(held)` is retried exactly once, at `held + 1` (D-A5c-19 /
    /// F4): no re-read, since `Stale` already carries the number a
    /// second round trip would only relearn. `Conflict` is not retried --
    /// a second writer exists, and retrying would only race it again.
    /// Either failure raises `BindingConflict`, folded into `opened` so
    /// the caller can publish it the same way every other alert this
    /// pass raised gets published.
    ///
    /// `#[allow(dead_code)]`: not reachable from any production call site
    /// in this slice, deliberately -- see the paragraph above. Exercised
    /// directly by tests 42-47 (§23).
    #[allow(dead_code)]
    async fn push_bindings(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let app_instance_id = plan.app_instance_id.to_string();
        let l_ref = svc.logical_ref.to_string();

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
                self.raise_binding_push_failure(instance_id, svc, &l_ref, &e, opened);
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
                    self.raise_binding_push_failure(instance_id, svc, &l_ref, &e, opened);
                    return Err(e);
                }
            }
        } else {
            outcomes
        };

        let failed = outcomes
            .iter()
            .any(|o| matches!(o, BindingWriteOutcome::Stale(_) | BindingWriteOutcome::Conflict(_)));
        if failed
            && let Ok(true) = self.store.alerts.raise(
                instance_id,
                Some(&l_ref),
                None,
                &svc.substrate.as_ref().map_or_else(String::new, ToString::to_string),
                AlertKind::BindingConflict,
                &format!(
                    "a binding push for '{l_ref}' did not land cleanly after one retry: \
                     {outcomes:?}"
                ),
            )
        {
            opened.push((AlertKind::BindingConflict, l_ref));
        }
        Ok(outcomes)
    }

    /// The alert half of matrix row 11: a push that fails to reach the
    /// dependent at all (not a clean `Stale`/`Conflict` outcome) still
    /// needs to be visible on `alerts`, the same `AlertKind` a bad
    /// outcome raises -- an operator reading `alerts` should not have to
    /// know which of the two shapes a failed push took.
    fn raise_binding_push_failure(
        &self,
        instance_id: &AppInstanceId,
        svc: &PlannedService,
        l_ref: &str,
        error: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(l_ref),
            None,
            &svc.substrate.as_ref().map_or_else(String::new, ToString::to_string),
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
    async fn write_bindings_at_epoch(
        &self,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        epoch: u64,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let binding_epochs = BTreeMap::from([(svc.logical_ref.clone(), epoch)]);
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
        actor
            .write_bindings(BindingWrite {
                service_id: svc.service_id.to_string(),
                app_instance_id: plan.app_instance_id.to_string(),
                bindings,
                generation,
            })
            .await
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

        // Best-effort immediate apply: still surfaced to the caller as an
        // error if it does not fully land (an operator's `submit` should
        // know when nothing landed), but the desired state above is
        // already durable regardless of this outcome.
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;
        let apply_result =
            self.apply_with_clients(&plan, &plan, &masters, &clients, s.generation, minted).await;
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
        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;

        let result = Self::claim_next_generation(&app_instance_id, &clients).await;
        Self::shutdown_clients(clients.into_values()).await;
        let next_generation = result?;

        self.store
            .set_generation(&app_instance_id, next_generation)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // `adopt` is the way back in from `retired` -- the message every
        // refusal on a retired instance points to (N3, Slice A5b review
        // round 2). Idempotent when the instance was never retired.
        self.store
            .un_retire(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // D-A5c-20 (§19.20/F5): a fresh generation is a fresh start, so a
        // terminal `InstanceNotRunning` service -- one nothing will ever
        // restart again on its own -- becomes escapable here.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);

        Ok(NativeResponse { payload: serde_json::to_value(next_generation).unwrap_or(Value::Null) })
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
            match self.connected_client(entry).await {
                Ok(client) => {
                    clients.insert(SubstrateAlias::new(alias.clone()), Arc::new(client));
                }
                Err(e) => failed.push((alias.clone(), e.to_string())),
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
        Ok(NativeResponse { payload: serde_json::json!({"status": "paused"}) })
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
        // D-A5c-20 (§19.20/F5): a directed reconcile is a fresh start,
        // regardless of what this call's own outcome turns out to be --
        // a terminal `InstanceNotRunning` service is otherwise never
        // restarted again, so the loop's own healthy-sweep clearing path
        // never fires for it.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);
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
            let dependent_ref = svc.logical_ref.to_string();
            let written_epoch =
                self.store.binding_epoch(app_instance_id, &dependent_ref).unwrap_or(0);
            let observed: BTreeMap<&str, u64> = report
                .services
                .iter()
                .find(|s| s.logical_ref.to_string() == dependent_ref)
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
            .map(|s| (s.logical_ref.to_string(), s.service_id.clone(), s.substrate_did.clone()))
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
            match deploy::current_placement(&landed, &svc.logical_ref.to_string()) {
                None => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                    });
                    missing_placement.insert(svc.logical_ref.to_string());
                }
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
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
        let mut opened = health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
            now,
            &extra_live_pairs,
        )
        .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Folded into `opened` (not published separately) so the publish
        // call below sees every alert this pass newly raised, not only
        // the ones `record_report` itself knows about.
        for svc in &plan.services {
            let l_ref = svc.logical_ref.to_string();
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
                logical_ref: s.logical_ref.to_string(),
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
                    .remediation_state(&app_instance_id, &s.logical_ref.to_string())
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
        } else if report.faults().is_empty() && missing_placement.is_empty() {
            ManagedState::Active
        } else {
            ManagedState::Degraded
        };

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
            "status" => self.handle_status(&invocation.caller, invocation.params).await,
            "alerts" => self.handle_alerts(&invocation.caller, invocation.params).await,
            method => Err(RpcError::MethodNotFound(method.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use syneroym_app_orchestration::{
        ActionState,
        models::{AppBlueprintId, LogicalServiceName, ServiceConfig, ServiceType, TopologyMode},
    };
    use syneroym_rpc::AuthLevel;

    use super::*;

    fn test_broker() -> Arc<MqttBroker> {
        Arc::new(MqttBroker::new(syneroym_mqtt_broker::MqttBrokerConfig::default()).unwrap())
    }

    fn service() -> SupervisorService {
        let dir = tempfile::tempdir().unwrap();
        let store = SupervisorStore::open_in_memory().unwrap();
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.path().join("db"), false).unwrap(),
        );
        let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
        let vault = MasterVault::new(
            storage_provider,
            key_store,
            "supervisor".to_string(),
            dir.path().join("backups"),
        );
        let identity = Identity::generate().unwrap();
        SupervisorService::new(
            "did:key:zSupervisorNode".to_string(),
            store,
            vault,
            &identity,
            test_broker(),
            "supervisor/alerts".to_string(),
            30,
            3,
            30,
        )
    }

    /// Encryption on, no KEK injected -- §0.31's whole point is that a
    /// disabled-encryption fixture proves nothing about the locked case.
    fn service_with_locked_vault() -> SupervisorService {
        let dir = tempfile::tempdir().unwrap();
        let store = SupervisorStore::open_in_memory().unwrap();
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.path().join("db"), true).unwrap(),
        );
        let key_store = Arc::new(syneroym_data_keystore::KeyStore::new());
        let vault = MasterVault::new(
            storage_provider,
            key_store,
            "supervisor".to_string(),
            dir.path().join("backups"),
        );
        let identity = Identity::generate().unwrap();
        SupervisorService::new(
            "did:key:zSupervisorNode".to_string(),
            store,
            vault,
            &identity,
            test_broker(),
            "supervisor/alerts".to_string(),
            30,
            3,
            30,
        )
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
                "inst-1/backend",
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
                "inst-1/backend",
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
                "inst-1/backend",
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
                "inst-1/backend",
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
                "inst-1/backend",
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
        let needs_work: BTreeSet<String> = ["inst-1/svc-b".to_string()].into_iter().collect();
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
        let needs_work: BTreeSet<String> = ["inst-1/svc-b".to_string()].into_iter().collect();

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

        s.apply_write_phase(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &needs_work,
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            0,
        )
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
                "inst-1/backend",
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

        s.apply_write_phase(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            &plan,
            &BTreeSet::new(),
            &restart_candidates,
            &did_to_alias,
            &clients,
            0,
        )
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
                "inst-1/frontend",
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
        assert_eq!(orphan.logical_ref.as_deref(), Some("inst-1/frontend"));
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

        let outcomes =
            s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await.unwrap();

        assert_eq!(outcomes, vec![BindingWriteOutcome::Applied]);
        assert_eq!(actor.calls.lock().unwrap().len(), 1);
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 1);
        assert!(opened.is_empty());
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

        s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await.unwrap();

        assert_eq!(actor.calls.lock().unwrap().len(), 1, "a conflict must not be retried");
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend".to_string())]);
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

        s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await.unwrap();

        let calls = actor.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "exactly one retry");
        assert_eq!(calls[1].bindings[0].epoch, 6, "the retry must land at held + 1, not held");
        drop(calls);
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend".to_string())]);
        assert_eq!(
            s.store.binding_epoch("inst-1", "inst-1/frontend").unwrap(),
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
        s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await.unwrap();

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
        assert_eq!(rows[0].dependent_logical_ref, "inst-1/frontend");
        assert_eq!(rows[0].dependency_name, "backend");
        assert_eq!(rows[0].written_epoch, 1);
        assert_eq!(rows[0].observed_epoch, Some(1));
        assert!(rows[0].converged);
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
        s.store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();

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
        assert_eq!(status.bindings[0].dependent_logical_ref, "inst-1/frontend");
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
        let _ = s.store.advance_binding_epoch("inst-1", "inst-1/frontend");

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

        let first = s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await;
        assert!(first.is_err());
        assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 1);
        assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend".to_string())]);
        let alerts = s.store.alerts.active(&instance_id).unwrap();
        assert!(alerts.iter().any(|a| a.kind == AlertKind::BindingConflict), "{alerts:?}");

        let second = s.push_bindings(&instance_id, &plan, &svc, &dyn_actor, 0, &mut opened).await;
        assert_eq!(second.unwrap(), vec![BindingWriteOutcome::Applied]);
        assert_eq!(
            s.store.binding_epoch("inst-1", "inst-1/frontend").unwrap(),
            2,
            "the retry must carry a fresh epoch, not reuse the one the failed attempt spent"
        );
    }
}

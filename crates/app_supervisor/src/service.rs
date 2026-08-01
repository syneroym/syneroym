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
    fmt, future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use syneroym_app_orchestration::{
    DeploymentState,
    models::{AppInstanceId, DeploymentPlan, SubstrateAlias},
};
use syneroym_identity::Identity;
use syneroym_rpc::{
    Ability, CallerContext, NativeInvocation, NativeResponse, NativeService,
    PERMISSION_DENIED_CODE, ResourceUri, RpcError, RpcResult,
};
use syneroym_sdk::{
    SyneroymClient,
    deploy::{self, ApplyRequest, DeployTarget, SubstrateActor},
    health::{self, ExpectedService, HealthTarget, Signal, StatusQuery},
};
use syneroym_wit_interfaces::supervisor::exports::syneroym::supervisor::supervisor::{
    Alert, BindingConvergence, InstanceStatus, ManagedService, ManagedState,
    MintedMaster as WitMintedMaster, Submission, SubmitResult,
};

use crate::{
    MasterVault, MintedMaster, inventory::SupervisorInventory, keys, store::SupervisorStore,
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
}

impl fmt::Debug for SupervisorService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorService")
            .field("node_did", &self.node_did)
            .finish_non_exhaustive()
    }
}

impl SupervisorService {
    pub fn new(
        node_did: String,
        store: SupervisorStore,
        vault: MasterVault,
        client_identity: &Identity,
    ) -> Self {
        Self { node_did, store, vault, client_identity_bytes: client_identity.to_bytes() }
    }

    pub fn store(&self) -> &SupervisorStore {
        &self.store
    }

    /// This slice's read surface is not idle -- `status` sweeps on demand
    /// (D-A5-21) -- but there is no resident loop yet; the loop is a later
    /// slice. Matches `EcosystemRegistry`/`ClientGateway`'s own
    /// `pending_component` shape when a role is absent.
    pub async fn run(&self) -> anyhow::Result<()> {
        future::pending().await
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
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
        entry: &crate::inventory::SupervisorInventoryEntry,
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

    /// Reads one substrate's held generation for an app instance, if it
    /// has ever recorded one. `app-instance-management-of` returns
    /// `option<app-instance-management>`, which serializes as `null` or
    /// the record's fields directly (ordinary serde `Option`).
    async fn read_held_generation(
        client: &SyneroymClient,
        app_instance_id: &str,
    ) -> anyhow::Result<Option<u64>> {
        let res = client
            .request(
                "orchestrator",
                "app-instance-management-of",
                serde_json::to_value((app_instance_id.to_string(),))?,
            )
            .await?;
        Ok(res.result.get("generation").and_then(Value::as_u64))
    }

    /// The highest generation any placed, reachable substrate reports
    /// holding for this instance -- best-effort, so one unreachable
    /// substrate cannot hide a real supersession another one reports.
    async fn max_held_generation(
        &self,
        app_instance_id: &str,
        aliases: &BTreeMap<String, String>,
        inventory: &SupervisorInventory,
    ) -> u64 {
        let mut held_max = 0u64;
        for alias in aliases.values() {
            let Some(entry) = inventory.get(alias) else { continue };
            let Ok(client) = self.connected_client(entry).await else { continue };
            if let Ok(Some(g)) = Self::read_held_generation(&client, app_instance_id).await {
                held_max = held_max.max(g);
            }
        }
        held_max
    }

    /// The mint/substitute/certify/apply pipeline shared by `submit` and
    /// `force-reconcile`.
    async fn deploy_submission(
        &self,
        mut plan: DeploymentPlan,
        inventory: &SupervisorInventory,
        generation: u64,
    ) -> Result<Vec<MintedMaster>, String> {
        let aliases = Self::placed_aliases(&plan)?;

        // Mint before connecting anywhere: a locked vault or a bad plan
        // must fail before the supervisor spends a network round trip on
        // substrates it cannot yet certify anything for.
        let (minted, masters) =
            keys::mint_and_substitute(&mut plan, &self.vault).await.map_err(|e| e.to_string())?;

        let clients = self.build_clients(&aliases, inventory).await?;

        let (instance_certs, registry_certs) = deploy::certify_placed_members(
            &plan,
            &masters,
            &clients,
            None,
            INSTANCE_CERT_EXPIRES_HOURS,
        )
        .await
        .map_err(|e| e.to_string())?;

        let deployment_id = self
            .store
            .journal
            .append(&plan, DeploymentState::Applying)
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

        let report = deploy::apply_plan(
            ApplyRequest {
                plan: &plan,
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

    async fn handle_submit(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (s,): (Submission,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse submit params: {e}")))?;

        let plan = DeploymentPlan::from_json(&s.plan_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid plan-json: {e}")))?;
        let inventory: SupervisorInventory = serde_json::from_str(&s.inventory_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid inventory-json: {e}")))?;

        let minted = self
            .deploy_submission(plan.clone(), &inventory, s.generation)
            .await
            .map_err(RpcError::InternalError)?;

        let plan_json_substituted = {
            // Re-run the substitution the deploy pipeline already performed
            // so the stored desired state carries the real master DIDs,
            // not the compiler's fabricated ones -- `mint_and_substitute`
            // resolves the same masters it already minted, so this is a
            // read, not a second mint.
            let mut p = plan;
            keys::mint_and_substitute(&mut p, &self.vault)
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            p.to_json().map_err(|e| RpcError::InternalError(e.to_string()))?
        };

        self.store
            .submit(
                &s.app_instance_id,
                &plan_json_substituted,
                &s.inventory_json,
                &caller.caller_did,
                s.generation,
            )
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let result = SubmitResult {
            masters: minted
                .into_iter()
                .map(|m| WitMintedMaster { service_name: m.service_name, master_did: m.master_did })
                .collect(),
        };
        Ok(NativeResponse { payload: serde_json::to_value(result).unwrap_or(Value::Null) })
    }

    async fn handle_adopt(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse adopt params: {e}")))?;

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

        let mut held_max = 0u64;
        for client in clients.values() {
            if let Some(g) = Self::read_held_generation(client, &app_instance_id)
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?
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
                    serde_json::to_value((app_instance_id.clone(), next_generation))
                        .map_err(|e| RpcError::InternalError(e.to_string()))?,
                )
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        }

        self.store
            .set_generation(&app_instance_id, next_generation)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        Ok(NativeResponse { payload: serde_json::to_value(next_generation).unwrap_or(Value::Null) })
    }

    /// Shared by `release` and `retire`: clears the management stamp on
    /// every substrate the instance is placed on.
    async fn release_on_every_substrate(&self, app_instance_id: &str) -> RpcResult<()> {
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
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;

        for client in clients.values() {
            client
                .request(
                    "orchestrator",
                    "release-app-instance",
                    serde_json::to_value((app_instance_id.to_string(), state.generation))
                        .map_err(|e| RpcError::InternalError(e.to_string()))?,
                )
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        }
        Ok(())
    }

    async fn handle_release(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse release params: {e}")))?;
        self.release_on_every_substrate(&app_instance_id).await?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "released"}) })
    }

    async fn handle_retire(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse retire params: {e}")))?;
        self.release_on_every_substrate(&app_instance_id).await?;
        self.store.retire(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "retired"}) })
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
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        for svc in &plan.services {
            match deploy::current_placement(&landed, &svc.logical_ref.to_string()) {
                None => expected.push(ExpectedService {
                    logical_ref: svc.logical_ref.clone(),
                    service_id: String::new(),
                    substrate_did: String::new(),
                }),
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
                    });
                    if let Some(alias) = &row.substrate_alias {
                        aliases.insert(row.substrate_did.clone(), alias.clone());
                    }
                }
            }
        }

        let mut targets: BTreeMap<String, HealthTarget> = BTreeMap::new();
        for (did, alias) in &aliases {
            let Some(entry) = inventory.get(alias) else { continue };
            let query: Arc<dyn StatusQuery> = match self.connected_client(entry).await {
                Ok(c) => Arc::new(c),
                Err(e) => Arc::new(UnreachableQuery(e.to_string())),
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
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        health::record_report(&self.store.alerts, &instance_id, &report, now)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // ADR-0021 §4 / matrix row 9: a substrate reporting a higher
        // generation than this supervisor holds means a second supervisor
        // has adopted the instance. Best-effort across every reachable
        // placed substrate -- one unreachable node must not hide a real
        // supersession visible through another.
        let held_max = self.max_held_generation(&app_instance_id, &aliases, &inventory).await;
        let superseded = held_max > state.generation;
        if superseded {
            self.store
                .alerts
                .raise(
                    &instance_id,
                    None,
                    None,
                    &self.node_did,
                    syneroym_app_orchestration::AlertKind::SupervisorSuperseded,
                    &format!(
                        "a managed substrate now holds generation {held_max}, higher than this \
                         supervisor's {}; another supervisor has adopted this instance",
                        state.generation
                    ),
                )
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        } else {
            self.store
                .alerts
                .clear(
                    &instance_id,
                    None,
                    &self.node_did,
                    syneroym_app_orchestration::AlertKind::SupervisorSuperseded,
                )
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        }

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
                restart_attempts: 0,
            })
            .collect();

        let overall_state = if state.retired {
            ManagedState::Retired
        } else if superseded {
            ManagedState::Superseded
        } else if state.paused {
            ManagedState::Paused
        } else if report.faults().is_empty() {
            ManagedState::Active
        } else {
            ManagedState::Degraded
        };

        let status = InstanceStatus {
            app_instance_id: app_instance_id.clone(),
            state: overall_state,
            generation: state.generation,
            supervisor_did: self.node_did.clone(),
            last_reconciled_at: Some(now),
            services,
            // A5b writes no bindings itself (`write-bindings` is the
            // resident loop's verb, A5c) -- there is nothing yet to
            // compare a substrate's observed epoch against, so this stays
            // empty rather than reporting a fabricated convergence.
            bindings: Vec::<BindingConvergence>::new(),
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
    use std::path::Path;

    use syneroym_rpc::AuthLevel;

    use super::*;

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
        SupervisorService::new("did:key:zSupervisorNode".to_string(), store, vault, &identity)
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
        SupervisorService::new("did:key:zSupervisorNode".to_string(), store, vault, &identity)
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
}

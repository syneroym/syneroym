use std::sync::Mutex;

use semver::Version;
use syneroym_app_orchestration::{
    DeploymentState,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, ServiceConfig, ServiceType,
        TopologyMode, TopologyVisibility,
    },
};
use syneroym_core::dht_registry::SignedEndpointInfo;

use super::*;

#[derive(Debug, Default)]
struct FailingApplier {
    should_fail: bool,
    calls: Mutex<Vec<WitDeploymentPlan>>,
}

#[async_trait::async_trait]
impl SubstrateActor for FailingApplier {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String> {
        self.calls.lock().unwrap().push(plan);
        if self.should_fail { Err("simulated failure".to_string()) } else { Ok(()) }
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by apply_plan's own tests")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by apply_plan's own tests")
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by apply_plan's own tests")
    }

    async fn instance_identity(&self, _service_id: &str) -> Result<InstanceIdentity, String> {
        unimplemented!("not exercised by apply_plan's own tests")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by apply_plan's own tests")
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
        visibility: Visibility::Private,
    }
}

fn service(name: &str, substrate: Option<&str>) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(format!("did:key:h{name}")),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(name),
        },
        substrate: substrate.map(SubstrateAlias::new),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: TopologyVisibility::Restricted,
    }
}

fn plan(services: Vec<PlannedService>) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: Version::parse("1.0.0").unwrap(),
        services,
    }
}

fn target(did: &str, alias: Option<&str>, actor: Arc<dyn SubstrateActor>) -> DeployTarget {
    DeployTarget { alias: alias.map(SubstrateAlias::new), substrate_did: did.to_string(), actor }
}

/// A `private` member gets no registry record at all -- not an empty
/// one, an absent one.
#[test]
fn member_registry_record_mints_nothing_for_a_private_member() {
    let master = Identity::generate().unwrap();
    let record =
        member_registry_record(Visibility::Private, "did:key:zSvc", "did:key:zSub", &master, 0)
            .unwrap();
    assert!(record.is_none());
}

/// `internal` signs `is_private: true`; `public`
/// signs `is_private: false`. This is the exact mapping ADR-0018 §4's
/// table specifies, and the one the record's own signature makes
/// impossible to correct downstream if it is ever wrong.
#[test]
fn member_registry_record_sets_is_private_from_the_declared_visibility() {
    let master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&master.public_key());

    let internal = member_registry_record(
        Visibility::Internal,
        &service_id,
        "did:key:zSub",
        &master,
        9_999_999_999,
    )
    .unwrap()
    .expect("internal must mint a record");
    let internal_signed: SignedEndpointInfo = serde_json::from_str(&internal).unwrap();
    assert!(internal_signed.info.is_private);
    assert_eq!(internal_signed.info.service_id, service_id);

    let public = member_registry_record(
        Visibility::Public,
        &service_id,
        "did:key:zSub",
        &master,
        9_999_999_999,
    )
    .unwrap()
    .expect("public must mint a record");
    let public_signed: SignedEndpointInfo = serde_json::from_str(&public).unwrap();
    assert!(!public_signed.info.is_private);

    // Both verify: the record's own signature, not just its shape, is
    // what a registry or an importer actually checks.
    assert!(internal_signed.verify().is_ok());
    assert!(public_signed.verify().is_ok());
}

#[test]
fn resolve_targets_fails_closed_naming_every_unknown_alias() {
    let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
    let targets = BTreeMap::new();
    let err = resolve_targets(&p, &targets, None).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("edge-1"));
    assert!(msg.contains("edge-2"));
}

#[tokio::test]
async fn apply_plan_deploys_each_service_to_its_own_target() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

    let applier_a = Arc::new(FailingApplier::default());
    let applier_b = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([
        (SubstrateAlias::new("edge-1"), target("did:key:zA", Some("edge-1"), applier_a.clone())),
        (SubstrateAlias::new("edge-2"), target("did:key:zB", Some("edge-2"), applier_b.clone())),
    ]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.deployed.len(), 2);
    assert!(report.failures.is_empty());
    assert_eq!(applier_a.calls.lock().unwrap().len(), 1);
    assert_eq!(applier_b.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn apply_plan_records_one_action_row_per_service_and_substrate() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

    let applier = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([(
        SubstrateAlias::new("edge-1"),
        target("did:key:zA", Some("edge-1"), applier),
    )]);

    apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    let completed = journal.get_completed_actions(deployment_id).unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].substrate_alias.as_deref(), Some("edge-1"));
    assert_eq!(completed[0].substrate_did, "did:key:zA");
}

#[tokio::test]
async fn apply_plan_continues_past_a_failure_and_reports_it() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

    let applier_a = Arc::new(FailingApplier { should_fail: true, ..Default::default() });
    let applier_b = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([
        (SubstrateAlias::new("edge-1"), target("did:key:zA", Some("edge-1"), applier_a)),
        (SubstrateAlias::new("edge-2"), target("did:key:zB", Some("edge-2"), applier_b)),
    ]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.deployed.len(), 1);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].error, "simulated failure");
    assert!(!report.is_complete());
}

#[tokio::test]
async fn apply_plan_skips_a_service_already_completed_on_the_same_substrate() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
    journal
        .append_action(
            deployment_id,
            "ADD",
            &p.services[0].member_ref().to_string(),
            Some("edge-1"),
            "did:key:zA",
            ActionState::Completed,
        )
        .unwrap();

    let applier = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([(
        SubstrateAlias::new("edge-1"),
        target("did:key:zA", Some("edge-1"), applier.clone()),
    )]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.skipped.len(), 1);
    assert!(report.deployed.is_empty());
    assert!(applier.calls.lock().unwrap().is_empty(), "a skipped service must not be re-applied");
}

#[tokio::test]
async fn apply_plan_does_not_skip_when_the_alias_now_resolves_to_a_different_did() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
    journal
        .append_action(
            deployment_id,
            "ADD",
            &p.services[0].member_ref().to_string(),
            Some("edge-1"),
            "did:key:zOldNode",
            ActionState::Completed,
        )
        .unwrap();

    // The alias now resolves to a different node than the completed row names.
    let applier = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([(
        SubstrateAlias::new("edge-1"),
        target("did:key:zNewNode", Some("edge-1"), applier.clone()),
    )]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.deployed.len(), 1);
    assert!(report.skipped.is_empty());
    assert_eq!(applier.calls.lock().unwrap().len(), 1);
}

/// `app forget` appends a `REMOVE` row for the
/// same (logical ref, DID) an earlier `ADD` in this very record already
/// completed. A skip check scoped to "does any completed ADD match"
/// would still find that ADD and skip -- reporting a service "already
/// applied" while nothing is running there, since it was forgotten. The
/// most-recent-row-wins reading must see the `REMOVE` and redeploy.
#[tokio::test]
async fn apply_plan_redeploys_a_service_whose_most_recent_row_is_remove() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1"))]);
    let deployment_id = journal.append(&p, DeploymentState::Degraded).unwrap();
    let l_ref = p.services[0].member_ref().to_string();
    journal
        .append_action(
            deployment_id,
            "ADD",
            &l_ref,
            Some("edge-1"),
            "did:key:zA",
            ActionState::Completed,
        )
        .unwrap();
    // `app forget`'s own write: a REMOVE row for the same (ref, DID),
    // appended to the same record.
    journal
        .append_action(
            deployment_id,
            "REMOVE",
            &l_ref,
            Some("edge-1"),
            "did:key:zA",
            ActionState::Completed,
        )
        .unwrap();

    let applier = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([(
        SubstrateAlias::new("edge-1"),
        target("did:key:zA", Some("edge-1"), applier.clone()),
    )]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.deployed.len(), 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(applier.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_mapping_error_fails_only_its_own_service() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let mut bad = service("a", Some("edge-1"));
    bad.config.service_type = ServiceType::Wasm;
    bad.config.source = "does-not-exist.wasm".to_string();
    let p = plan(vec![bad, service("b", Some("edge-2"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

    let applier_a = Arc::new(FailingApplier::default());
    let applier_b = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([
        (SubstrateAlias::new("edge-1"), target("did:key:zA", Some("edge-1"), applier_a.clone())),
        (SubstrateAlias::new("edge-2"), target("did:key:zB", Some("edge-2"), applier_b.clone())),
    ]);

    let report = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &BTreeMap::new(),
            registry_certificates: &BTreeMap::new(),
            emit_bindings: false,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();

    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.deployed.len(), 1);
    assert!(applier_a.calls.lock().unwrap().is_empty(), "the failing mapper must not be applied");
    assert_eq!(applier_b.calls.lock().unwrap().len(), 1);
}

/// `apply_plan` holds `&DeploymentJournal` across every `.await`,
/// so its future is `Send` only because the journal itself now is
/// (`Arc<Mutex<Connection>>`, not a bare `Connection`) -- required for
/// a supervisor's reconcile loop to `tokio::spawn` a call to this
/// function.
#[test]
fn apply_plan_returns_a_send_future() {
    fn assert_send<T: Send>(_: T) {}

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let p = plan(vec![service("a", Some("edge-1"))]);
    let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
    let applier = Arc::new(FailingApplier::default());
    let targets = BTreeMap::from([(
        SubstrateAlias::new("edge-1"),
        target("did:key:zA", Some("edge-1"), applier),
    )]);

    let no_certs = BTreeMap::new();
    let no_epochs = BTreeMap::new();
    let fut = apply_plan(
        ApplyRequest {
            plan: &p,
            targets: &targets,
            fallback: None,
            instance_certificates: &no_certs,
            registry_certificates: &no_certs,
            emit_bindings: false,
            generation: 0,
            binding_epochs: &no_epochs,
        },
        &journal,
        deployment_id,
    );
    assert_send(fut);
}

// ── The durable actor's try-then-queue decision ────────────────────

/// A fake standing in for `SyneroymClient`: `attempt_write_bindings`
/// returns whatever `outcome` says, and `write_bindings`/every other
/// `SubstrateActor` method is exercised only where a test names it, so
/// each one's own `should_fail` flag drives it directly rather than
/// sharing one flag across five unrelated actions.
#[derive(Debug, Default)]
struct DurableTestActor {
    write_bindings_outcome: Mutex<Option<Result<Vec<BindingWriteOutcome>>>>,
    apply_plan_should_fail: bool,
    restart_should_fail: bool,
    renew_cert_should_fail: bool,
    run_scheduled_should_fail: bool,
}

#[async_trait::async_trait]
impl WriteBindingsAttempt for DurableTestActor {
    async fn attempt_write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>> {
        self.write_bindings_outcome.lock().unwrap().take().expect("outcome not set for test")
    }
}

#[async_trait::async_trait]
impl SubstrateActor for DurableTestActor {
    async fn apply_plan(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
        if self.apply_plan_should_fail { Err("apply_plan failed".to_string()) } else { Ok(()) }
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("DurableActor<T> never calls T::write_bindings directly")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        if self.restart_should_fail { Err("restart failed".to_string()) } else { Ok(()) }
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        if self.renew_cert_should_fail { Err("renew_cert failed".to_string()) } else { Ok(()) }
    }

    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        if self.run_scheduled_should_fail {
            Err("run_scheduled failed".to_string())
        } else {
            Ok(())
        }
    }

    async fn instance_identity(&self, _service_id: &str) -> Result<InstanceIdentity, String> {
        unimplemented!("not exercised by the durable-actor tests")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by the durable-actor tests")
    }
}

#[derive(Debug, Default)]
struct RecordingOutbox {
    enqueued: Mutex<Vec<(String, String, BindingWrite)>>,
}

#[async_trait::async_trait]
impl WriteBindingsOutbox for RecordingOutbox {
    async fn enqueue(&self, queue_key: &str, substrate_did: &str, write: &BindingWrite) {
        self.enqueued.lock().unwrap().push((
            queue_key.to_string(),
            substrate_did.to_string(),
            write.clone(),
        ));
    }
}

fn binding_write() -> BindingWrite {
    BindingWrite {
        service_id: "did:key:zSvc".to_string(),
        app_instance_id: "inst-1".to_string(),
        bindings: vec![],
        generation: 0,
    }
}

/// A successful call returns exactly what the trait returns today, with
/// no queue involvement.
#[tokio::test]
async fn a_successful_write_bindings_returns_the_same_outcomes_it_returns_today() {
    let inner = Arc::new(DurableTestActor {
        write_bindings_outcome: Mutex::new(Some(Ok(vec![BindingWriteOutcome::Applied]))),
        ..Default::default()
    });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    let outcomes = actor.write_bindings(binding_write()).await.unwrap();
    assert_eq!(outcomes, vec![BindingWriteOutcome::Applied]);
    assert!(outbox.enqueued.lock().unwrap().is_empty());
}

/// The enqueue half; `Degraded` reporting is `push_bindings`'s own
/// concern in `app_supervisor`, unchanged by this wrap: a transport
/// failure enqueues before returning the same error a bare client
/// would have.
#[tokio::test]
async fn a_transport_failure_enqueues_and_returns_the_same_error_a_bare_client_would() {
    let inner = Arc::new(DurableTestActor {
        write_bindings_outcome: Mutex::new(Some(Err(anyhow::anyhow!("connection refused")))),
        ..Default::default()
    });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    let err = actor.write_bindings(binding_write()).await.unwrap_err();
    assert!(err.contains("connection refused"), "{err}");
    let enqueued = outbox.enqueued.lock().unwrap();
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].0, "inst-1/backend@did:key:zB");
    assert_eq!(enqueued[0].1, "did:key:zB");
}

/// A callee error (the substrate reached and refused the
/// call) is never enqueued -- retrying the same refusal forever would
/// be a second policy competing with the one that already answered.
#[tokio::test]
async fn a_callee_error_is_not_enqueued() {
    let callee_err =
        JsonRpcError { code: -32010, message: "not authorized".to_string(), data: None };
    let inner = Arc::new(DurableTestActor {
        write_bindings_outcome: Mutex::new(Some(Err(callee_err.into()))),
        ..Default::default()
    });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    let err = actor.write_bindings(binding_write()).await.unwrap_err();
    assert!(err.contains("not authorized"), "{err}");
    assert!(outbox.enqueued.lock().unwrap().is_empty(), "a callee error must not be queued");
}

/// `restart` never touches the outbox at all, whatever it returns.
#[tokio::test]
async fn a_failed_restart_is_never_enqueued() {
    let inner = Arc::new(DurableTestActor { restart_should_fail: true, ..Default::default() });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    let err = actor.restart("svc".to_string(), 0).await.unwrap_err();
    assert_eq!(err, "restart failed");
    assert!(outbox.enqueued.lock().unwrap().is_empty());
}

/// A scheduled run is never queued, the same
/// shape `restart` already proved: `DurableActor` forwards the call
/// straight through and touches the outbox not at all, whatever the
/// inner actor returns.
#[tokio::test]
async fn a_durable_actor_runs_a_scheduled_tick_without_touching_the_queue() {
    let inner =
        Arc::new(DurableTestActor { run_scheduled_should_fail: true, ..Default::default() });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    let err = actor
        .run_scheduled(
            "svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(err, "run_scheduled failed");
    assert!(outbox.enqueued.lock().unwrap().is_empty());
}

/// `apply_plan` and `renew_cert` embed a certificate that expires in
/// hours, so neither is ever queued either, whatever it returns.
#[tokio::test]
async fn a_failed_apply_plan_and_renew_cert_are_never_enqueued() {
    let inner = Arc::new(DurableTestActor {
        apply_plan_should_fail: true,
        renew_cert_should_fail: true,
        ..Default::default()
    });
    let outbox = Arc::new(RecordingOutbox::default());
    let actor = build_durable_actor(
        inner,
        "did:key:zB".to_string(),
        "inst-1/backend@did:key:zB".to_string(),
        outbox.clone(),
    );

    assert!(
        actor
            .apply_plan(WitDeploymentPlan {
                app_instance_id: "inst-1".to_string(),
                blueprint_id: "syneroym:test".to_string(),
                version: "1.0.0".to_string(),
                services: vec![],
            })
            .await
            .is_err()
    );
    assert!(actor.renew_cert("svc".to_string(), 0, "cert".to_string()).await.is_err());
    assert!(outbox.enqueued.lock().unwrap().is_empty());
}

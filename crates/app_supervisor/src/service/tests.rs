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
        AppBlueprintId, InterfaceName, LogicalServiceRef, ServiceConfig, ServiceType, TopologyMode,
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
    /// refuses reads. A disabled-encryption fixture proves nothing
    /// about the locked case.
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
    /// on its own `_fixture_tempdir` field, for its own lifetime. The
    /// handover test needs two fixture-built services to share one
    /// backup directory (a stand-in for two supervisors handed the
    /// same operator-carried file), which the default cannot do -- so
    /// the test owns and passes one in, held for the whole test.
    backup_dir: Option<PathBuf>,
    /// `None` leaves the default (5s). Tests driving the queue worker
    /// against a fake clock set this explicitly.
    queue_tick_secs: Option<u64>,
    /// `None` leaves the default (30s). Tests proving recovery happens
    /// within one worker tick rather than one poll interval set this
    /// explicitly, far above the tick.
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
    /// way to reach the race where `kek_is_loaded()` answers
    /// "unlocked" and the vault read that follows still fails.
    fn build_with_key_store(self) -> (SupervisorService, Arc<syneroym_data_keystore::KeyStore>) {
        // Kept alive on the returned `SupervisorService` itself
        // (`_fixture_tempdir`), not left to drop here: a dropped
        // `TempDir` deletes the directory tree from disk the instant
        // this function returns, while `SqliteStorageProvider` already
        // holds an open connection into it (found while
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
        // leaking every fixture-built test's directory permanently.
        // Tying its lifetime to the service's own restores ordinary
        // cleanup on every ordinary test's `Drop`, ~150 of them,
        // while keeping the fix for the handful that mint under
        // encryption.
        let dir = tempfile::tempdir().unwrap();
        let store = SupervisorStore::open_in_memory().unwrap();
        let storage_provider: Arc<dyn syneroym_data_db::traits::StorageProvider> = Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.path().join("db"), self.locked_vault)
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
        // No new resource namespace -- gated exactly like the
        // neighbouring verbs above.
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

/// The plan's own `app_instance_id` and the submission's outer
/// `app_instance_id` are both caller-supplied and, before this check,
/// never compared. A mismatch would key the journal and vault under
/// one instance while `status`/`adopt`/`retire` key on the other,
/// splitting the instance in two.
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

/// `submit` is the backstop entry point for the open-topology /
/// private-service contradiction check -- a plan reaching the
/// supervisor was never necessarily compiled through `compile()` (a
/// hand-built plan, or a client other than `roymctl`), so the same
/// refusal must fire here too, not only inside the compiler.
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

/// `deploy_submission` used to run the whole mint/certify/apply
/// pipeline *before* `store.submit`'s retired guard
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

/// The generation check lived only at `store.submit`, which still ran
/// *after* `deploy_submission` --
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

/// Before `connect_best_effort`, `release_on_every_substrate` used
/// `build_clients`, whose contract
/// fails the whole call the moment one placed alias cannot be
/// reached -- so retiring an instance placed on even one unreachable
/// substrate was permanently impossible. `ucan: null` fails fast at
/// the credential check rather than waiting out a real connect
/// timeout; either way is "cannot reach it" for this purpose.
#[tokio::test]
async fn retire_succeeds_and_marks_the_store_retired_even_when_a_placed_substrate_is_unreachable() {
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
    let unreleased = res.payload.get("unreleased_substrates").and_then(|v| v.as_array()).unwrap();
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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// The journal keys every completed action row on a `MemberRef`, not a
/// bare `LogicalServiceRef` -- if `handle_status`'s own
/// expected-service builder (one of three, alongside the loop's sweep
/// and `roymctl`'s two) ever went back to reading it by the old key,
/// member 1's placement would silently stop matching and this service
/// would report `substrate_did` empty and land in `missing_placement`
/// even though it is fully landed. Scaled (index 1, not 0) on purpose:
/// an unscaled member's `MemberRef` string is unchanged from the
/// bare-ref era and would not catch a regression to the old key.
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
        "member 1's completed placement must be found by its own MemberRef, not read as missing: \
         {:?}",
        status.services[0]
    );
    assert_ne!(
        status.services[0].signal, "instance-not-running",
        "a landed member must not be reported as never-deployed: {:?}",
        status.services[0]
    );
}

/// A re-submit that moves a landed service to a different substrate
/// must be refused before anything is deployed -- an early version
/// shipped `submit` with no such check, so this silently ran a second
/// live copy of the same member.
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

/// `force-reconcile` never calls `store.submit`, so it needs its own
/// placement check -- without one
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

/// A planned service the journal has never recorded landed must report
/// the instance `Degraded`, not `Active` -- an earlier gap, since
/// `Signal::NotDeployed` is deliberately not a fault.
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

/// A reconcile in flight is now observable -- `apply_with_clients`
/// writes `Applying` before it
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

/// The whole point of the fix is that an alias serving double duty --
/// both a landed placement's alias and
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

    let record_plan = SupervisorService::record_plan_for_pass(&plan, &needs_work, &BTreeMap::new());

    assert!(record_plan.services.is_empty(), "an unreachable needs_work service must not land");
}

// ── MQTT alert publication ─────────────────────────────────────────

fn expected_alert_topic(app_instance_id: &str) -> String {
    namespace_topic_for_publish(
        SUPERVISOR_RESERVED_SERVICE_ID,
        &format!("supervisor/alerts/{app_instance_id}"),
    )
}

/// A sweep that opens a new alert publishes it under the supervisor's
/// own topic -- `<alert_topic>/<app_instance_id>`,
/// namespaced with the publish-side rule under
/// `SUPERVISOR_RESERVED_SERVICE_ID`, the exact string the router's own
/// subscribe-side fix (`dispatch.rs::subscribe_namespaced_topic`)
/// produces for the same service id.
#[tokio::test]
async fn a_newly_opened_alert_is_published_under_the_supervisors_own_topic() {
    let s = service();
    // No substrate placement: the sweep reports `not-deployed`, which
    // becomes a raised `InstanceNotRunning` alert -- the cheapest
    // fixture that opens a real alert with no live substrate.
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let topic = expected_alert_topic("inst-1");
    let (_handle, mut receiver) = s.messaging_broker.subscribe(topic.clone()).await.unwrap();

    dispatch(&s, admin_caller("did:key:zSupervisorNode"), "status", serde_json::json!(["inst-1"]))
        .await
        .unwrap();

    let (received_topic, payload) = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("did not time out waiting for the published alert")
        .expect("broker channel closed");
    assert_eq!(received_topic, topic);
    let value: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(value["app_instance_id"], "inst-1");
    assert_eq!(value["kind"], AlertKind::InstanceNotRunning.to_string());
}

/// `publish_opened_alerts` returns `()`, not a `Result` -- there is no
/// `?` for a publish failure to propagate through, by
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
fn plan_json_one_service(instance: &str, service_name: &str, substrate: Option<&str>) -> String {
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

/// A by-construction property, pinned so a later change cannot quietly
/// reintroduce a key-bearing verb: walks the WIT interface and asserts
/// no function or record field is named like key material.
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
                assert!(!suspicious(&field.name), "field '{}' looks like key material", field.name);
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

// ── The resident loop ──────────────────────────────────────────────

/// `all_active` already excludes both flags from the loop's own work
/// list, so a pass over either instance never runs at
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

/// `apply_write_phase` is the write phase `reconcile_instance_pass`
/// calls after its health sweep -- this tests its own re-read
/// directly, standing in for a `pause` that lands during the sweep
/// (which does not hold the per-instance lock a pass otherwise holds
/// for its whole duration). If the write phase used the state the pass
/// started with instead of re-reading, this would append a journal
/// record; it must not.
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

/// A placement change was already refused, but nothing raised an alert
/// for it -- only `Display`/`FromStr` ever touched the variant. A
/// refusal must now be visible on `alerts`, not only as this call's
/// own `Err`.
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
        SupervisorInventoryEntry { did: "did:key:zEdge2".to_string(), api_url: None, ucan: None },
    )]);

    let err = s.refuse_placement_change(&moved_plan, &inventory).await.unwrap_err();
    assert!(err.contains("does not relocate"), "{err}");

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(alerts.iter().any(|a| a.kind == AlertKind::PlacementChangeRefused), "{alerts:?}");
}

/// `refuse_placement_change` used to compare a member's plan entry
/// against `current_placement(&landed, &l_ref)` keyed on the bare
/// logical ref, so with two members placed on different substrates,
/// member 1's entry was compared against member 0's landed row --
/// different DIDs, refused as a relocation though nothing moved.
/// Keying on `member_ref()` is what makes cross-substrate `replicas`
/// even expressible.
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

/// `update_superseded_alert` is the exact decision
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

/// `instance_lock` itself, which two concurrently driven holders for
/// the *same* instance id must never both be inside at once.
/// `instance_lock` for two *different* ids would return two different
/// mutexes and is not what this proves. This pins the lock's own
/// mutual exclusion, not that `submit` and a loop pass actually reach
/// for it -- the two tests below drive the real methods.
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

/// The loop is spawned, not pinned in a `select!` that would drop it
/// mid-pass -- `shutdown` only cancels the token
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

/// Pins `run`'s interval configuration directly, under a paused clock
/// rather than a real slow pass -- `Skip` must let a
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

// ── Remediation ────────────────────────────────────────────────────

/// A fake `SubstrateActor` that only counts `restart` calls -- every
/// other method is unreachable from a remediation test.
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

/// A landed service the sweep finds `InstanceNotRunning` gets one
/// bounded restart attempt.
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

/// Two members of one scaled service must each spend their own
/// `max_restart_attempts` budget -- `restart_candidates` keys on
/// `ServiceHealth::member_ref()` (member 0 and member 1 are two
/// distinct candidates), and `attempt_restart`'s remediation row is
/// keyed on that same string.
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
    assert_eq!(*actor.restart_calls.lock().unwrap(), 1, "the second attempt was inside backoff");
    assert_eq!(s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().attempts, 1);
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

/// A declared readiness probe failing is an author assertion this
/// supervisor cannot verify, not a substrate-verified fact -- alert
/// only, pinned so a later change cannot silently widen remediation
/// onto it.
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

/// A substrate that did not answer is never inferred to mean its
/// services are down, so restarting cannot be the fix for it either.
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

/// A service the resubmitted plan no longer names, but that this
/// supervisor's own journal still shows
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

// ── The binding push and convergence ───────────────────────────────

/// A fake `SubstrateActor` that only answers `write_bindings`, from a
/// caller-queued sequence of responses (defaulting to `Applied` once
/// the queue empties) -- every other method is unreachable from a
/// push test.
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

/// The convergence budget is measured from the membership change to
/// the last applied write returning `Applied`/`NoOp` --
/// *not* off `binding-epochs`, whose own refresh is bounded by
/// `poll_interval_secs` (default 30s, six times the 5s budget). This
/// harness proves the two are not the same clock: a push against a
/// fake actor that answers immediately completes in a time nowhere
/// near a poll interval, so a measurement taken this way is the
/// write's own latency, never silently the read surface's lag.
///
/// The clock starts at `Reconciler::compute_diff` and runs through
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
    let (_, push_candidates) = SupervisorService::classify_update_actions(&landed, &diff.actions);
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
        "the measured interval must cover the routing decision and the write's own latency, far \
         under a poll interval (default 30s) and the 5s budget alike, not `binding-epochs`' own \
         read lag: {elapsed:?}"
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

/// A write with zero bindings is a real, converged success -- not the
/// same value `push_bindings`
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

/// `map_deployment_plan_to_wit` reads a binding's `mode` off the
/// *dependency's own* `PlannedService.topology_mode` in the plan --
/// not off `resolved_dependencies`' member count -- so `backend` must
/// be present in `plan.services` with `Redundant` already compiled
/// onto it (`replicas > 1` implies `Redundant`) for the push to carry
/// it correctly. Proves the flip at the binding-write layer itself,
/// without needing a live substrate to observe cross-member
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

    s.push_bindings(&instance_id, &plan, &frontend, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
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

/// An operator reads a converged binding once the written and observed
/// epochs agree. Read directly
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

// ── The push trigger ───────────────────────────────────────────────

/// The classifier's whole point. A resubmit whose only change to a
/// dependent member is which DIDs a dependency resolves to must be
/// routed to a push, not a redeploy.
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
        .apply_with_membership_pushes(&new_plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
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
        "a record carrying an unlanded push must not read as this instance's converged baseline: \
         {latest:?}"
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
async fn a_diff_that_also_changes_config_still_takes_the_redeploy_path_through_membership_pushes() {
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
        .apply_with_membership_pushes(&new_plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .unwrap_err();
    assert!(
        !err.contains("not connected to its landed substrate this call"),
        "a config change must not be routed through the push path: {err}"
    );
}

/// `Reconciler::compute_diff` produces one `Update` action per member
/// of the scaled dependency's dependent -- each is
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
    let new_plan = DeploymentPlan { services: vec![frontend_0, frontend_1], ..old_plan.clone() };

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

/// A push candidate this pass could not even connect to used to be
/// dropped with a bare `continue` -- no alert, no `Degraded`. Drives
/// `apply_write_phase` directly (the same entry point
/// `a_pause_landing_mid_pass_stops_that_passs_writes` uses) with
/// `did_to_alias` empty, standing in for a dependent whose substrate
/// this pass's own connect step never reached.
#[tokio::test]
async fn an_unreachable_push_candidate_raises_binding_conflict_instead_of_being_dropped_silently() {
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

/// The narrower loop-path shape:
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
async fn a_needs_work_redeploy_and_a_failing_push_in_the_same_pass_leaves_the_record_degraded() {
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
            .append_action(deployment_id, "ADD", l_ref, Some(alias), did, ActionState::Completed)
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
        "the redeploy landed (vacuously, backend was revoked and filtered out) but the push did \
         not -- the record must not read as this instance's converged baseline: {latest:?}"
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
        "nothing was journaled this pass -- the pre-existing record must be left alone: {latest:?}"
    );
}

/// `push_bindings` clears `BindingConflict` for that member once a
/// later push lands cleanly -- the clear site this alert kind never
/// had before.
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
        "the active alert set (what handle_status's overall_state reads) must be clear once the \
         conflict clears"
    );
}

/// The raise site must write the substrate's real DID into the
/// alert's `substrate_did` column, not `svc.substrate`
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
    s.push_bindings(&instance_id, &plan, &frontend_1, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 0);
    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#1").unwrap(), 1);
}

/// The test above calls `binding_convergence_rows` directly -- never
/// through a real `status` response, so nothing pins the wire shape
/// (`InstanceStatus.bindings` field name, its serialization) a caller
/// actually reads. This drives the exact same declared
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

/// The negative half: a dependent absent from the sweep (unreachable,
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

/// A push against a dependent that cannot be reached fails (the epoch
/// has still advanced -- the invariant that the next attempt must
/// never retry at an epoch already spent), is visible on the operator
/// read surface (an earlier version of this test asserted only the
/// epoch and a successful retry, and never checked `opened`/`alerts`
/// at all), and succeeds cleanly once that dependent answers again. A
/// unit test against a fake actor, deliberately: the wire path is
/// already proven live by `binding_push_e2e.rs`, so this is entirely
/// the supervisor's own control flow.
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

/// `push_bindings` advances the binding epoch before every attempt,
/// but a *durable* actor only enqueues
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
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &first_actor, 0, &mut opened)
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
    let second_actor =
        deploy::build_durable_actor(second_client, "did:key:zEdge1".to_string(), queue_key, outbox);
    let second = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &second_actor, 0, &mut opened)
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
    let payload: outbox::QueuedBindingWrite = serde_json::from_slice(&queued[0].payload).unwrap();
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

// ── Unattended renewal, anchor refresh, revocation ─────────────────

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

    async fn revoke_instance(&self, master: &Identity, instance_did: &str) -> Result<(), String> {
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
fn plan_json_with_master(service_name: &str, master_did: &str, rotation_policy: &str) -> String {
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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    let report = report_of(vec![near_expiry_health("backend", &master_did)]);

    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
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
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
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
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    assert_eq!(candidates.len(), 1, "{candidates:?}");
}

/// D-A5d-4: a locked vault skips the renewal work-list and nothing
/// else -- health, remediation, and the anchor check all continue,
/// since none of them opens the vault.
#[tokio::test]
async fn a_locked_vault_skips_renewal_but_not_health_or_remediation_this_pass() {
    let s = service_with_locked_vault();
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
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
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
            .unwrap();
    let report = report_of(vec![
        near_expiry_health("backend", "did:key:hBackend"),
        near_expiry_health("frontend", "did:key:hFrontend"),
        fresh_health("worker", "did:key:hWorker"),
    ]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
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
    assert_eq!(opened, vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]);
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
    assert_eq!(opened, vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]);
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
    assert_eq!(opened, vec![(AlertKind::RotationRestartPending, "inst-1/backend#0".to_string())]);
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

/// The clearing rule: a raised alert with no path back to cleared is a
/// bug. Recomputed from the substrate's own answer, not tracked as a
/// flag -- so a renewal that succeeded out of band clears these too.
#[tokio::test]
async fn certificate_near_expiry_clears_on_the_next_passs_healthy_read() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    for kind in
        [AlertKind::CertificateNearExpiry, AlertKind::CertificateExpired, AlertKind::VaultLocked]
    {
        s.store
            .alerts
            .raise(&instance_id, Some("inst-1/backend#0"), None, "did:key:zEdge1", kind, "stalled")
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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
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
async fn a_vault_error_locked_race_during_mint_raises_vault_locked_not_certificate_near_expiry() {
    // An encrypted vault, open at construction so the cheap up-front
    // check passes, then closed again before the mint -- the exact
    // ordering the carve-out exists for, produced directly rather than
    // raced.
    let (s, key_store) =
        Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
            .build_with_key_store();
    assert!(s.vault.kek_is_loaded());

    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
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
        "a vault lock found mid-mint is the same condition as one found up front, and must not \
         surface under a different alert kind"
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
        names.iter().map(|n| near_expiry_health(n, &format!("did:key:h{n}"))).collect::<Vec<_>>(),
    );

    let first =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
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

    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 1);

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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    s.store.record_master_anchor_refresh(&master_did, NOW as i64 - 43_201).unwrap();

    s.refresh_due_master_anchors(&plan, NOW).await;

    assert_eq!(*writer.refreshed.lock().unwrap(), vec![master_did]);
}

/// `refresh_due_master_anchors` reads `svc.member_index` to resolve
/// which master to sign with (`keys::master_for_member`) -- a
/// regression to a hardcoded `0` would silently sign member 1's anchor
/// with member 0's key instead of failing loudly. Asserts the *key
/// the writer actually received*, not merely that a call happened.
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
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();

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

/// A row with no app master DID yet (before this instance's first
/// `adopt`) is skipped without even opening the vault, and nothing
/// gets minted to fill the gap.
#[tokio::test]
async fn an_instance_with_no_app_master_did_is_skipped_and_nothing_is_minted() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    let state = desired_state_with_app_master("inst-1", "");
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened).await;

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

/// A supervisor with no registry configured holds no writer at all
/// (mirroring `RegistryAnchorWriter::from_registry_client`), and a
/// pass with none configured is a quiet no-op, never a panic. The
/// `RegistryTier1Writer::from_registry_client` half is asserted
/// directly here; the warning log line itself is written once at
/// supervisor init (`runtime.rs`), outside what a `SupervisorService`
/// unit test can reach.
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

    s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened).await;

    assert_eq!(
        s.store.last_tier1_refresh(&app_did).unwrap(),
        None,
        "with no writer configured there is nothing to stamp"
    );
}

/// The real property is not the interval, it is that a failure never
/// gives up: every pass this many seconds after the last success
/// retries again, with nothing here counting attempts toward a cap.
/// `tier1_refresh_survives_sixty_consecutive_failures_against_the_default_interval`
/// (`syneroym-core`) pins the number this buys against `EndpointInfo`'s
/// 30-day `not_after`.
#[tokio::test]
async fn a_failed_tier1_publish_is_retried_every_interval_with_no_internal_cap() {
    let failing = Arc::new(RecordingTier1Writer { fail: true, ..RecordingTier1Writer::default() });
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
/// a hardcoded one -- `generation` is the entire mechanism that keeps
/// two supervisors publishing one app DID from colliding.
#[tokio::test]
async fn a_refresh_runs_on_the_ordinary_pass_tick_and_carries_the_instances_generation() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0).unwrap();
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
    let s =
        Fixture { locked_vault: true, tier1_writer: Some(writer.clone()), ..Fixture::default() }
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
/// on that answer (which this refresh briefly copied) would skip this
/// instance's Tier-1 publish forever and raise a `VaultLocked` alert
/// that is never true. Reading the real attempt's own
/// `VaultError::Locked` instead must reach the writer here, where the
/// vault is merely unencrypted, not locked.
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

/// The vault's own key must match the DID the row recorded at the last
/// `adopt` -- a mismatch (an `import-master` not yet followed by an
/// `adopt`) must refuse and raise, never publish under whichever key
/// the vault happens to hold.
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

/// Companion to the two loop paused-instance tests: `pause` excludes
/// an instance from the write phase entirely, and the Tier-1 refresh
/// is no exception.
#[tokio::test]
async fn a_paused_instance_gets_no_tier1_refresh() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0).unwrap();
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
    let plan = DeploymentPlan::from_json(&plan_json_two_services("inst-1", "backend", "frontend"))
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
    assert!(err.contains("hFabricatedB"), "the rest of the plan must still be attempted: {err}");

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    let revoked: Vec<_> = alerts.iter().filter(|a| a.kind == AlertKind::InstanceRevoked).collect();
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
    s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new()).await.unwrap();

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
    s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new()).await.unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    let after_first_apply = s.store.journal.get_latest(&instance_id).unwrap().unwrap();

    // Checked directly against what the
    // next pass's own diff would read: the revoked member must not
    // show up as a fresh `Add` against the baseline the call above
    // just journaled.
    let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
    assert!(
        diff.actions.is_empty(),
        "a revoked member must not read back as a change against its own just-journaled baseline: \
         {:?}",
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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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
    assert_eq!(SupervisorService::select_revocation_did(installed), "did:key:zActuallyInstalled");

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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

// ── The app-instance master identity ──────────────────────────────

fn adopt_field<'a>(res: &'a NativeResponse, field: &str) -> Option<&'a Value> {
    res.payload.get(field)
}

/// The ordinary path, over a services-less plan so no substrate is
/// involved -- `adopt` mints an app master and both the vault and the
/// instance row carry it afterwards.
#[tokio::test]
async fn adopt_mints_an_app_master_and_records_it_on_the_instance_row() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// A locked vault refuses the whole call, before a generation is
/// claimed and before any key is minted -- not through a
/// `kek_is_loaded` pre-check. The locked fixture (encryption on, no
/// KEK) is the only shape that proves anything about locking.
#[tokio::test]
async fn adopt_on_a_locked_vault_refuses_before_it_claims_a_generation() {
    let s = Fixture { locked_vault: true, ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// `adopt` is the only mint point, stated as a decision rather than
/// merely true of the paths tested so far. Another test shows the
/// field absent right after `submit`; this covers the two paths most
/// likely to grow a mint by accident later, since both re-run the
/// same apply pipeline `adopt` does over the identical plan --
/// `force-reconcile` and one resident-loop pass.
#[tokio::test]
async fn app_master_did_stays_empty_through_force_reconcile_and_a_loop_pass_without_adopt() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// The DID stays stable across two `adopt`s -- resolving, not minting,
/// on the second call. Over a services-less plan, the generation
/// itself stays `1` on both calls too: `claim_next_generation` reads
/// the held maximum only from the substrates the plan places services
/// on, and an empty plan has none to remember a prior claim, so this
/// in-process shape cannot demonstrate the generation actually
/// advancing -- that needs a real substrate, which the e2e proves
/// alongside DID stability. Named for what it actually asserts, not
/// for an increment this test cannot produce.
#[tokio::test]
async fn a_second_adopt_reports_the_same_app_master_did() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// `status` reports the same DID `adopt` minted.
#[tokio::test]
async fn status_reports_the_app_master_did_of_an_adopted_instance() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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

/// Absent, not an empty string -- an instance that has never been
/// adopted must not read as though it holds a DID of `""`.
#[tokio::test]
async fn status_reports_no_app_master_for_an_instance_that_was_never_adopted() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// `status` reports the currently-published Tier-1 record's expiry,
/// derived from the last successful refresh this supervisor
/// stamped -- not from a fresh registry lookup.
#[tokio::test]
async fn status_reports_the_tier_one_record_expiry() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// `pause`'s response reports the date its own write-phase skip
/// (`a_paused_instance_gets_no_tier1_refresh`, above) will let the
/// currently-published record decay to. Named for the response field
/// this actually asserts, not the `tracing::warn!` alongside it --
/// that log line is real (`handle_pause`) but outside what a
/// dispatch-level RPC test can capture without the same
/// `run_capturing_logs`-shaped machinery `keys.rs` uses.
#[tokio::test]
async fn pause_reports_the_records_expiry_date() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

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

/// The handover-order repair inside one vault -- mint by adopting,
/// import a different key under the same name (simulating an
/// operator-carried backup replacing this vault's own key), adopt
/// again, and the row follows the vault rather than keeping the
/// replaced DID.
#[tokio::test]
async fn adopt_after_an_import_records_the_imported_did_not_the_one_it_replaced() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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

/// An instance whose row predates the app-master column -- generation
/// already claimed, `app_master_did` empty -- gains one at its *next*
/// `adopt`, never anywhere else. Simulated by writing that older state
/// directly rather than going through `adopt` to reach it.
#[tokio::test]
async fn an_instance_row_with_no_app_master_gains_one_on_its_next_adopt() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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
    // The generation bump itself is the named cost -- not asserted at
    // a specific number here, since a services-less plan has no
    // substrate to remember `2` was already claimed and
    // `claim_next_generation` always computes fresh from what the plan
    // places, which is nothing.
    let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
    assert!(did.starts_with("did:key:"));
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, did);
}

/// An earlier failure, asserted directly -- the returned name must be
/// one `export-master` actually accepts, not the bare logical name.
#[tokio::test]
async fn adopt_returns_the_vault_name_export_master_accepts() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let vault_name = adopt_field(&res, "vault_name").and_then(Value::as_str).unwrap().to_string();

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

/// `status` must stay readable through a genuinely locked vault -- the
/// column exists precisely so the app's identity is visible while the
/// vault is shut, and this is the only test in this file that reaches
/// that state over one held service rather than a fresh, empty
/// rebuild. Locked *in place*: unlocked at construction so `adopt` can
/// mint, then the KEK is cleared afterward.
#[tokio::test]
async fn status_reports_the_app_master_did_while_the_vault_is_locked() {
    let (s, key_store) =
        Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
            .build_with_key_store();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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

/// A *second* supervisor, which has never adopted this instance,
/// imports the app master another supervisor already exported, and
/// its first `adopt` reports the imported DID rather than minting a
/// fresh one. Two independent fixture-built services sharing one
/// test-owned backup directory -- the stand-in for the file an
/// operator carries between two real supervisors during a handover.
#[tokio::test]
async fn a_second_supervisor_that_imports_the_app_master_adopts_without_minting_a_new_one() {
    let backup_dir = tempfile::tempdir().unwrap();
    let supervisor_a =
        Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }.build();
    let supervisor_b =
        Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }.build();

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

/// The mint-before-claim, record-after-claim asymmetry's failing
/// direction -- a claim that
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
    // ever stopped running before the claim -- the exact regression
    // this test exists to catch -- since both reads would then find
    // nothing rather than the same key.
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
    assert_eq!(minted_did, second_did, "a retried mint must resolve the same key, not a new one");
}

/// `adopt`'s un-retire and its app-master write now land in the same
/// `record_adopt` call, but nothing at the service level had exercised
/// them running back to back on a genuinely retired instance --
/// store-level coverage
/// (`store.rs`'s
/// `record_adopt_writes_generation_retired_and_app_master_did_together`)
/// proves the store method alone, not that `handle_adopt` actually
/// reaches it starting from `retired`.
#[tokio::test]
async fn adopt_on_a_retired_instance_un_retires_and_records_the_app_master_together() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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

// ── The durable outbox worker ─────────────────────────────────────

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
    /// in-flight when shutdown is asked for.
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
        let next = self.scripted.lock().unwrap().get_mut(&entry.did).and_then(VecDeque::pop_front);
        let result: anyhow::Result<Vec<BindingWriteOutcome>> = match next {
            Some(FakeDelivery::ConnectFails) => {
                return Err(anyhow::anyhow!("simulated connect failure"));
            }
            Some(FakeDelivery::Attempt(Ok(outcomes))) => Ok(outcomes),
            Some(FakeDelivery::Attempt(Err(msg))) => Err(anyhow::anyhow!(msg)),
            Some(FakeDelivery::CalleeError(msg)) => {
                Err(syneroym_rpc::JsonRpcError { code: -32010, message: msg, data: None }.into())
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
/// action synchronously.
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
        match self.write_bindings_outcome.lock().unwrap().take().expect("outcome not set for test")
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
    let payload = outbox::QueuedBindingWrite { substrate_did: substrate_did.to_string(), write };
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

/// A reachable substrate's `write_bindings` call must never touch the
/// real, wired-up
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

/// The worker's replay of an already-applied write must be a no-op
/// from the substrate's own
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

    assert!(s.store.queue.all().unwrap().is_empty(), "the item {id} must be gone from the outbox");
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

/// A `stale` delivery means a newer epoch already landed --
/// convergence, not loss -- so it must complete and must not
/// dead-letter.
#[tokio::test]
async fn a_queued_write_delivered_stale_completes_and_does_not_dead_letter() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Stale(7)])));
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

/// A queued item whose instance was retired between enqueue and
/// delivery must be quietly completed --
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

/// The `conflict` row -- the case the synchronous coverage misses.
/// Completes **and** raises the same `BindingConflict` alert the
/// synchronous path does.
#[tokio::test]
async fn a_queued_write_delivered_conflicting_raises_the_same_alert_as_the_synchronous_path() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector
        .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Conflict(9)])));
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

/// A callee error that does *not* name this write's own target as gone
/// -- a transient, reached-and-answered
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

/// A callee error on replay naming this write's own target as gone
/// (the exact wording `control_plane`'s `write-bindings` dispatch uses
/// for that specific refusal) is terminal -- straight to the DLQ, not
/// retried.
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

/// Without taking `instance_lock`, the worker could interleave with a
/// live pass write for the same instance and race
/// this supervisor into a spurious `BindingConflict`. Proven by
/// holding the lock externally (as a live pass would) and asserting
/// the worker's own attempt to take it blocks until released.
#[tokio::test]
async fn the_worker_and_a_loop_pass_never_write_one_instance_concurrently() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
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
    assert!(s.store.queue.all().unwrap().is_empty(), "the worker delivers once the lock frees up");
}

/// Cancelling the token must not wait for a delivery *genuinely in
/// flight* -- `FakeDelivery::Blocks` makes `connect` never resolve, so
/// if `shutdown` returned promptly here it is because cancellation
/// actually interrupted a real, ongoing delivery. An earlier version
/// of this test left the substrate unscripted, which fails `connect`
/// immediately and proves nothing about abandoning work in flight.
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

/// The in-process analogue of the e2e restart step -- a queued item
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
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    s.queue_connector = connector;

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty(), "the resumed item must have been delivered");
}

/// Recovery must complete within one `queue_tick_secs`, not one
/// `poll_interval_secs` -- driven against a paused clock with
/// `poll_interval_secs` set far above the worker tick, so passing by
/// accident (both loops running for real) is impossible.
#[tokio::test(start_paused = true)]
async fn recovery_completes_within_one_worker_tick_and_not_one_poll_interval() {
    let mut s =
        Fixture { queue_tick_secs: Some(5), poll_interval_secs: Some(3600), ..Fixture::default() }
            .build();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
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

// ── The DLQ surface ───────────────────────────────────────────────

/// `AlertStore`'s unique index cannot express one row per dead letter,
/// and an operator wants the standing fact anyway --
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

/// The clear path -- the same one `RemediationExhausted` already
/// documents. Replaying every dead letter for a key clears its alert;
/// an earlier replay leaving one behind must not.
#[tokio::test]
async fn the_alert_clears_when_the_last_dead_letter_for_that_key_is_gone() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
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
    // replay` refuses a second pending row for one key, so this drives
    // the worker to deliver (and complete) the first replay's item
    // before trying the second.
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

/// A *pruned* dead letter (the DLQ cap evicting the oldest row on
/// write) must clear its own standing alert exactly the same way an
/// explicit `replay` does -- not just when a human happens to replay
/// it. Distinct from the test above, which only exercises the replay
/// path.
#[tokio::test]
async fn a_pruned_dead_letter_clears_its_own_alert_too() {
    let mut s = service();
    // A cap of 1 so the second key's own dead letter immediately
    // evicts the first key's -- both keys share one group (the app
    // instance), so the cap applies across them.
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

/// The outbox growth bound -- "at most one row per `(instance,
/// logical_ref, substrate)`" -- is enforced by
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
            .enqueue(&key.to_string(), "did:key:zB", &BindingWrite { generation, ..write.clone() })
            .await;
    }

    assert_eq!(
        queue.all().unwrap().len(),
        1,
        "fifty enqueue attempts for one key must still leave exactly one outbox row"
    );
}

/// `dead-letters` lists exactly what the store holds, mapped onto the
/// WIT shape (logical ref and substrate DID pulled back out
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

/// `replay` re-enqueues through the ordinary worker path and does not
/// execute inline -- proven by scripting no delivery
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

/// Over the RPC surface -- a replayed item that fails again returns to
/// the DLQ with its attempt history intact, listable the same way the
/// first one was.
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (NOW - 3600) as i64, ..Default::default() },
    );
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
    assert_eq!(decisions.len(), 1);
    match &decisions[0] {
        ScheduleDecision::Run { logical_ref, member_index, service_id, substrate_did, .. } => {
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);

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
    let second = SupervisorService::schedule_decisions(&plan, &states, &report, minute(0) + 10, 60);
    assert_eq!(
        second,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
        "a second look inside the same cron minute must not run again"
    );
}

#[test]
fn a_tick_missed_while_the_supervisor_was_down_is_skipped_not_run_late() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
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
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
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
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
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
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
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
        scheduled_health("worker", 1, "did:key:zEdgeB", Signal::ProbeFailing("down".to_string())),
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
    let actions =
        s.store.journal.get_completed_actions_for_instance(&AppInstanceId::new("inst-1")).unwrap();
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
        "the run must time out at the 30s ceiling, not the configured 100s or the actor's own 60s \
         delay: {elapsed:?}"
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

fn plan_json_n_member_service(instance: &str, service_name: &str, mode: &str, n: u32) -> String {
    plan_json_n_member_service_with_vis(instance, service_name, mode, n, "private", "restricted")
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

/// The property the gateway's own check depends on -- the signed
/// document never echoes the hash back, always the real name, even
/// when the caller supplied the hash. Also pins that the epoch is
/// still carried and preserved on a hashed request.
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

/// An ungranted caller resolving an `open` service receives the signed
/// document.
#[tokio::test]
async fn resolve_open_service_answers_ungranted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
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

/// An ungranted caller naming a non-existent service gets the same
/// refusal as an unauthorized call.
#[tokio::test]
async fn resolve_open_service_nonexistent_service_refuses_identically_for_ungranted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
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

/// An ungranted caller resolving an `open` service on a retired
/// instance is refused.
#[tokio::test]
async fn resolve_open_service_on_retired_instance_is_refused() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
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

/// A granted caller naming a non-existent service gets InvalidParams,
/// not denied.
#[tokio::test]
async fn resolve_granted_caller_naming_nonexistent_service_returns_invalid_params() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
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

/// The document served to an ungranted `open` caller is byte-identical
/// to the one served to a granted caller.
#[tokio::test]
async fn resolve_open_service_served_to_ungranted_caller_is_identical_to_granted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
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

/// `topology_visibility = open` and `visibility = private` are not
/// redundant, and neither alone proves the other is unnecessary. The
/// compiler refuses this combination at both of its entry points
/// (`compile()` and `handle_submit`) precisely because, left to reach
/// a supervisor, `open` alone is sufficient for `handle_resolve` to
/// hand out the document -- the visibility read at
/// [`SupervisorService::handle_resolve`]'s top does not consult
/// `config.visibility` at all, only `topology_visibility`. This test
/// goes around both refusal points the way `adopted_instance` always
/// does (`s.store.submit` directly, not the RPC `submit` that calls
/// `handle_submit`) to prove that half of the claim as an actual
/// runtime behaviour, not just an absence of a compile-time refusal.
///
/// The other half -- that the member named in this document is then
/// unreachable -- is proven separately, structurally:
/// `member_registry_record_mints_nothing_for_a_private_member`
/// (`crates/sdk/src/deploy.rs`) shows a `private` member gets no
/// registry record at all, so nothing a caller could look up ever
/// exists to dial. Reproducing that failure here as well would need a
/// live registry and a live gateway dial, which is
/// `gateway_hostname_e2e.rs`'s job for the `(open, internal)` pair.
/// The two halves come from two different mechanisms.
#[tokio::test]
async fn resolve_open_service_over_a_private_member_still_serves_the_document() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "private", "open");
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
    assert!(signed.verify(&AppDid::new(app_did)).is_ok());
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

/// A cached document is re-signed once less than half its validity
/// remains, so a served copy always outlives a caller's own
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
async fn a_handover_to_a_different_app_did_does_not_serve_the_previous_masters_cached_document() {
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

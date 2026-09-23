use std::{fs, sync::Arc};

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::ServiceConfig;

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

#[tokio::test]
async fn a_deploy_carrying_an_app_context_registers_a_resolvable_binding() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
    );
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(ctx),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap();

    assert_eq!(
        service.registry.app_context_of("frontend-svc"),
        Some(("app-1".to_string(), "frontend".to_string()))
    );
    let resolved = service
        .logical_resolver
        .resolve(
            &TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            None,
        )
        .unwrap();
    assert_eq!(resolved.to_string(), "did:key:zBackendMember");
}

#[tokio::test]
async fn a_redeploy_that_drops_a_dependency_leaves_no_stale_persisted_row() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("test-caller");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();
    assert_eq!(
        service.registry.all_bindings().await.unwrap().len(),
        1,
        "the first deploy must have written exactly one binding row"
    );

    // A redeploy whose manifest no longer declares any dependency.
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &caller,
        )
        .await
        .unwrap();

    assert!(
        service.registry.all_bindings().await.unwrap().is_empty(),
        "a redeploy that drops a dependency must not leave its persisted row behind"
    );
}

/// Extended explicitly to redeploy: a `TopologyEntry` is an app-scoped
/// fact, not a per-dependent one, so dropping the only dependent that
/// declared it must not evict the in-memory `StaticInventory` entry
/// other dependents in the same app instance might still rely on. This
/// asserts "keep it", matching `undeploy`'s existing behavior and the
/// same reasoning restated at its call site.
#[tokio::test]
async fn a_redeploy_that_drops_a_dependency_still_resolves_it_in_memory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("test-caller");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    // A redeploy whose manifest no longer declares any dependency.
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &caller,
        )
        .await
        .unwrap();

    let resolved = service
        .logical_resolver
        .resolve(
            &TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            None,
        )
        .unwrap();
    assert_eq!(
        resolved.to_string(),
        "did:key:zBackendMember",
        "the persisted row is gone (asserted above), but the in-memory StaticInventory entry a \
         different dependent in the same app instance might still rely on must survive until \
         restart"
    );
}

/// ADR-0021 §2: a deploy may only bind
/// dependencies for its own declared app instance. Without this check,
/// a `DependencyBinding.app_instance_id` that disagrees with its own
/// `AppContext.app_instance_id` would silently write into a different
/// app instance's resolution table.
#[tokio::test]
async fn a_binding_naming_a_different_app_instance_than_its_own_context_fails_the_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let mismatched_binding = DependencyBinding {
        dependency_name: "backend".to_string(),
        app_instance_id: "app-2".to_string(),
        mode: WitTopologyMode::Singleton,
        members: vec!["did:key:zBackendMember".to_string()],
        epoch: 0,
        cache_ttl_ms: 60_000,
    };
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![mismatched_binding])),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("app-1") && err.contains("app-2"), "{err}");

    // Nothing must have been written -- the rejection is at validation
    // time, before any registry write.
    assert!(service.registry.all_bindings().await.unwrap().is_empty());
}

/// An app instance's first successful deploy
/// becomes its owner (first-write-wins, the same shape `service_id`
/// ownership already uses). Without it, any caller authorized to
/// deploy *some* service could name a different, already-claimed app
/// instance in its own `app_context` and overwrite the binding that
/// instance's other, unrelated services resolve -- reachable even
/// though every `binding.app_instance_id` here correctly matches its
/// own `app_context.app_instance_id` (the same-instance check alone does
/// not close this: it only forces the attacker to also lie about which
/// app instance its own service belongs to).
/// `open_service_db` and `native_dispatch` both
/// key on a bare `service_id` with no reservation of their own, so
/// before this check a deploy under the node's own DID overwrote
/// `ControlPlaneService`'s own dispatch entry (full node takeover), and
/// a deploy under `"supervisor"` opened the supervisor's vault and
/// overwrote its dispatch entry. Each caller below holds a capability
/// scoped exactly to the `service_id` it targets, so the rejection is
/// the reserved-name check firing, not the ordinary authorization gate.
#[tokio::test]
async fn a_deploy_cannot_claim_the_nodes_own_did_or_the_supervisor_dispatch_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let err = service
        .deploy(
            "did:key:zTestNode".to_string(),
            inline_manifest(None, None, None),
            &scoped_deploy_caller("did:key:zMallory", "did:key:zTestNode"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("reserved"), "{err}");

    let err = service
        .deploy(
            SUPERVISOR_RESERVED_SERVICE_ID.to_string(),
            inline_manifest(None, None, None),
            &scoped_deploy_caller("did:key:zMallory", SUPERVISOR_RESERVED_SERVICE_ID),
        )
        .await
        .unwrap_err();
    assert!(err.contains("reserved"), "{err}");
}

#[tokio::test]
async fn a_deploy_cannot_claim_an_app_instance_owned_by_a_different_caller() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &scoped_deploy_caller("did:key:zAlice", "frontend-svc"),
        )
        .await
        .unwrap();

    let attacker_binding = DependencyBinding {
        dependency_name: "backend".to_string(),
        app_instance_id: "app-1".to_string(),
        mode: WitTopologyMode::Singleton,
        members: vec!["did:key:zAttackerMember".to_string()],
        epoch: 0,
        cache_ttl_ms: 60_000,
    };
    let err = service
        .deploy_with_context(
            "evil-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "evil", vec![attacker_binding])),
            &scoped_deploy_caller("did:key:zBob", "evil-svc"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("app-1") && err.contains("owned by"), "{err}");

    let resolved = service
        .logical_resolver
        .resolve(
            &TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            None,
        )
        .unwrap();
    assert_eq!(
        resolved.to_string(),
        "did:key:zBackendMember",
        "the rejected deploy must not have overwritten alice's binding"
    );
}

/// Positive-path counterpart: the app instance's own owner may go on
/// deploying further services into it, and a second service sharing an
/// app instance with the first is the ordinary multi-service-per-app
/// shape A2 exists for -- the ownership check above must not lock out
/// the caller who legitimately owns the app instance.
#[tokio::test]
async fn an_app_instance_owner_may_deploy_a_second_service_into_it() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice_frontend = scoped_deploy_caller("did:key:zAlice", "frontend-svc");
    let alice_worker = scoped_deploy_caller("did:key:zAlice", "worker-svc");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &alice_frontend,
        )
        .await
        .unwrap();

    let result = service
        .deploy_with_context(
            "worker-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "worker",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &alice_worker,
        )
        .await;
    assert!(result.is_ok(), "the app instance's own owner must be able to join it: {result:?}");
}

/// A write presenting a generation
/// below the held one is rejected, and the error names the held
/// generation -- the text a supervisor parses to know it has been
/// superseded (ADR-0021 §4).
#[tokio::test]
async fn a_lower_generation_write_is_rejected_and_the_error_names_the_held_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
            &alice,
        )
        .await
        .unwrap();

    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await
        .unwrap_err();
    assert!(err.contains("at generation 5"), "{err}");
    assert!(err.contains("ADR-0021"), "{err}");
}

/// Matrix row 8: two writers both authorized on this substrate (both
/// node-wide here, so the pre-existing app-instance ownership check
/// does not itself reject the second one) presenting the *same*
/// generation is a two-writer conflict, not a tie.
#[tokio::test]
async fn a_second_writer_at_the_same_generation_is_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");
    let bob = node_wide_caller("did:key:zBob");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 3, ..app_context("app-1", "frontend", vec![]) }),
            &alice,
        )
        .await
        .unwrap();

    let err = service
        .deploy_with_context(
            "worker-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 3, ..app_context("app-1", "worker", vec![]) }),
            &bob,
        )
        .await
        .unwrap_err();
    assert!(err.contains("second writer"), "{err}");
}

/// Regression guard for the bug that would have locked a
/// supervisor out of its own app on its first post-adopt reconcile.
/// The same caller, presenting the *same* generation it already holds
/// (not 0), must keep succeeding -- this is the supervisor's steady
/// state.
#[tokio::test]
async fn the_recorded_supervisor_may_write_repeatedly_at_its_own_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor = node_wide_caller("did:key:zSupervisor");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 7, ..app_context("app-1", "frontend", vec![]) }),
            &supervisor,
        )
        .await
        .unwrap();

    let result = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 7, ..app_context("app-1", "frontend", vec![]) }),
            &supervisor,
        )
        .await;
    assert!(
        result.is_ok(),
        "the recorded supervisor must be able to redeploy at its own generation: {result:?}"
    );
}

/// The A0-A4 compatibility property: an app instance nobody has ever
/// `adopt`ed keeps accepting its authorized writer's ordinary,
/// unmanaged (`generation: 0`) deploys, unaffected by the new gate.
#[tokio::test]
async fn an_unadopted_app_instance_accepts_any_authorized_writer() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await
        .unwrap();

    // No `adopt`/`claim` ever ran -- every deploy still presents
    // generation 0, the A0-A4 convention, and must keep succeeding.
    let result = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await;
    assert!(
        result.is_ok(),
        "an unadopted instance must keep accepting its authorized writer: {result:?}"
    );
}

/// The same property, but with a genuinely *different* authorized
/// writer than the instance's first deploy -- the case
/// `an_unadopted_app_instance_accepts_any_authorized_writer` names but
/// does not actually exercise, since it reuses the same caller twice.
/// A node-wide caller is what `deploy_with_context`'s own
/// app-instance-owner check requires to deploy over a different
/// owner's instance; without generation 0 staying unmanaged on the
/// instance's *first* write, this second, node-wide-authorized deploy
/// would be rejected as "a second writer at the same generation",
/// defeating that owner-check bypass entirely.
#[tokio::test]
async fn a_second_different_authorized_writer_may_also_deploy_into_an_unadopted_instance() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");
    let bob = node_wide_caller("did:key:zBob");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await
        .unwrap();

    let result = service
        .deploy_with_context(
            "backend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "backend", vec![])),
            &bob,
        )
        .await;
    assert!(
        result.is_ok(),
        "a different node-wide-authorized caller must also be able to deploy into an unadopted \
         instance: {result:?}"
    );
}

/// Releasing an app instance clears its management stamp
/// (`supervisor_did`/`generation`, not `owner_did` -- release restores
/// manual operation, it does not transfer ownership), so a plain
/// operator deploy (presenting generation 0, since nothing manages the
/// instance any more) can touch it again. Without this an
/// adopted-then-released instance would be locked out forever. Uses a
/// node-wide caller for the post-release deploy since `owner_did`
/// still names the supervisor, not this operator -- the same bypass
/// the pre-existing ownership check already grants a substrate owner.
#[tokio::test]
async fn releasing_an_app_instance_lets_a_plain_deploy_touch_it_again() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor = node_wide_caller("did:key:zSupervisor");

    service.claim_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();
    service.release_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();

    let operator = node_wide_caller("did:key:zOperator");
    let result = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &operator,
        )
        .await;
    assert!(result.is_ok(), "a released instance must accept a plain deploy again: {result:?}");
}

/// `check_generation`'s `None` arm exists so `deploy`/`claim` can
/// create a row on an app instance's first touch -- right for them,
/// wrong for `release`. Releasing an instance nobody has ever deployed
/// must be a no-op, not mint an `owner_did` row that blocks a later
/// legitimate deploy from a different caller and can never be reclaimed
/// (no service ever names an instance nobody deployed, so `undeploy`'s
/// cleanup can never reach it).
#[tokio::test]
async fn releasing_an_unknown_app_instance_is_a_no_op() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let attacker = node_wide_caller("did:key:zAttacker");

    service.release_app_instance("app-never-deployed".to_string(), 0, &attacker).await.unwrap();

    assert!(
        service.registry.app_instance_management_of("app-never-deployed").is_none(),
        "releasing an app instance with no row must not create one"
    );
}

/// The other half of the backlog row `release-app-instance` was built
/// to close: undeploying the last service naming an app instance must
/// forget its management row, or the instance id can never be
/// reclaimed by another caller.
#[tokio::test]
async fn undeploying_the_last_service_of_an_instance_forgets_its_management_row() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &caller,
        )
        .await
        .unwrap();
    assert!(service.registry.app_instance_management_of("app-1").is_some());

    service.undeploy("frontend-svc".to_string(), 0, &caller).await.unwrap();

    assert!(
        service.registry.app_instance_management_of("app-1").is_none(),
        "the last service's undeploy must forget the app instance's management row"
    );
}

/// `adopt`'s read half must report the held generation to the
/// instance's own owner -- otherwise a supervisor cannot compute
/// `held + 1`.
#[tokio::test]
async fn app_instance_management_of_reports_the_held_generation_to_the_owner() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 4, ..app_context("app-1", "frontend", vec![]) }),
            &alice,
        )
        .await
        .unwrap();

    let management = service
        .app_instance_management_of("app-1".to_string(), &alice)
        .await
        .unwrap()
        .expect("the owner must see its own instance's management stamp");
    assert_eq!(management.generation, 4);
    assert_eq!(management.owner_did, "did:key:zAlice");
    assert_eq!(management.supervisor_did.as_deref(), Some("did:key:zAlice"));
}

/// A caller with no visibility
/// into the instance gets `Ok(None)`, indistinguishable from "never
/// deployed here", not an error -- so it cannot be used to probe for
/// the instance's existence.
#[tokio::test]
async fn app_instance_management_of_returns_none_to_a_caller_with_no_grant() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await
        .unwrap();

    let mallory = scoped_deploy_caller("did:key:zMallory", "some-other-svc");
    let result = service.app_instance_management_of("app-1".to_string(), &mallory).await.unwrap();
    assert!(
        result.is_none(),
        "a caller with no visibility into the instance must not learn it exists, not even as an \
         error"
    );
}

/// The property that makes `adopt` durable at the moment of the
/// claim, not on whatever write happens next -- a bare claim, with no
/// deploy at all, must be readable back and must not have installed
/// anything else.
#[tokio::test]
async fn claim_app_instance_records_the_generation_without_any_other_write() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor = node_wide_caller("did:key:zSupervisor");

    service.claim_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();

    let management = service
        .app_instance_management_of("app-1".to_string(), &supervisor)
        .await
        .unwrap()
        .expect("the claim must have created a management row");
    assert_eq!(management.generation, 1);
    assert_eq!(management.supervisor_did.as_deref(), Some("did:key:zSupervisor"));
    assert!(
        service.registry.app_context_of_any("app-1").is_none(),
        "a bare claim must not install a service or an app context"
    );
}

/// Two supervisors racing an `adopt` must lose deterministically at
/// the substrate, at the moment of the claim -- not discover it only
/// once one of them happens to issue a deploy.
#[tokio::test]
async fn a_second_claim_at_the_same_generation_is_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor_a = node_wide_caller("did:key:zSupervisorA");
    let supervisor_b = node_wide_caller("did:key:zSupervisorB");

    service.claim_app_instance("app-1".to_string(), 1, &supervisor_a).await.unwrap();

    let err = service.claim_app_instance("app-1".to_string(), 1, &supervisor_b).await.unwrap_err();
    assert!(err.contains("second writer"), "{err}");
}

/// `claim`/`release` are node-scoped acts (an app instance
/// spans services), so an app-scoped `orchestrator/deploy` grant --
/// enough to deploy one service -- must not be enough for either.
#[tokio::test]
async fn claim_and_release_are_rejected_without_node_wide_orchestrator_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let scoped = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

    let claim_err = service.claim_app_instance("app-1".to_string(), 1, &scoped).await.unwrap_err();
    assert!(claim_err.contains("node-wide"), "{claim_err}");

    let release_err =
        service.release_app_instance("app-1".to_string(), 1, &scoped).await.unwrap_err();
    assert!(release_err.contains("node-wide"), "{release_err}");
}

/// Generation 0 means unmanaged, so a claim presenting it cannot mean
/// "claim supervision" -- without this refusal it would silently
/// record no supervisor at all and still report success, and on a
/// fresh instance still create the owner row, making the no-op harder
/// to notice.
#[tokio::test]
async fn claiming_an_app_instance_at_generation_0_is_refused() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor = node_wide_caller("did:key:zSupervisor");

    let err = service.claim_app_instance("app-1".to_string(), 0, &supervisor).await.unwrap_err();
    assert!(err.contains("generation 0"), "{err}");
    assert!(
        service.registry.app_instance_management_of("app-1").is_none(),
        "a refused claim must not create a management row"
    );
}

#[tokio::test]
async fn write_bindings_is_rejected_without_an_orchestrator_deploy_grant() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    let no_grant = CallerContext::service_system("nobody");
    let err = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                generation: 0,
            },
            &no_grant,
        )
        .await
        .unwrap_err();
    assert!(err.contains("orchestrator/deploy"), "{err}");
}

/// A grant scoped to one service of an app instance is not the same as
/// authority over the instance as a whole: `bob` holds
/// `orchestrator/deploy` on `worker-svc` specifically -- enough to pass
/// the capability check and the app-context match, since `worker-svc`
/// genuinely belongs to `app-1` -- but `app-1` is `alice`'s, and `bob`
/// holds no node-wide authority either. `deploy_with_context` refuses
/// exactly this shape of caller for the same app instance; `write-
/// bindings` must too, since a push lands in the shared resolver entry
/// every service of the instance resolves through, not only `bob`'s
/// own.
#[tokio::test]
async fn write_bindings_is_rejected_for_a_non_owner_with_only_a_service_scoped_grant() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "frontend", vec![])),
            &alice,
        )
        .await
        .unwrap();
    service
        .deploy_with_context(
            "worker-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "worker",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &alice,
        )
        .await
        .unwrap();

    let bob = scoped_deploy_caller("did:key:zBob", "worker-svc");
    let err = service
        .write_bindings(
            BindingWrite {
                service_id: "worker-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                generation: 0,
            },
            &bob,
        )
        .await
        .unwrap_err();
    assert!(err.contains("app-1") && err.contains("zAlice"), "{err}");
}

#[tokio::test]
async fn write_bindings_refuses_a_service_whose_app_context_names_another_instance() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    let err = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-2".to_string(),
                bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("app-1") && err.contains("app-2"), "{err}");
}

/// A push may only update a dependency the service already declared
/// at deploy -- a new logical name changes the guest's contract and
/// needs a redeploy, not a push.
#[tokio::test]
async fn write_bindings_refuses_a_dependency_the_service_never_declared() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    let err = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("cache", vec!["did:key:zCacheMember"])],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("cache") && err.contains("redeploy"), "{err}");
}

/// The accepting generation is persisted before any binding is
/// examined, not after the whole call succeeds -- so a write that is
/// later refused (here, an undeclared dependency) still leaves the
/// substrate remembering who was authorized to write at that
/// generation, the same property the deploy path proves.
#[tokio::test]
async fn a_refused_write_still_persists_the_accepting_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let supervisor = node_wide_caller("did:key:zSupervisor");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext {
                generation: 1,
                ..app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )
            }),
            &supervisor,
        )
        .await
        .unwrap();

    let result = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("cache", vec!["did:key:zCacheMember"])],
                generation: 2,
            },
            &supervisor,
        )
        .await;
    assert!(result.is_err(), "the undeclared dependency must still be refused");

    let management = service.registry.app_instance_management_of("app-1").unwrap();
    assert_eq!(
        management.generation, 2,
        "the accepting generation must be persisted even though the write itself failed"
    );
}

/// The whole binding list is validated before any of it is applied: a
/// refusal partway through (here, the second binding names an
/// undeclared dependency) must leave every earlier binding in the same
/// call untouched, not partially applied with no way for the caller to
/// know which ones landed.
#[tokio::test]
async fn a_refused_binding_leaves_no_earlier_binding_in_the_same_call_applied() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    let err = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![
                    DependencyBinding {
                        epoch: 1,
                        members: vec!["did:key:zNewMember".to_string()],
                        ..dependency_binding("backend", vec!["did:key:zNewMember"])
                    },
                    dependency_binding("cache", vec!["did:key:zCacheMember"]),
                ],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("cache") && err.contains("redeploy"), "{err}");

    let backend = service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
    assert!(
        backend.contains("zBackendMember") && !backend.contains("zNewMember"),
        "the earlier, individually-valid binding must not have been applied: {backend}"
    );
}

/// Matrix row 6: an ordinary retry -- the same epoch, the same content
/// -- is a success that writes nothing.
#[tokio::test]
async fn a_binding_write_at_the_current_epoch_with_identical_content_writes_nothing() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    let outcomes = service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(outcomes[0], BindingWriteOutcomeWire::NoOp),
        "expected NoOp, got a differently-shaped outcome"
    );
}

/// The property reference-scenario step 5 turns on: a binding push
/// must never go through the deploy path. Uses the config-generation
/// counter `deploy_with_context` always bumps as the proxy, since
/// nothing else in this test harness tracks sandbox-engine calls.
#[tokio::test]
async fn a_binding_write_does_not_restart_the_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();
    let generation_before =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();

    service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![dependency_binding("backend", vec!["did:key:zNewBackendMember"])],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap();

    let generation_after =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
    assert_eq!(
        generation_before, generation_after,
        "a binding push must not go through the deploy path"
    );
}

/// The epoch guard and the convergence read both classify
/// against the **persisted per-dependent row**, not the shared
/// resolver entry -- a push targeted at one dependent must not affect
/// what a different dependent of the same instance has recorded.
#[tokio::test]
async fn two_dependents_of_one_instance_report_their_own_binding_epochs() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();
    service
        .deploy_with_context(
            "worker-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "worker",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![DependencyBinding {
                    dependency_name: "backend".to_string(),
                    app_instance_id: "app-1".to_string(),
                    mode: WitTopologyMode::Singleton,
                    members: vec!["did:key:zNewBackendMember".to_string()],
                    epoch: 1,
                    cache_ttl_ms: 60_000,
                }],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap();

    let frontend_entry: TopologyEntry = serde_json::from_str(
        &service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap(),
    )
    .unwrap();
    let worker_entry: TopologyEntry = serde_json::from_str(
        &service.registry.binding_of("worker-svc", "backend").await.unwrap().unwrap(),
    )
    .unwrap();
    assert_eq!(
        frontend_entry.epoch,
        TopologyEpoch(1),
        "frontend must report the epoch pushed to it"
    );
    assert_eq!(
        worker_entry.epoch,
        TopologyEpoch(0),
        "worker's own persisted row must be unaffected by a push targeted at frontend -- the \
         epoch guard classifies against the per-dependent row, not the shared resolver entry"
    );
}

/// A retry after a lost response -- the same manifest,
/// the same app context minus generation, against a still-running
/// service -- is a no-op. Regression guard: the management
/// stamp must still advance to the new generation even though the
/// deploy itself is deduplicated, because it is persisted at the
/// generation gate, before the dedup check ever runs.
#[tokio::test]
async fn an_identical_redeploy_of_a_running_service_is_a_no_op() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let manifest = inline_manifest(None, None, None);
    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
    );

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            manifest.clone(),
            Some(AppContext { generation: 1, ..ctx.clone() }),
            &caller,
        )
        .await
        .unwrap();
    let gen_before =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();

    // A later write at a higher generation (the supervisor's own
    // reconcile after an `adopt`) with an identical manifest and
    // context.
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            manifest,
            Some(AppContext { generation: 2, ..ctx }),
            &caller,
        )
        .await
        .unwrap();
    let gen_after =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
    assert_eq!(gen_before, gen_after, "an identical redeploy of a running service must be a no-op");

    let management = service
        .registry
        .app_instance_management_of("app-1")
        .expect("the generation gate's persist must not be skipped by the dedup no-op");
    assert_eq!(
        management.generation, 2,
        "the redeploy's generation must be recorded even though the deploy itself was a no-op -- \
         without this assertion this test passes against the bug §0.27 fixes"
    );
}

#[tokio::test]
async fn a_binding_naming_a_non_did_key_member_fails_the_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let ctx =
        app_context("app-1", "frontend", vec![dependency_binding("backend", vec!["not-a-did"])]);
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(ctx),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("invalid member DID"), "{err}");
}

#[tokio::test]
async fn undeploy_clears_the_persisted_binding_rows() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("test-caller");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    service.undeploy("frontend-svc".to_string(), 0, &caller).await.unwrap();

    assert_eq!(service.registry.app_context_of("frontend-svc"), None);
    assert!(service.registry.all_bindings().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_dependency_name_containing_a_slash_fails_the_deploy_rather_than_panicking() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("bad/name", vec!["did:key:zBackendMember"])],
    );
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(ctx),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("invalid dependency name"), "{err}");
}

#[tokio::test]
async fn an_empty_dependency_name_fails_the_deploy_rather_than_panicking() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("", vec!["did:key:zBackendMember"])],
    );
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(ctx),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("invalid dependency name"), "{err}");
}

#[tokio::test]
async fn an_empty_app_instance_id_in_the_app_context_fails_the_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let ctx = app_context("", "frontend", vec![]);
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(ctx),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();
    assert!(err.contains("invalid app instance id"), "{err}");
}

#[tokio::test]
async fn test_deploy_fdae_policy_validates_persists_and_is_loadable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    // A policy with no `custom_config` on the manifest -- the regression
    // test for the FDAE block's placement outside the `custom_config`
    // block (unlike `schema`, which is only read inside it).
    let policy_filename = format!("test_fdae_policy_{}.json", std::process::id());
    fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };

    let result = service
        .deploy("fdae_test_service".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;

    let _ = fs::remove_file(&policy_filename);

    assert!(result.is_ok(), "{result:?}");
    let loaded = storage_provider.load_fdae_policy("fdae_test_service").await.unwrap();
    assert_eq!(loaded, Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()));
}

use std::sync::Arc;

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

/// `deploy` records `caller.caller_did` as
/// the owner -- the same DID `build_caller` resolves to the
/// `DelegationCertificate`'s `master_did`, never the ephemeral
/// `temporary_did`. `crates/router/src/route_handler/io.rs`'s
/// `build_caller_uses_master_did_not_temporary_did_as_caller_did`
/// proves that resolution; this test covers what
/// `ControlPlaneService` does with whatever `caller_did` it is handed.
#[tokio::test]
async fn deploy_records_owner_as_caller_did() {
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
        registry.clone(),
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        native_dispatch,
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let caller = node_wide_caller("did:key:zOwnerDid");
    let service_id = "owner-attribution-svc".to_string();
    service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();

    assert_eq!(registry.owner_of(&service_id), Some(caller.caller_did.clone()));

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    assert_eq!(registry.owner_of(&service_id), None);
}

#[tokio::test]
async fn the_derived_instance_identity_is_stable_across_calls() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
    let caller = status_capable_caller("did:key:zOwner");

    let first = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();
    let second = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();

    assert_eq!(first.instance_did, second.instance_did);
    assert_eq!(first.pubkey_hex, second.pubkey_hex);
}

#[tokio::test]
async fn two_owners_get_different_instance_identities_for_the_same_service_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;

    let alice = status_capable_caller("did:key:zAlice");
    let bob = status_capable_caller("did:key:zBob");

    let for_alice = service.instance_identity("shared-svc".to_string(), &alice).await.unwrap();
    let for_bob = service.instance_identity("shared-svc".to_string(), &bob).await.unwrap();

    assert_ne!(for_alice.instance_did, for_bob.instance_did);
}

/// Before anything is installed, there is no ground
/// truth to report -- `installed_temporary_did` is `None`, and
/// `instance_did` alone is what a caller about to certify a service
/// for the first time reads.
#[tokio::test]
async fn instance_identity_reports_no_installed_did_before_anything_is_deployed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
    let caller = status_capable_caller("did:key:zOwner");

    let identity = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();

    assert_eq!(identity.installed_temporary_did, None);
}

/// The whole reason `installed_temporary_did` exists.
/// Once a certificate is installed under one caller, a *different*
/// caller's `instance_identity` still derives its own (different)
/// prospective DID in `instance_did` -- unchanged, since `deploy`'s
/// certify flow depends on that -- but `installed_temporary_did` now
/// reports the certificate actually in force, which is alice's, not
/// bob's, regardless of who is asking.
#[tokio::test]
async fn instance_identity_reports_the_installed_did_even_for_a_different_caller() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let alice = node_wide_caller("did:key:zAlice");
    // `instance_identity` gates on `orchestrator/status`, not `deploy`
    // (the abilities are flat and independent) -- bob needs the former here.
    let bob = status_capable_caller("did:key:zBob");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();
    let cert = instance_cert_for(&node_identity, &master, &alice.caller_did, &service_id, 3600);
    service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &alice).await.unwrap();
    let installed = registry.instance_cert(&service_id).unwrap().temporary_did;

    let for_bob = service.instance_identity(service_id.clone(), &bob).await.unwrap();

    assert_ne!(
        for_bob.instance_did, installed,
        "bob's own derived DID must not equal what alice actually installed"
    );
    assert_eq!(
        for_bob.installed_temporary_did,
        Some(installed),
        "installed_temporary_did must report the real key regardless of who is asking"
    );
}

#[tokio::test]
async fn a_deploy_without_a_certificate_still_succeeds_and_stores_none() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
    let caller = node_wide_caller("did:key:zOwner");
    let service_id = "no-cert-svc".to_string();

    service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();

    assert_eq!(registry.instance_cert(&service_id), None);
}

#[tokio::test]
async fn a_deploy_is_rejected_when_the_certificates_master_is_not_the_service_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) =
        service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");
    let service_id = "member-master-svc".to_string();

    let wrong_master = syneroym_identity::Identity::generate().unwrap();
    let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
    let cert = DelegationCertificate::issue(
        &wrong_master,
        derived.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cert.to_json().unwrap());

    let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
    assert!(err.contains("does not name this deploy's service_id"), "unexpected error: {err}");
    assert_eq!(registry.instance_cert(&service_id), None);
}

#[tokio::test]
async fn a_deploy_is_rejected_when_the_certificate_certifies_a_different_key() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
    let caller = node_wide_caller("did:key:zOwner");

    let member_master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&member_master.public_key());

    // Certifies some *other* key, not the one this substrate would derive
    // for (caller, service_id).
    let wrong_instance = syneroym_identity::Identity::generate().unwrap();
    let cert = DelegationCertificate::issue(
        &member_master,
        wrong_instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cert.to_json().unwrap());

    let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
    assert!(err.contains("not the key this substrate would derive"), "unexpected error: {err}");
    assert_eq!(registry.instance_cert(&service_id), None);
}

#[tokio::test]
async fn a_deploy_is_rejected_when_the_certificate_carries_the_routing_scope() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) =
        service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let member_master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&member_master.public_key());
    let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        derived.public_key(),
        3600,
        "routing".to_string(),
    )
    .unwrap();
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cert.to_json().unwrap());

    let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
    assert!(err.contains("scope"), "unexpected error: {err}");
    assert_eq!(registry.instance_cert(&service_id), None);
}

/// ADR-0022 §2's `generation` is a real content field, not freshness
/// churn like `not_after`/`pkarr_packet_hex` -- omitting it from the
/// dedup hash would let two records that differ only in generation
/// hash identically, silently defeating redeploy dedup the day a
/// publisher's generation is ever nonzero (every one is `0` today).
#[test]
fn stable_registry_certificate_for_hash_distinguishes_by_generation() {
    fn record_json(generation: u64) -> String {
        format!(
            r#"{{"info":{{"service_id":"did:key:zA","substrate_id":"did:key:zNode",
                "endpoint_type":"service","mechanisms":[],"is_private":false,
                "not_after":4102444800,"generation":{generation}}},
                "pkarr_packet_hex":"aa"}}"#
        )
    }
    let at_zero = stable_registry_certificate_for_hash(&record_json(0));
    let at_one = stable_registry_certificate_for_hash(&record_json(1));
    assert_ne!(at_zero, at_one, "two records differing only in generation must hash differently");
}

/// A0-02: `deploy-manifest.instance-certificate`'s WIT doc says "absent
/// leaves the service its own master" -- that must hold on a redeploy
/// that drops `--master`, not only on the first deploy of a service_id,
/// or the stale certificate keeps being presented on outbound guest
/// calls under a `temporary_did` the redeploy's new owner no longer
/// derives to.
#[tokio::test]
async fn a_redeploy_without_a_certificate_clears_a_previously_installed_one() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) =
        service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let member_master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&member_master.public_key());
    let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        derived.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cert.to_json().unwrap());
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
    assert!(registry.instance_cert(&service_id).is_some());

    service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();
    assert_eq!(
        registry.instance_cert(&service_id),
        None,
        "a redeploy without --master must clear the previously installed certificate"
    );
}

/// The certificate-only install path: the new certificate lands and the
/// service's *config* generation is untouched, which is the whole
/// reason `renew-cert` exists rather than a redeploy.
#[tokio::test]
async fn renew_cert_installs_a_new_certificate_without_touching_the_config_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let first = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(first.to_json().unwrap());
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

    let gen_before =
        service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();

    let renewed = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
    service.renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller).await.unwrap();

    assert_eq!(
        registry.instance_cert(&service_id).map(|c| c.expires_at_secs),
        Some(renewed.expires_at_secs),
        "the renewed certificate must be the one installed"
    );
    assert_eq!(
        service.storage_provider.get_latest_config_generation(&service_id).await.unwrap(),
        gen_before,
        "a renewal changes no configuration, so it must not bump the config generation"
    );
}

/// The correctness prerequisite: the running service's by-value copy of
/// the certificate must move with the installed one, or every
/// `RelationshipProof` it signs afterwards carries a certificate the
/// verifier will reject.
#[tokio::test]
async fn renew_cert_rebuilds_syn_svc_native_service_with_the_new_certificate() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let first = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(first.to_json().unwrap());
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

    let before = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
    assert_eq!(before.delegation.as_deref(), Some(first.to_json().unwrap().as_str()));

    let renewed = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
    assert_ne!(renewed.to_json().unwrap(), first.to_json().unwrap());
    service.renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller).await.unwrap();

    let after = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
    assert_eq!(
        after.delegation.as_deref(),
        Some(renewed.to_json().unwrap().as_str()),
        "a proof signed after the renewal must carry the *new* certificate, not the one the \
         native service was constructed with"
    );
    after.verify(&service_id).expect("the proof must verify against the renewed certificate");
}

/// `renew-cert` is a lifecycle write, so it inherits `restart`'s own
/// service-owner check rather than being reachable by any holder of a
/// service-scoped grant.
#[tokio::test]
async fn renew_cert_is_refused_for_a_service_owned_by_another_caller() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let alice = node_wide_caller("did:key:zAlice");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();

    let bob = scoped_deploy_caller("did:key:zBob", &service_id);
    let cert = instance_cert_for(&node_identity, &master, &bob.caller_did, &service_id, 3600);
    let err =
        service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &bob).await.unwrap_err();
    assert!(err.contains("owned by"), "{err}");
}

/// The boundary of the check above: a node-wide `orchestrator/deploy`
/// grantee -- the shape a supervisor holds -- renews a service it does
/// not own. The certificate must be minted for *that* caller, since the
/// derived instance key depends on the calling DID.
#[tokio::test]
async fn renew_cert_by_a_node_wide_deploy_grantee_ignores_the_service_owner() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let alice = node_wide_caller("did:key:zAlice");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();

    let bob = node_wide_caller("did:key:zBob");
    let cert = instance_cert_for(&node_identity, &master, &bob.caller_did, &service_id, 3600);
    service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &bob).await.unwrap();
    assert_eq!(
        registry.instance_cert(&service_id).map(|c| c.temporary_did),
        Some(cert.temporary_did)
    );
}

/// A superseded supervisor must not be able to install a certificate on
/// a service it no longer manages -- the same generation gate `restart`
/// and `undeploy` already apply.
#[tokio::test]
async fn renew_cert_respects_the_same_generation_gate_as_restart() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zAlice");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    service
        .deploy_with_context(
            service_id.clone(),
            owner_test_manifest(),
            Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
            &caller,
        )
        .await
        .unwrap();

    let cert = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    let err = service
        .renew_cert(service_id.clone(), 3, cert.to_json().unwrap(), &caller)
        .await
        .unwrap_err();
    assert!(err.contains("at generation 5"), "{err}");
}

/// The backstop against an unbounded mint: nothing else caps
/// `expires_at_secs`, so a certificate valid for years would sit there
/// unnoticed -- the near-expiry warning that would catch it never fires.
#[tokio::test]
async fn verify_installed_instance_cert_rejects_a_certificate_over_the_thirty_day_cap() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let too_long = instance_cert_for(
        &node_identity,
        &master,
        &caller.caller_did,
        &service_id,
        MAX_INSTANCE_CERT_LIFETIME_SECS + 3600,
    );
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(too_long.to_json().unwrap());

    let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
    assert!(err.contains("maximum"), "{err}");
    assert_eq!(registry.instance_cert(&service_id), None);
}

/// The regression guard for the cap above: the attended posture's own
/// CLI default (24 hours) is nowhere near it, and so is any reasonable
/// manual cadence. The cap catches an unbounded mistake, not a
/// deliberate long-lived certificate.
#[tokio::test]
async fn verify_installed_instance_cert_accepts_the_cli_default_twenty_four_hour_certificate() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    // `roymctl svc deploy --expires-hours`'s own default, restated here
    // rather than imported: `syneroym-sdk` depends on this crate, so
    // the constant cannot travel the other way.
    let cli_default_expires_hours = 24;
    let cli_default = instance_cert_for(
        &node_identity,
        &master,
        &caller.caller_did,
        &service_id,
        cli_default_expires_hours * 3600,
    );
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cli_default.to_json().unwrap());

    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
    assert!(registry.instance_cert(&service_id).is_some());
}

/// The `load_fdae_policy` `None` arm: a service that never declared a
/// policy renews cleanly and still has none afterwards.
#[tokio::test]
async fn renew_cert_leaves_fdae_policy_untouched_when_none_was_ever_saved() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, _dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();
    assert_eq!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap(), None);

    let cert = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &caller).await.unwrap();
    assert_eq!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap(), None);
}

/// Without this gate a capability-holding caller could hand
/// `renew-cert` a `service_id` nothing ever deployed and have it
/// register a live native-dispatch entry for it: `owner_of` passes
/// vacuously for an unknown id, and an absent FDAE policy is not an
/// error. `restart` already refuses on the same signal.
#[tokio::test]
async fn renew_cert_is_refused_for_a_service_id_with_no_recorded_deploy_facts() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let cert = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);

    let err = service
        .renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &caller)
        .await
        .unwrap_err();
    assert!(err.contains("not deployed here"), "{err}");
    assert!(
        dispatch.get(&service_id).is_none(),
        "a refused renewal must never have registered a dispatch entry"
    );
}

/// The renewed native service must mirror the *whole* of
/// `deploy_with_context`'s construction site, not an enumerated subset
/// of it: an implementation copying only the obvious inputs produces a
/// service whose FDAE policy, proxy, and row-authorizer hooks are dead.
///
/// Observed through `RENEWAL_POLICY`: a service holding it routes
/// `resolve-relation` on `members` into a real query against the
/// `members` table (which this fixture never creates, so the query
/// reports collection-not-found), while a service whose policy was
/// silently dropped short-circuits and answers with a signed, empty
/// proof. Err-versus-Ok is therefore exactly "does the rebuilt service
/// still hold its policy", and the certificate on the proof is the
/// renewal itself.
#[tokio::test]
async fn renew_cert_mirrors_the_deploy_call_sites_service_proxy_and_row_authorizer() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry, dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let first = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    let mut manifest =
        inline_manifest(None, None, Some(DocumentSource::Inline(RENEWAL_POLICY.to_string())));
    manifest.instance_certificate = Some(first.to_json().unwrap());
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
    assert!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap().is_some());

    let after_deploy =
        resolve_relation_through_dispatch(&dispatch, &service_id, "members", &caller).await;
    assert!(
        after_deploy.is_err(),
        "the deploy-built service must route this relation through its policy"
    );
    let deploy_asserter =
        relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await.asserter_did;

    let renewed = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
    service.renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller).await.unwrap();

    let after_renewal =
        resolve_relation_through_dispatch(&dispatch, &service_id, "members", &caller).await;
    assert!(
        after_renewal.is_err(),
        "the rebuilt service must still route this relation through its policy -- an \
         implementation that mirrored only part of the call site would answer it with a signed, \
         empty proof instead"
    );
    let renewed_proof = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
    assert_eq!(
        renewed_proof.asserter_did, deploy_asserter,
        "the rebuilt service must speak as the same member master"
    );
    assert_eq!(renewed_proof.delegation.as_deref(), Some(renewed.to_json().unwrap().as_str()));
}

/// A stored FDAE document that no longer parses must abort the renewal
/// before anything is installed. Falling back to `fdae_policy: None`
/// would silently drop row/column filtering for the renewed instance --
/// a materially worse failure than `deploy`'s, which fails the whole
/// call on the same bad document.
#[tokio::test]
async fn renew_cert_aborts_on_a_stored_fdae_policy_that_fails_to_reparse() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry, dispatch) =
        service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&master.public_key());
    let first = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
    let mut manifest =
        inline_manifest(None, None, Some(DocumentSource::Inline(RENEWAL_POLICY.to_string())));
    manifest.instance_certificate = Some(first.to_json().unwrap());
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

    // Stands in for a schema or parser change landing between the
    // deploy that saved this document and the renewal that re-reads it.
    service
        .storage_provider
        .save_fdae_policy(&service_id, "{ this is not a policy document }")
        .await
        .unwrap();

    let renewed = instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
    let err = service
        .renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller)
        .await
        .unwrap_err();
    assert!(err.contains("no longer validates"), "{err}");
    assert_eq!(
        registry.instance_cert(&service_id).map(|c| c.expires_at_secs),
        Some(first.expires_at_secs),
        "an aborted renewal must leave the previously installed certificate in place"
    );
    let still = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
    assert_eq!(still.delegation.as_deref(), Some(first.to_json().unwrap().as_str()));
}

#[tokio::test]
async fn undeploy_removes_the_instance_certificate_with_the_owner_row() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, registry) =
        service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let member_master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&member_master.public_key());
    let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        derived.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut manifest = owner_test_manifest();
    manifest.instance_certificate = Some(cert.to_json().unwrap());

    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
    assert!(registry.instance_cert(&service_id).is_some());

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    assert_eq!(registry.instance_cert(&service_id), None);
}

/// The whole point: nothing is staged on the substrate's filesystem, and
/// the deploy still validates against a schema that arrived in the call.
#[tokio::test]
async fn test_deploy_inline_schema_validates_without_a_staged_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let schema = DocumentSource::Inline(
        r#"{"type":"object","properties":{"port":{"type":"integer"}}}"#.to_string(),
    );
    let result = service
        .deploy(
            "inline_schema_ok".to_string(),
            inline_manifest(Some(r#"{"port": 8080}"#), Some(schema), None),
            &node_wide_caller("test-caller"),
        )
        .await;

    assert!(result.is_ok(), "{:?}", result.unwrap_err());
}

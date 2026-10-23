use std::time::{SystemTime, UNIX_EPOCH};

use syneroym_core::{
    dht_registry::{EndpointInfo, EndpointMailbox, EndpointType},
    mailbox::HostedMailbox,
};
use syneroym_identity::Identity;
use syneroym_mailbox_format::OPEN_MAX_TTL_SECS;
use syneroym_rpc::{NativeInvocation, RpcError};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::ContainerPortMapping;

use super::{super::*, helpers::*};

/// A service member for these tests: its master, the service id that master
/// names, and the node identity of the substrate that will host it.
struct Member {
    master: Identity,
    service_id: String,
    node_identity: Arc<Identity>,
}

impl Member {
    fn new() -> Self {
        let master = Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        Self { master, service_id, node_identity: Arc::new(Identity::generate().unwrap()) }
    }

    fn mailbox_key(&self) -> MailboxSecretKey {
        MailboxSecretKey::derive_from_master(&self.master)
    }

    fn mailbox_route(&self, key: &MailboxSecretKey) -> EndpointMailbox {
        EndpointMailbox {
            service_id: derive_did_key(&Identity::generate().unwrap().public_key()),
            key: key.public_key(),
        }
    }

    fn record(&self, mailbox: Option<EndpointMailbox>, is_private: bool) -> String {
        let not_after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 86_400;
        let signed = EndpointInfo {
            service_id: self.service_id.clone(),
            substrate_id: "did:key:zTestNode".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private,
            ttl: None,
            not_after,
            generation: 0,
            mailbox,
        }
        .sign(&self.master)
        .unwrap();
        serde_json::to_string(&signed).unwrap()
    }

    /// A public manifest whose record names `route` and whose credentials
    /// carry `key` (either may be `None`).
    fn manifest(
        &self,
        caller: &CallerContext,
        route: Option<EndpointMailbox>,
        key: Option<&MailboxSecretKey>,
    ) -> DeployManifest {
        let cert = instance_cert_for(
            &self.node_identity,
            &self.master,
            &caller.caller_did,
            &self.service_id,
            3600,
        );
        let mut manifest = owner_test_manifest();
        manifest.config.visibility = Some(WitVisibility::Public);
        manifest.registry_certificate = Some(self.record(route, false));
        manifest.instance_certificate = Some(cert.to_json().unwrap());
        manifest.mailbox = key.map(credentials_for);
        manifest
    }
}

fn caller() -> CallerContext {
    node_wide_caller("did:key:zOwner")
}

async fn host_of(member: &Member, dir: &std::path::Path) -> ControlPlaneService {
    service_with_node_identity(dir, member.node_identity.clone()).await.0
}

fn credentials_for(key: &MailboxSecretKey) -> MailboxCredentials {
    MailboxCredentials { secret_key: key.to_z32().to_string(), methods: Vec::new() }
}

// -- validate_mailbox ------------------------------------------------------

#[test]
fn no_mailbox_in_the_record_or_the_manifest_is_fine() {
    let member = Member::new();
    let record = member.record(None, false);
    let result =
        validate_mailbox(&member.service_id, None, AppVisibility::Public, Some(&record), true);
    assert!(result.unwrap().is_none());
}

#[test]
fn a_record_with_a_mailbox_and_no_key_is_refused() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(Some(member.mailbox_route(&key)), false);
    let err =
        validate_mailbox(&member.service_id, None, AppVisibility::Public, Some(&record), true)
            .unwrap_err();
    assert!(err.contains("no mailbox key was supplied"), "{err}");
}

#[test]
fn a_key_with_a_record_that_names_no_mailbox_is_refused() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(None, false);
    let err = validate_mailbox(
        &member.service_id,
        Some(&credentials_for(&key)),
        AppVisibility::Public,
        Some(&record),
        true,
    )
    .unwrap_err();
    assert!(err.contains("names no mailbox"), "{err}");
}

#[test]
fn only_a_public_service_can_have_a_mailbox() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(Some(member.mailbox_route(&key)), true);
    for visibility in [AppVisibility::Internal, AppVisibility::Private] {
        let err = validate_mailbox(
            &member.service_id,
            Some(&credentials_for(&key)),
            visibility,
            Some(&record),
            true,
        )
        .unwrap_err();
        assert!(err.contains("only a public service"), "{err}");
    }
}

#[test]
fn a_mailbox_needs_an_instance_certificate() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(Some(member.mailbox_route(&key)), false);
    let err = validate_mailbox(
        &member.service_id,
        Some(&credentials_for(&key)),
        AppVisibility::Public,
        Some(&record),
        false,
    )
    .unwrap_err();
    assert!(err.contains("needs an instance certificate"), "{err}");
}

#[test]
fn a_key_that_does_not_parse_is_refused_without_showing_it() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(Some(member.mailbox_route(&key)), false);
    let bad =
        MailboxCredentials { secret_key: "not-a-key-but-secret".to_string(), methods: Vec::new() };
    let err = validate_mailbox(
        &member.service_id,
        Some(&bad),
        AppVisibility::Public,
        Some(&record),
        true,
    )
    .unwrap_err();
    assert!(err.contains("does not parse"), "{err}");
    assert!(!err.contains("not-a-key-but-secret"), "{err}");
}

#[test]
fn a_key_that_differs_from_the_record_is_refused_without_showing_it() {
    let member = Member::new();
    let key = member.mailbox_key();
    let record = member.record(Some(member.mailbox_route(&key)), false);
    let other = MailboxSecretKey::generate().unwrap();
    let err = validate_mailbox(
        &member.service_id,
        Some(&credentials_for(&other)),
        AppVisibility::Public,
        Some(&record),
        true,
    )
    .unwrap_err();
    assert!(err.contains("does not match"), "{err}");
    assert!(!err.contains(other.to_z32().as_str()), "{err}");
}

#[test]
fn the_matching_key_is_accepted() {
    let member = Member::new();
    let key = member.mailbox_key();
    let route = member.mailbox_route(&key);
    let record = member.record(Some(route.clone()), false);
    let accepted = validate_mailbox(
        &member.service_id,
        Some(&credentials_for(&key)),
        AppVisibility::Public,
        Some(&record),
        true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(accepted.key.public_key(), key.public_key());
    assert_eq!(accepted.mailbox, route.service_id);
    assert!(accepted.methods.is_empty());
}

/// A record whose JSON `info` was changed after signing: the signed packet
/// does not name the mailbox, so a deploy must not trust it.
#[test]
fn a_record_whose_mailbox_was_added_after_signing_is_refused() {
    let member = Member::new();
    let key = member.mailbox_key();
    let mut signed: SignedEndpointInfo = serde_json::from_str(&member.record(None, false)).unwrap();
    signed.info.mailbox = Some(member.mailbox_route(&key));
    let record = serde_json::to_string(&signed).unwrap();

    for credentials in [Some(credentials_for(&key)), None] {
        let err = validate_mailbox(
            &member.service_id,
            credentials.as_ref(),
            AppVisibility::Public,
            Some(&record),
            true,
        )
        .unwrap_err();
        assert!(err.contains("does not verify"), "{err}");
    }
}

#[test]
fn a_key_with_a_record_that_does_not_verify_and_names_no_mailbox_is_refused() {
    let member = Member::new();
    let key = member.mailbox_key();
    let mut signed: SignedEndpointInfo = serde_json::from_str(&member.record(None, false)).unwrap();
    signed.info.nickname = Some("changed after signing".to_string());
    let record = serde_json::to_string(&signed).unwrap();

    let err = validate_mailbox(
        &member.service_id,
        Some(&credentials_for(&key)),
        AppVisibility::Public,
        Some(&record),
        true,
    )
    .unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

// -- deploy and undeploy ---------------------------------------------------

#[tokio::test]
async fn a_deploy_with_a_mailbox_stores_the_key_in_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let service = host_of(&member, dir.path()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&key)), Some(&key));

    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();

    let stored = service.mailbox_key(&member.service_id).await.unwrap().unwrap();
    assert_eq!(stored.public_key(), key.public_key());
}

#[tokio::test]
async fn a_redeploy_without_a_mailbox_clears_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let service = host_of(&member, dir.path()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let with = member.manifest(&caller, Some(member.mailbox_route(&key)), Some(&key));
    service.deploy(member.service_id.clone(), with, &caller).await.unwrap();
    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_some());

    let without = member.manifest(&caller, None, None);
    service.deploy(member.service_id.clone(), without, &caller).await.unwrap();
    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_none());
}

#[tokio::test]
async fn undeploy_clears_the_mailbox_key() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let service = host_of(&member, dir.path()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&key)), Some(&key));
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();

    service.undeploy(member.service_id.clone(), 0, &caller).await.unwrap();

    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_deploy_with_a_mailbox_records_it_and_a_redeploy_without_one_forgets_it() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry) =
        service_with_node_identity(dir.path(), member.node_identity.clone()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let route = member.mailbox_route(&key);
    let with = member.manifest(&caller, Some(route.clone()), Some(&key));
    service.deploy(member.service_id.clone(), with, &caller).await.unwrap();

    let hosted = registry.mailbox_of(&member.service_id).unwrap();
    assert_eq!(hosted.mailbox, route.service_id);
    assert_eq!(hosted.max_ttl_secs, OPEN_MAX_TTL_SECS);

    let without = member.manifest(&caller, None, None);
    service.deploy(member.service_id.clone(), without, &caller).await.unwrap();
    assert_eq!(registry.mailbox_of(&member.service_id), None);
}

#[tokio::test]
async fn undeploy_forgets_the_recorded_mailbox() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry) =
        service_with_node_identity(dir.path(), member.node_identity.clone()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&key)), Some(&key));
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();
    assert!(registry.mailbox_of(&member.service_id).is_some());

    service.undeploy(member.service_id.clone(), 0, &caller).await.unwrap();

    assert_eq!(registry.mailbox_of(&member.service_id), None);
}

#[tokio::test]
async fn a_redeploy_to_the_same_mailbox_keeps_the_queue_lifetime_the_receiver_learned() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry) =
        service_with_node_identity(dir.path(), member.node_identity.clone()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let route = member.mailbox_route(&key);
    let first = member.manifest(&caller, Some(route.clone()), Some(&key));
    service.deploy(member.service_id.clone(), first, &caller).await.unwrap();

    let learned = HostedMailbox {
        mailbox: route.service_id.clone(),
        max_ttl_secs: 3 * 24 * 3600,
        methods: Vec::new(),
    };
    registry.set_mailbox(member.service_id.clone(), learned.clone()).await.unwrap();

    // A changed argument makes this deploy more than a no-op, so the commit
    // step runs again with the same mailbox.
    let mut redeploy = member.manifest(&caller, Some(route), Some(&key));
    redeploy.config.args = vec!["--changed".to_string()];
    service.deploy(member.service_id.clone(), redeploy, &caller).await.unwrap();

    assert_eq!(registry.mailbox_of(&member.service_id), Some(learned));
}

#[tokio::test]
async fn a_deploy_with_a_mismatched_mailbox_key_is_refused_and_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry) =
        service_with_node_identity(dir.path(), member.node_identity.clone()).await;
    let caller = caller();
    let key = member.mailbox_key();
    let other = MailboxSecretKey::generate().unwrap();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&key)), Some(&other));

    let err = service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap_err();

    assert!(err.contains("does not match"), "{err}");
    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_none());
    assert_eq!(registry.owner_of(&member.service_id), None);
}

#[tokio::test]
async fn a_redeploy_that_changes_only_the_mailbox_is_not_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let service = host_of(&member, dir.path()).await;
    let caller = caller();
    let first = member.mailbox_key();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&first)), Some(&first));
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();

    // Same member, same everything else; only the mailbox key differs.
    let second = MailboxSecretKey::generate().unwrap();
    let manifest = member.manifest(&caller, Some(member.mailbox_route(&second)), Some(&second));
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();

    let stored = service.mailbox_key(&member.service_id).await.unwrap().unwrap();
    assert_eq!(stored.public_key(), second.public_key());
}

#[tokio::test]
async fn the_same_member_on_two_substrates_stores_the_same_key() {
    let member = Member::new();
    let caller = caller();
    let mut stored = Vec::new();
    for _ in 0..2 {
        let dir = tempfile::tempdir().unwrap();
        // Each substrate has its own node identity.
        let node_identity = Arc::new(Identity::generate().unwrap());
        let host = Member {
            master: Identity::from_bytes(&member.master.to_bytes()),
            service_id: member.service_id.clone(),
            node_identity: node_identity.clone(),
        };
        let service = host_of(&host, dir.path()).await;
        let key = host.mailbox_key();
        let manifest = host.manifest(&caller, Some(host.mailbox_route(&key)), Some(&key));
        service.deploy(host.service_id.clone(), manifest, &caller).await.unwrap();
        stored.push(service.mailbox_key(&host.service_id).await.unwrap().unwrap().public_key());
    }
    assert_eq!(stored[0], stored[1]);
}

#[tokio::test]
async fn a_guest_cannot_export_the_mailbox_interface() {
    let dir = tempfile::tempdir().unwrap();
    let (service, registry) =
        service_with_node_identity(dir.path(), Arc::new(Identity::generate().unwrap())).await;

    let refused = service
        .register_wasm_endpoints("svc-guest", vec!["greeter".to_string(), "mailbox".to_string()])
        .await
        .unwrap_err();

    assert!(refused.to_string().contains("reserved"), "{refused}");
    assert!(registry.lookup("svc-guest", "greeter").is_none(), "nothing was registered");
}

#[tokio::test]
async fn a_tcp_service_cannot_take_the_mailbox_interface_either() {
    let dir = tempfile::tempdir().unwrap();
    let (service, registry) =
        service_with_node_identity(dir.path(), Arc::new(Identity::generate().unwrap())).await;
    let mut manifest = owner_test_manifest();
    let WitServiceType::Tcp(tcp) = &mut manifest.service_type else { panic!("a tcp manifest") };
    tcp.endpoints[0].interface_name = "mailbox".to_string();

    let err = service.deploy("svc-tcp".to_string(), manifest, &caller()).await.unwrap_err();

    assert!(err.contains("reserved"), "{err}");
    assert!(registry.lookup("svc-tcp", "mailbox").is_none());
}

#[tokio::test]
async fn a_container_service_cannot_take_the_mailbox_interface_either() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _registry) =
        service_with_node_identity(dir.path(), Arc::new(Identity::generate().unwrap())).await;
    let mut manifest = container_manifest_with(None);
    let WitServiceType::Container(container) = &mut manifest.service_type else {
        panic!("a container manifest")
    };
    container.ports.push(ContainerPortMapping {
        interface_name: "mailbox".to_string(),
        host_port: None,
        container_port: 80,
        protocol: "tcp".to_string(),
    });

    let err = service.deploy("svc-box".to_string(), manifest, &caller()).await.unwrap_err();

    assert!(err.contains("reserved"), "{err}");
}

// -- the operator door -----------------------------------------------------

#[tokio::test]
async fn set_secret_refuses_a_reserved_vault_name() {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(Identity::generate().unwrap());
    let (service, _registry) = service_with_node_identity(dir.path(), node_identity).await;
    let admin = CallerContext {
        caller_did: "did:key:zAdmin".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zAdmin".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate("did:key:zTestNode"),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
        on_behalf_did: None,
    };
    let call = |name: &str| NativeInvocation {
        caller: admin.clone(),
        interface: "security".to_string(),
        method: "set-secret".to_string(),
        params: serde_json::to_value(("svc".to_string(), name.to_string(), b"v".to_vec())).unwrap(),
    };

    let refused = service.dispatch(call("syneroym.mailbox-secret")).await.unwrap_err();
    assert!(matches!(refused, RpcError::InvalidParams(m) if m.contains("reserved")));
    assert!(service.dispatch(call("ordinary")).await.is_ok());
}

// -- one limit for the certificate lifetime ----------------------------------

/// The manifest and the command line accept up to
/// `MAX_INSTANCE_CERT_EXPIRES_HOURS`; the substrate refuses a longer
/// certificate at install. If the two differ, a value one side accepts fails
/// at the other.
#[test]
fn the_manifest_limit_for_the_certificate_lifetime_is_the_substrate_limit() {
    assert_eq!(
        u64::from(syneroym_app_orchestration::MAX_INSTANCE_CERT_EXPIRES_HOURS) * 3600,
        MAX_INSTANCE_CERT_LIFETIME_SECS
    );
}

// -- no database is made just to clear a key --------------------------------

#[tokio::test]
async fn a_deploy_and_undeploy_without_a_mailbox_make_no_service_database() {
    let dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(Identity::generate().unwrap());
    let (service, _registry) = service_with_node_identity(dir.path(), node_identity).await;
    let caller = caller();
    let service_id = "no-mailbox-tcp".to_string();
    let database = dir.path().join("services").join(&service_id).join("state.db");

    service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();
    assert!(!database.exists(), "a deploy without a mailbox must not create the database");

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    assert!(!database.exists(), "an undeploy must not create the database");
}

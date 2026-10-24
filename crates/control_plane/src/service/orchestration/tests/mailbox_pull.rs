//! The control plane's side of the mailbox receiver: deploy-time method
//! checks, the stored record, the wakes, and removing an old queue.

use std::{future, path::Path, sync::Mutex};

use syneroym_core::{
    dht_registry::{EndpointInfo, EndpointType},
    local_registry::EndpointRegistry,
    mailbox::{PullSummary, is_reserved_mailbox_interface},
    storage::MockStorage,
    util,
};
use syneroym_identity::Identity;
use syneroym_rpc::{Ability, AuthLevel, Capability, NativeInvocation, NativeService, RpcError};
use tokio::time::Instant;

use super::{super::*, helpers::*};
use crate::mailbox_pull::MailboxPullControl;

const METHOD: &str = "greeter#greet";

/// What a `release` call saw in the registry at the moment it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseCall {
    service_id: String,
    mailbox: String,
    had_instance_cert: bool,
    had_owner: bool,
    fact: Option<String>,
}

#[derive(Debug)]
struct FakePuller {
    registry: EndpointRegistry,
    releases: Mutex<Vec<ReleaseCall>>,
    wakes: Mutex<u32>,
    hang_on_release: bool,
}

impl FakePuller {
    fn new(registry: EndpointRegistry) -> Arc<Self> {
        Arc::new(Self {
            registry,
            releases: Mutex::new(Vec::new()),
            wakes: Mutex::new(0),
            hang_on_release: false,
        })
    }

    fn hanging(registry: EndpointRegistry) -> Arc<Self> {
        Arc::new(Self {
            registry,
            releases: Mutex::new(Vec::new()),
            wakes: Mutex::new(0),
            hang_on_release: true,
        })
    }

    fn wakes(&self) -> u32 {
        *self.wakes.lock().unwrap()
    }

    fn releases(&self) -> Vec<ReleaseCall> {
        self.releases.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl MailboxPullControl for FakePuller {
    async fn pull_now(&self) -> PullSummary {
        PullSummary { services: 7, ..PullSummary::default() }
    }

    fn wake(&self) {
        *self.wakes.lock().unwrap() += 1;
    }

    async fn release(&self, service_id: &str, mailbox: &str) {
        self.releases.lock().unwrap().push(ReleaseCall {
            service_id: service_id.to_string(),
            mailbox: mailbox.to_string(),
            had_instance_cert: self.registry.instance_cert(service_id).is_some(),
            had_owner: self.registry.owner_of(service_id).is_some(),
            fact: self.registry.mailbox_of(service_id).map(|f| f.mailbox),
        });
        if self.hang_on_release {
            future::pending::<()>().await;
        }
    }
}

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

    fn key(&self) -> MailboxSecretKey {
        MailboxSecretKey::derive_from_master(&self.master)
    }

    fn route(&self, host: &Identity) -> EndpointMailbox {
        EndpointMailbox {
            service_id: derive_did_key(&host.public_key()),
            key: self.key().public_key(),
        }
    }

    fn manifest(
        &self,
        caller: &CallerContext,
        route: Option<EndpointMailbox>,
        methods: &[&str],
    ) -> DeployManifest {
        let not_after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 86_400;
        let record = EndpointInfo {
            service_id: self.service_id.clone(),
            substrate_id: "did:key:zTestNode".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after,
            generation: 0,
            mailbox: route.clone(),
        }
        .sign(&self.master)
        .unwrap();
        let cert = instance_cert_for(
            &self.node_identity,
            &self.master,
            &caller.caller_did,
            &self.service_id,
            3600,
        );
        let mut manifest = owner_test_manifest();
        manifest.config.visibility = Some(WitVisibility::Public);
        manifest.registry_certificate = Some(serde_json::to_string(&record).unwrap());
        manifest.instance_certificate = Some(cert.to_json().unwrap());
        manifest.mailbox = route.map(|_| MailboxCredentials {
            secret_key: self.key().to_z32().to_string(),
            methods: methods.iter().map(ToString::to_string).collect(),
        });
        manifest
    }
}

fn caller() -> CallerContext {
    node_wide_caller("did:key:zOwner")
}

fn admin() -> CallerContext {
    CallerContext::verified(
        "did:key:zController",
        AuthLevel::Delegated,
        vec![Capability {
            with: ResourceUri::substrate("did:key:zTestNode"),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        }],
    )
}

async fn host(
    member: &Member,
    dir: &Path,
) -> (ControlPlaneService, EndpointRegistry, Arc<FakePuller>) {
    let (service, registry) = service_with_node_identity(dir, member.node_identity.clone()).await;
    let puller = FakePuller::new(registry.clone());
    let control: Arc<dyn MailboxPullControl> = puller.clone();
    service.set_mailbox_pull(Arc::downgrade(&control));
    // `control` is the only other owner; keep the fake alive through `puller`.
    (service, registry, puller)
}

fn mailbox_host() -> Identity {
    Identity::generate().unwrap()
}

// -- mailbox methods ---------------------------------------------------------

fn validate(member: &Member, methods: &[&str]) -> Result<Option<ValidatedMailbox>, String> {
    let caller = caller();
    let manifest = member.manifest(&caller, Some(member.route(&mailbox_host())), methods);
    validate_mailbox(
        &member.service_id,
        manifest.mailbox.as_ref(),
        AppVisibility::Public,
        manifest.registry_certificate.as_deref(),
        true,
    )
}

#[test]
fn listed_methods_are_kept_in_order() {
    let member = Member::new();
    let validated = validate(&member, &[METHOD, "api#invoke"]).unwrap().unwrap();
    assert_eq!(validated.methods, [METHOD, "api#invoke"]);
}

#[test]
fn a_node_or_platform_interface_is_refused_by_name_and_by_short_hash() {
    let member = Member::new();
    for name in ["orchestrator", "security", "supervisor", "mailbox", "data-layer", "conversation"]
    {
        assert!(is_reserved_mailbox_interface(name));
        let by_name = format!("{name}#x");
        let by_hash = format!("{}#x", util::short_hash(name));
        for entry in [by_name, by_hash] {
            let err = validate(&member, &[entry.as_str()]).unwrap_err();
            assert!(err.contains("node or platform interface"), "{entry}: {err}");
        }
    }
}

#[test]
fn a_duplicate_or_a_malformed_entry_is_refused() {
    let member = Member::new();
    let err = validate(&member, &[METHOD, METHOD]).unwrap_err();
    assert!(err.contains("listed twice"), "{err}");
    let err = validate(&member, &["no-hash"]).unwrap_err();
    assert!(err.contains("no-hash"), "{err}");
}

#[test]
fn sixty_four_methods_pass_and_sixty_five_are_refused() {
    let member = Member::new();
    let make = |n: usize| (0..n).map(|i| format!("i{i}#m")).collect::<Vec<_>>();
    let sixty_four = make(64);
    let refs: Vec<&str> = sixty_four.iter().map(String::as_str).collect();
    assert!(validate(&member, &refs).is_ok());
    let sixty_five = make(65);
    let refs: Vec<&str> = sixty_five.iter().map(String::as_str).collect();
    let err = validate(&member, &refs).unwrap_err();
    assert!(err.contains("the most allowed is 64"), "{err}");
}

#[tokio::test]
async fn methods_are_stored_and_a_redeploy_replaces_them() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry, _puller) = host(&member, dir.path()).await;
    let caller = caller();
    let route = member.route(&mailbox_host());

    let first = member.manifest(&caller, Some(route.clone()), &[METHOD]);
    service.deploy(member.service_id.clone(), first, &caller).await.unwrap();
    assert_eq!(registry.mailbox_of(&member.service_id).unwrap().methods, [METHOD]);

    let second = member.manifest(&caller, Some(route), &["greeter#other"]);
    service.deploy(member.service_id.clone(), second, &caller).await.unwrap();
    assert_eq!(registry.mailbox_of(&member.service_id).unwrap().methods, ["greeter#other"]);
}

// -- the stored record -------------------------------------------------------

#[tokio::test]
async fn the_stored_deploy_record_is_read_back_even_when_it_is_expired() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, _puller) = host(&member, dir.path()).await;
    assert!(service.deploy_record(&member.service_id).await.unwrap().is_none());

    let caller = caller();
    let route = member.route(&mailbox_host());
    let manifest = member.manifest(&caller, Some(route.clone()), &[]);
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();

    let record = service.deploy_record(&member.service_id).await.unwrap().unwrap();
    assert_eq!(record.info.mailbox.unwrap().service_id, route.service_id);
}

// -- pull-now and the wakes --------------------------------------------------

async fn security_call(
    service: &ControlPlaneService,
    caller: CallerContext,
    method: &str,
    params: serde_json::Value,
) -> syneroym_rpc::RpcResult<syneroym_rpc::NativeResponse> {
    service
        .dispatch(NativeInvocation {
            caller,
            interface: "security".to_string(),
            method: method.to_string(),
            params,
        })
        .await
}

#[tokio::test]
async fn pull_now_answers_the_summary_and_needs_the_node_owner() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, _puller) = host(&member, dir.path()).await;

    let answer =
        security_call(&service, admin(), "mailbox-pull-now", serde_json::json!([])).await.unwrap();
    assert_eq!(serde_json::from_value::<PullSummary>(answer.payload).unwrap().services, 7);

    let err = security_call(&service, caller(), "mailbox-pull-now", serde_json::json!([]))
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Custom(..)), "{err:?}");
}

#[tokio::test]
async fn pull_now_without_a_receiver_is_method_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry) =
        service_with_node_identity(dir.path(), member.node_identity.clone()).await;
    let err = security_call(&service, admin(), "mailbox-pull-now", serde_json::json!([]))
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::MethodNotFound(_)), "{err:?}");
}

#[tokio::test]
async fn injecting_the_kek_wakes_the_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, puller) = host(&member, dir.path()).await;
    security_call(&service, admin(), "inject-kek", serde_json::json!((hex::encode([1u8; 32]),)))
        .await
        .unwrap();
    assert_eq!(puller.wakes(), 1);
}

#[tokio::test]
async fn a_renewed_certificate_wakes_the_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry, _dispatch) =
        service_with_dispatch(dir.path(), member.node_identity.clone()).await;
    let puller = FakePuller::new(registry);
    let control: Arc<dyn MailboxPullControl> = puller.clone();
    service.set_mailbox_pull(Arc::downgrade(&control));
    let caller = caller();
    service
        .deploy(member.service_id.clone(), member.manifest(&caller, None, &[]), &caller)
        .await
        .unwrap();
    assert_eq!(puller.wakes(), 0, "a deploy without a mailbox does not wake it");

    let cert = instance_cert_for(
        &member.node_identity,
        &member.master,
        &caller.caller_did,
        &member.service_id,
        7200,
    );
    service
        .dispatch(NativeInvocation {
            caller: caller.clone(),
            interface: "orchestrator".to_string(),
            method: "renew-cert".to_string(),
            params: serde_json::json!((member.service_id.clone(), 0u64, cert.to_json().unwrap())),
        })
        .await
        .unwrap();
    assert_eq!(puller.wakes(), 1);
}

#[tokio::test]
async fn a_deploy_with_a_mailbox_wakes_the_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, puller) = host(&member, dir.path()).await;
    let caller = caller();
    let manifest = member.manifest(&caller, Some(member.route(&mailbox_host())), &[]);
    service.deploy(member.service_id.clone(), manifest, &caller).await.unwrap();
    assert_eq!(puller.wakes(), 1);
}

// -- removing the old queue --------------------------------------------------

#[tokio::test]
async fn an_undeploy_releases_the_queue_while_the_certificate_and_owner_are_still_installed() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, puller) = host(&member, dir.path()).await;
    let caller = caller();
    let route = member.route(&mailbox_host());
    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(route.clone()), &[]),
            &caller,
        )
        .await
        .unwrap();

    service.undeploy(member.service_id.clone(), 0, &caller).await.unwrap();

    // The fact is gone before the call (a running pass must not register the
    // queue again); the certificate and owner are still there (a remote
    // mailbox admits the call only under that instance).
    assert_eq!(
        puller.releases(),
        [ReleaseCall {
            service_id: member.service_id.clone(),
            mailbox: route.service_id,
            had_instance_cert: true,
            had_owner: true,
            fact: None,
        }]
    );
}

#[tokio::test]
async fn a_redeploy_to_another_mailbox_or_to_none_releases_the_old_one_after_the_new_fact() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, puller) = host(&member, dir.path()).await;
    let caller = caller();
    let first = member.route(&mailbox_host());
    let second = member.route(&mailbox_host());
    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(first.clone()), &[]),
            &caller,
        )
        .await
        .unwrap();
    assert!(puller.releases().is_empty());

    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(second.clone()), &[]),
            &caller,
        )
        .await
        .unwrap();
    let calls = puller.releases();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].mailbox, first.service_id);
    assert_eq!(calls[0].fact.as_deref(), Some(second.service_id.as_str()));
    assert!(calls[0].had_instance_cert);

    service
        .deploy(member.service_id.clone(), member.manifest(&caller, None, &[]), &caller)
        .await
        .unwrap();
    let calls = puller.releases();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].mailbox, second.service_id);
    assert_eq!(calls[1].fact, None);
}

#[tokio::test]
async fn a_redeploy_without_a_mailbox_or_a_certificate_releases_before_the_certificate_goes() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, registry, puller) = host(&member, dir.path()).await;
    let caller = caller();
    let route = member.route(&mailbox_host());
    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(route.clone()), &[]),
            &caller,
        )
        .await
        .unwrap();
    let mut bare = member.manifest(&caller, None, &[]);
    bare.instance_certificate = None;

    service.deploy(member.service_id.clone(), bare, &caller).await.unwrap();

    assert_eq!(
        puller.releases(),
        [ReleaseCall {
            service_id: member.service_id.clone(),
            mailbox: route.service_id,
            had_instance_cert: true,
            had_owner: true,
            fact: None,
        }]
    );
    assert!(registry.instance_cert(&member.service_id).is_none());
}

#[tokio::test]
async fn a_redeploy_to_the_same_mailbox_releases_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let member = Member::new();
    let (service, _registry, puller) = host(&member, dir.path()).await;
    let caller = caller();
    let route = member.route(&mailbox_host());
    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(route.clone()), &[]),
            &caller,
        )
        .await
        .unwrap();
    service
        .deploy(
            member.service_id.clone(),
            member.manifest(&caller, Some(route), &["greeter#greet"]),
            &caller,
        )
        .await
        .unwrap();
    assert!(puller.releases().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_release_that_never_returns_is_given_up_after_the_timeout() {
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let puller = FakePuller::hanging(registry);

    let started = Instant::now();
    mailbox_key::release_within(
        mailbox_key::RELEASE_TIMEOUT,
        puller.as_ref(),
        "svc",
        "did:key:zBox",
    )
    .await;

    assert_eq!(started.elapsed(), Duration::from_secs(30));
    assert_eq!(puller.releases().len(), 1);
}

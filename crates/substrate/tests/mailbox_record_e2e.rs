#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A service member's mailbox in its signed record (ADR-0026), end to end
//! against one live `syneroym-substrate`: the record carries the mailbox
//! and the key derived from the member master, the substrate keeps the
//! certificate lifetime it was given, and a sender finds the mailbox of a
//! peer whose substrate record is gone.

use std::time::{SystemTime, UNIX_EPOCH};

use common::SubstrateNode;
use rustls::crypto::ring;
use syneroym_core::dht_registry::{
    DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMailbox, EndpointType, RegistryClient,
    SignedEndpointInfo,
};
use syneroym_identity::{Identity, substrate};
use syneroym_mailbox_format::MailboxSecretKey;
use syneroym_router::{
    net_iroh::{resolve_iroh_addr, resolve_peer},
    peer_records::PeerRecordStore,
};
use syneroym_sdk::{
    NetworkEndpoint, Publication, SyneroymClient, deploy,
    deploy::{certify_instance, member_registry_record},
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, MailboxCredentials, ServiceConfig, ServiceType, TcpManifest, Visibility,
};

mod common;

const CERT_HOURS: u64 = 168;

fn did_of(master: &Identity) -> String {
    substrate::derive_did_key(&master.public_key())
}

fn not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

struct Fixture {
    node: SubstrateNode,
    operator: SyneroymClient,
    registry: RegistryClient,
}

async fn boot() -> Fixture {
    let _ = ring::default_provider().install_default();
    let owner = Identity::generate().unwrap();
    let node = SubstrateNode::builder().owner(&owner).inject_kek_bytes([0x55; 32]).boot().await;
    let mut operator = node.client_as(Identity::from_bytes(&owner.to_bytes()));
    operator.connect().await.expect("failed to connect to the node");
    let registry = RegistryClient::new(false, Some(node.registry_url().to_string()));
    Fixture { node, operator, registry }
}

fn endpoints(port: u16) -> Vec<NetworkEndpoint> {
    vec![NetworkEndpoint {
        interface_name: "default".to_string(),
        host: "127.0.0.1".to_string(),
        port,
    }]
}

fn route_for(master: &Identity, host: &str) -> EndpointMailbox {
    EndpointMailbox {
        service_id: host.to_string(),
        key: MailboxSecretKey::derive_from_master(master).public_key(),
    }
}

/// A signed service record of `master` on `substrate_id`, public, naming a
/// mailbox.
fn record_with_mailbox(
    master: &Identity,
    substrate_id: &str,
    is_private: bool,
) -> SignedEndpointInfo {
    EndpointInfo {
        service_id: did_of(master),
        substrate_id: substrate_id.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private,
        ttl: None,
        not_after: not_after(),
        generation: 0,
        mailbox: Some(route_for(master, &did_of(&Identity::generate().unwrap()))),
    }
    .sign(master)
    .unwrap()
}

#[tokio::test]
async fn a_deploy_with_a_mailbox_publishes_the_derived_key_and_keeps_the_certificate_lifetime() {
    let fx = boot().await;
    let master = Identity::generate().unwrap();
    let id = did_of(&master);
    let host = did_of(&Identity::generate().unwrap());
    fx.registry.publish_master_anchor(&id, vec![], None, &master, true).await.unwrap();

    let cert = certify_instance(&fx.operator, &master, &id, CERT_HOURS).await.unwrap();
    let record = member_registry_record(
        deploy::Visibility::Public,
        &id,
        fx.node.did(),
        &master,
        not_after(),
        Some(route_for(&master, &host)),
    )
    .unwrap()
    .unwrap();
    let key = MailboxSecretKey::derive_from_master(&master);
    let publication = Publication::Public {
        record: serde_json::from_str(&record).unwrap(),
        mailbox_key: Some(key.clone()),
        mailbox_methods: Vec::new(),
        mailbox_child_token: None,
    };

    fx.operator
        .deploy_svc_tcp(id.clone(), endpoints(41501), publication, Some(cert))
        .await
        .expect("a deploy with a mailbox must succeed");

    // The record names the mailbox and the key derived from the master.
    let published = fx.registry.lookup(&id, false).await.unwrap();
    let mailbox = published.info.mailbox.expect("the published record names a mailbox");
    assert_eq!(mailbox.service_id, host);
    assert_eq!(mailbox.key, key.public_key());

    // The lifetime the deployer chose is the lifetime installed.
    let listed = fx.operator.list_svcs().await.unwrap();
    let entry = listed.iter().find(|s| s.service_id == id).unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let expires = entry.instance_certificate_expires_at.expect("a certificate is installed");
    assert!(expires.abs_diff(now + CERT_HOURS * 3600) < 120, "expires at {expires}, now {now}");

    // The sender's record store was opened at boot.
    assert!(fx.node.base_path().join("data/db/peer_records.db").exists());
}

#[tokio::test]
async fn a_peer_with_a_mailbox_but_no_substrate_record_still_resolves() {
    let fx = boot().await;
    let master = Identity::generate().unwrap();
    // Its substrate has no record anywhere.
    let asleep = did_of(&Identity::generate().unwrap());
    let record = record_with_mailbox(&master, &asleep, false);
    fx.registry.register(&record, false).await.expect("the registry must admit the record");

    let peer = resolve_peer(&fx.registry, None, &did_of(&master)).await.unwrap();

    assert_eq!(peer.mailbox, record.info.mailbox);
    assert!(peer.addr.is_none());
    assert!(resolve_iroh_addr(&fx.registry, &did_of(&master)).await.is_err());
}

#[tokio::test]
async fn a_stored_record_routes_a_direct_call_when_the_registry_has_none() {
    let fx = boot().await;
    let master = Identity::generate().unwrap();
    // Names this running node, but was never registered.
    let record = record_with_mailbox(&master, fx.node.did(), false);
    let store = PeerRecordStore::open_in_memory().unwrap();
    store.remember(&record).await.unwrap();

    let peer = resolve_peer(&fx.registry, Some(&store), &did_of(&master)).await.unwrap();

    let node_addr = resolve_iroh_addr(&fx.registry, fx.node.did()).await.unwrap();
    assert_eq!(peer.addr.expect("the stored record gives the address").id, node_addr.id);
    assert!(resolve_peer(&fx.registry, None, &did_of(&master)).await.is_err());
}

fn manifest(
    visibility: Visibility,
    record: Option<&SignedEndpointInfo>,
    mailbox_key: &MailboxSecretKey,
) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: Some(visibility),
        },
        service_type: ServiceType::Tcp(TcpManifest { endpoints: endpoints(41502) }),
        registry_certificate: record.map(|r| serde_json::to_string(r).unwrap()),
        instance_certificate: None,
        mailbox: Some(MailboxCredentials {
            secret_key: mailbox_key.to_z32().to_string(),
            methods: Vec::new(),
            child_token: None,
        }),
    }
}

#[tokio::test]
async fn a_mailbox_on_a_service_that_is_not_public_is_refused() {
    let fx = boot().await;
    let master = Identity::generate().unwrap();
    let id = did_of(&master);
    let key = MailboxSecretKey::derive_from_master(&master);

    // Private: no record at all, so a key has nothing to match.
    let err = common::try_deploy_app(&fx.operator, &id, manifest(Visibility::Private, None, &key))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("names no mailbox"), "{err}");

    // Internal: a record that names a mailbox, signed by the master.
    let record = record_with_mailbox(&master, fx.node.did(), true);
    let err = common::try_deploy_app(
        &fx.operator,
        &id,
        manifest(Visibility::Internal, Some(&record), &key),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("only a public service"), "{err}");
    assert!(fx.operator.list_svcs().await.unwrap().iter().all(|s| s.service_id != id));
}

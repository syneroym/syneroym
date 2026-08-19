#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! ADR-0018's publication declaration, proven end to end against one live
//! `syneroym-substrate` instance (M06B B2). §7 tests 33-36, 40 of the
//! slice's implementation plan: the substrate refuses a mis-declared
//! deploy, publishes exactly what was declared, clears a stale record on a
//! redeploy to `private`, and reports the declared visibility back through
//! `orchestrator/list`.
//!
//! Cross-node resolution for `internal`/`public` (tests 38-39) is proven in
//! `multi_substrate_placement_e2e.rs`, which already boots two independent
//! substrates -- duplicating that harness here would only add boot time.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::dht_registry::{
    DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, Publication, ServiceConfig, ServiceType, SyneroymClient,
    TcpManifest, Visibility,
};

mod common;
use common::SubstrateTestContext;

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// A minimal TCP manifest, never actually dialed -- mirrors
/// `master_endpoint_record_e2e.rs`'s own `bare_tcp_manifest`, generalized
/// over `visibility`/`registry_certificate` so each test can drive
/// `validate_publication`'s exact combination.
fn tcp_manifest(
    port: u16,
    visibility: Option<Visibility>,
    registry_certificate: Option<String>,
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
            visibility,
        },
        service_type: ServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate,
        instance_certificate: None,
    }
}

async fn deploy_raw(
    client: &SyneroymClient,
    service_id: &str,
    manifest: DeployManifest,
) -> anyhow::Result<()> {
    let params = serde_json::to_value((service_id.to_string(), manifest))?;
    let res = client.request("orchestrator", "deploy", params).await?;
    if res.result == json!({"status": "deployed"}) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("deploy did not report success: {:?}", res.result))
    }
}

/// Test 33: a service deployed with no declared visibility and no
/// certificate is not published -- a registry lookup for it misses. The
/// exit criterion's first half, and failure-matrix row 11.
#[tokio::test]
async fn undeclared_visibility_deploys_and_publishes_nothing() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;

    let svc_identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&svc_identity.public_key());

    deploy_raw(&ctx.substrate_client, &svc_id, tcp_manifest(45001, None, None))
        .await
        .expect("a deploy with no visibility declaration and no certificate must succeed");

    let registry_client =
        RegistryClient::new(false, Some(format!("http://localhost:{registry_port}")));
    let err = registry_client
        .lookup(&svc_id, false)
        .await
        .expect_err("an undeclared service must not be published");
    let _ = err; // any error (miss / not found) proves the record was never written

    ctx.teardown().await;
}

/// Test 34: declaring `public` with no registry certificate is refused,
/// loudly, at deploy time -- ADR-0018 §4's `validate_publication`, naming
/// the missing certificate.
#[tokio::test]
async fn declaring_public_with_no_certificate_is_refused() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;

    let svc_identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&svc_identity.public_key());

    let err = deploy_raw(
        &ctx.substrate_client,
        &svc_id,
        tcp_manifest(45002, Some(Visibility::Public), None),
    )
    .await
    .expect_err("declaring public with no certificate must be refused");
    let msg = err.to_string();
    assert!(msg.contains("no registry certificate was supplied"), "{msg}");

    ctx.teardown().await;
}

/// Test 35: `public` with a matching certificate publishes, and the record
/// resolves through the registry.
#[tokio::test]
async fn declaring_public_with_a_matching_certificate_publishes_and_resolves() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;

    let svc_identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&svc_identity.public_key());
    let record = EndpointInfo {
        service_id: svc_id.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: far_future_not_after(),
        generation: 0,
    }
    .sign(&svc_identity)
    .unwrap();

    deploy_raw(
        &ctx.substrate_client,
        &svc_id,
        tcp_manifest(
            45003,
            Some(Visibility::Public),
            Some(serde_json::to_string(&record).unwrap()),
        ),
    )
    .await
    .expect("declaring public with a matching certificate must succeed");

    let registry_client =
        RegistryClient::new(false, Some(format!("http://localhost:{registry_port}")));
    let signed = registry_client
        .lookup(&svc_id, false)
        .await
        .expect("a declared-public service with a valid certificate must resolve");
    assert_eq!(signed.info.service_id, svc_id);
    assert!(!signed.info.is_private);

    ctx.teardown().await;
}

/// Test 36: a public service redeployed as `private` records the new
/// declaration (`D-B2-5` -- the actual removal of the stale record *file*,
/// so the substrate stops republishing it on the next heartbeat sweep, is
/// proven at the unit level in `control_plane::service::orchestration`'s
/// `a_private_redeploy_removes_the_stored_endpoint_record_file`, which has
/// direct access to `hosted_apps_dir`; the already-registered HTTP record
/// itself is not retroactively revoked and keeps resolving until its own
/// `not_after` lapses, by design -- F2's defect was the substrate
/// continuing to *republish* it, not the registry's own TTL).
#[tokio::test]
async fn a_public_service_redeployed_private_is_recorded_as_private() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;

    let svc_identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&svc_identity.public_key());
    let record = EndpointInfo {
        service_id: svc_id.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: far_future_not_after(),
        generation: 0,
    }
    .sign(&svc_identity)
    .unwrap();

    deploy_raw(
        &ctx.substrate_client,
        &svc_id,
        tcp_manifest(
            45004,
            Some(Visibility::Public),
            Some(serde_json::to_string(&record).unwrap()),
        ),
    )
    .await
    .expect("initial public deploy must succeed");

    let registry_client =
        RegistryClient::new(false, Some(format!("http://localhost:{registry_port}")));
    registry_client
        .lookup(&svc_id, false)
        .await
        .expect("the service must be published after the first, public deploy");

    deploy_raw(
        &ctx.substrate_client,
        &svc_id,
        tcp_manifest(45004, Some(Visibility::Private), None),
    )
    .await
    .expect("redeploying as private must succeed");

    let services = ctx.substrate_client.list_svcs().await.expect("list_svcs failed");
    let entry =
        services.iter().find(|s| s.service_id == svc_id).expect("service missing from list");
    assert_eq!(entry.visibility, Some(Visibility::Private));

    ctx.teardown().await;
}

/// Test 40: `orchestrator/list` (and, transitively, `roymctl svc list`)
/// distinguishes a private service from a public one -- ADR-0018 §4's
/// whole complaint: without this, "deliberately private" and "forgotten"
/// are indistinguishable from the outside.
#[tokio::test]
async fn list_distinguishes_a_private_service_from_a_public_one() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports();
    let mut ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    ctx.substrate_client.wait_for_ready(Duration::from_secs(10)).await.unwrap();

    let private_identity = Identity::generate().unwrap();
    let private_id = substrate::derive_did_key(&private_identity.public_key());
    ctx.substrate_client
        .deploy_svc_tcp(
            private_id.clone(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 45005,
            }],
            Publication::Private,
            None,
        )
        .await
        .expect("private deploy must succeed");

    let public_identity = Identity::generate().unwrap();
    let public_id = substrate::derive_did_key(&public_identity.public_key());
    let record = EndpointInfo {
        service_id: public_id.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: far_future_not_after(),
        generation: 0,
    }
    .sign(&public_identity)
    .unwrap();
    ctx.substrate_client
        .deploy_svc_tcp(
            public_id.clone(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 45006,
            }],
            Publication::Public(record),
            None,
        )
        .await
        .expect("public deploy must succeed");

    let services = ctx.substrate_client.list_svcs().await.expect("list_svcs failed");
    let private_entry = services
        .iter()
        .find(|s| s.service_id == private_id)
        .expect("private service missing from list");
    let public_entry = services
        .iter()
        .find(|s| s.service_id == public_id)
        .expect("public service missing from list");
    assert_eq!(private_entry.visibility, Some(Visibility::Private));
    assert_eq!(public_entry.visibility, Some(Visibility::Public));

    ctx.teardown().await;
}

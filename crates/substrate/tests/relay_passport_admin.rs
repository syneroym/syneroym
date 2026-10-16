#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration test for coordinator relay passport administration through
//! the control plane and roymctl command code.

mod common;

use std::path::{Path, PathBuf};

use common::SubstrateNode;
use reqwest::Certificate;
use rustls::crypto::ring;
use serde_json::Map;
use syneroym_core::config::{CoordinatorAccess, SubstrateTlsConfig};
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    Challenge, PassportId, RELAY_HOME,
    wire::{CHALLENGE_PATH, ChallengeResponse, REGISTER_PATH, RegisterRequest, RegisterResponse},
};
use syneroym_rpc::{JsonRpcError, PERMISSION_DENIED_CODE};

fn setup_certs(dir: &Path) -> (PathBuf, PathBuf) {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    common::generate_self_signed_cert(&cert_path, &key_path, "coord");
    (cert_path, key_path)
}

async fn register_test_passport(
    info_port: u16,
    cert_path: &Path,
    controller: &Identity,
    node_did: &str,
) -> anyhow::Result<PassportId> {
    let cert_pem = std::fs::read(cert_path)?;
    let reqwest_cert = Certificate::from_pem(&cert_pem)?;
    let https_client = reqwest::Client::builder().add_root_certificate(reqwest_cert).build()?;

    let ch_url = format!("https://127.0.0.1:{info_port}{CHALLENGE_PATH}");
    let ch_res: ChallengeResponse = https_client.post(&ch_url).send().await?.json().await?;
    let challenge = Challenge::from_hex(&ch_res.challenge)?;

    let owner = Identity::generate()?;
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let client_node = Identity::generate()?;
    let client_node_did = substrate::derive_did_key(&client_node.public_key());

    let now = syneroym_passport::now_secs();
    let passport = syneroym_passport::issue_passport(
        controller,
        node_did,
        &RELAY_HOME,
        &owner_did,
        Map::new(),
        86_400,
        now,
    )?;
    let passport_id = PassportId::of(&passport)?;
    let child = syneroym_passport::child_token(&owner, &passport, &client_node_did, 86_400, now)?;

    let signature =
        syneroym_passport::sign_challenge(&client_node, node_did, &RELAY_HOME, &challenge, &child)?;
    let req = RegisterRequest {
        token: child,
        challenge: ch_res.challenge,
        signer_did: client_node_did,
        signature,
    };

    let reg_url = format!("https://127.0.0.1:{info_port}{REGISTER_PATH}");
    let reg_res: RegisterResponse =
        https_client.post(&reg_url).body(serde_json::to_vec(&req)?).send().await?.json().await?;
    assert_eq!(reg_res.passport_id, passport_id);
    Ok(passport_id)
}

#[tokio::test]
async fn relay_passport_admin_controls_leases_and_denies() -> anyhow::Result<()> {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    // 1. Boot an open relay node.
    let open = SubstrateNode::builder().boot().await;

    // 2. Allocate an info port and self-signed certificates.
    let [info_port] = common::alloc_ports::<1>();
    common::release_held_ports(&[info_port]);

    let cert_dir = tempfile::tempdir()?;
    let (cert_path, key_path) = setup_certs(cert_dir.path());

    // 3. Make the passport node a claimed node with verified controller.
    let controller = Identity::generate()?;
    let passport_node_dir = tempfile::tempdir()?;
    let node_key = common::write_claim(passport_node_dir.path(), &controller);
    let node_did = substrate::derive_did_key(&node_key.public_key());

    let node_cert_path = cert_path.clone();
    let node_key_path = key_path.clone();
    let node = SubstrateNode::builder()
        .unowned()
        .base_path(passport_node_dir.path())
        .shared_registry(open.registry_url())
        .shared_relay(open.relay_url())
        .configure(move |c| {
            c.tls = Some(SubstrateTlsConfig {
                cert_path: node_cert_path.clone(),
                key_path: node_key_path.clone(),
                reload_on_sigusr1: false,
            });
            if let Some(coord) = &mut c.roles.coordinator {
                coord.access = CoordinatorAccess::Passport;
                if let Some(iroh) = &mut coord.iroh {
                    iroh.info_http_bind_address = Some(format!("127.0.0.1:{info_port}"));
                }
            }
        })
        .boot()
        .await;

    // 4. Register a passport over HTTPS.
    let passport_id = register_test_passport(info_port, &cert_path, &controller, &node_did).await?;

    // 5. Save controller to roymctl dir and invoke roymctl coordinator commands.
    let roymctl_dir = tempfile::tempdir()?;
    let identities_dir = roymctl_dir.path().join("identities");
    std::fs::create_dir_all(&identities_dir)?;
    controller.save_to_path(identities_dir.join("controller.key"))?;

    let leases_list = roymctl::commands::coordinator::leases(
        node.registry_url(),
        Some(node.did().to_string()),
        roymctl_dir.path(),
        Some("controller"),
        None,
    )
    .await?;
    assert_eq!(leases_list.len(), 1);
    assert_eq!(leases_list[0].passport_id, passport_id);

    let ended = roymctl::commands::coordinator::deny(
        passport_id.as_str(),
        node.registry_url(),
        Some(node.did().to_string()),
        roymctl_dir.path(),
        Some("controller"),
        None,
    )
    .await?;
    assert_eq!(ended, 1);

    // Verify lease was removed via SDK client.
    let mut controller_client = node.client_as(Identity::from_bytes(&controller.to_bytes()));
    controller_client.connect().await?;
    let leases_after = controller_client.relay_passport_leases().await?;
    assert!(leases_after.is_empty(), "leases must be empty after deny");
    controller_client.shutdown().await.ok();

    // 6. A client acting as an unrelated identity is denied on both verbs.
    let stranger = Identity::generate()?;
    let mut stranger_client = node.client_as(stranger);
    stranger_client.connect().await?;

    let err_leases = stranger_client.relay_passport_leases().await.unwrap_err();
    assert_eq!(
        err_leases.downcast_ref::<JsonRpcError>().map(|e| e.code),
        Some(PERMISSION_DENIED_CODE)
    );

    let err_deny = stranger_client.relay_passport_deny(&passport_id).await.unwrap_err();
    assert_eq!(
        err_deny.downcast_ref::<JsonRpcError>().map(|e| e.code),
        Some(PERMISSION_DENIED_CODE)
    );
    stranger_client.shutdown().await.ok();

    // 7. A node without passport access (the open relay) answers MethodNotFound.
    let open_owner = Identity::from_bytes(&open.owner().expect("open owner").to_bytes());
    let mut open_client = open.client_as(open_owner);
    open_client.connect().await?;
    let err_open = open_client.relay_passport_leases().await.unwrap_err();
    assert_eq!(err_open.downcast_ref::<JsonRpcError>().map(|e| e.code), Some(-32601));
    open_client.shutdown().await.ok();

    node.teardown().await;
    open.teardown().await;
    Ok(())
}

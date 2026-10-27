//! Shared test harness for coordinator relay passport integration tests.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr,
};
use reqwest::Certificate;
use serde_json::Map;
use syneroym_core::config::{CoordinatorAccess, SubstrateTlsConfig};
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    CapabilityToken, Challenge, PassportId, RELAY_HOME,
    wire::{CHALLENGE_PATH, ChallengeResponse, REGISTER_PATH, RegisterRequest, RegisterResponse},
};
use syneroym_router::{SYNEROYM_ALPN, net_iroh};
use tempfile::TempDir;
use tokio::time;

use super::SubstrateNode;

pub fn setup_certs(dir: &Path) -> (PathBuf, PathBuf) {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    super::generate_self_signed_cert(&cert_path, &key_path, "coord");
    (cert_path, key_path)
}

pub async fn register_test_passport(
    info_port: u16,
    cert_path: &Path,
    controller: &Identity,
    node_did: &str,
) -> anyhow::Result<PassportId> {
    let cert_pem = fs::read(cert_path)?;
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

pub struct PassportRelay {
    pub open: SubstrateNode,
    pub relay: SubstrateNode,
    pub controller: Identity,
    pub cert_pem: String,
    pub info_url: String,
    _cert_dir: TempDir,
    _passport_node_dir: TempDir,
}

impl PassportRelay {
    pub async fn boot() -> Self {
        let open = SubstrateNode::builder().boot().await;

        let [info_port] = super::alloc_ports::<1>();
        super::release_held_ports(&[info_port]);

        let cert_dir = tempfile::tempdir().expect("tempdir for certs");
        let (cert_path, key_path) = setup_certs(cert_dir.path());
        let cert_pem = fs::read_to_string(&cert_path).expect("read cert pem");

        let controller = Identity::generate().expect("generate controller identity");
        let passport_node_dir = tempfile::tempdir().expect("tempdir for passport node");
        let _node_key = super::write_claim(passport_node_dir.path(), &controller);

        let node_cert_path = cert_path.clone();
        let node_key_path = key_path.clone();
        let relay = SubstrateNode::builder()
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

        let info_url = format!("https://127.0.0.1:{info_port}");
        Self {
            open,
            relay,
            controller,
            cert_pem,
            info_url,
            _cert_dir: cert_dir,
            _passport_node_dir: passport_node_dir,
        }
    }

    pub fn relay_url(&self) -> &str {
        self.relay.relay_url()
    }

    pub async fn restart_relay(&mut self) {
        self.relay.restart_in_place().await;
    }

    pub fn issue(&self, owner_did: &str, secs: u64) -> CapabilityToken {
        let now = syneroym_passport::now_secs();
        syneroym_passport::issue_passport(
            &self.controller,
            self.relay.did(),
            &RELAY_HOME,
            owner_did,
            Map::new(),
            secs,
            now,
        )
        .expect("issue passport")
    }

    pub async fn leased_peer(&self, passport: &CapabilityToken, owner: &Identity) -> Endpoint {
        let peer_key = Identity::generate().expect("generate peer identity");
        let peer_did = substrate::derive_did_key(&peer_key.public_key());
        let now = syneroym_passport::now_secs();
        let life = passport.expires_at_secs.saturating_sub(now);
        let child = syneroym_passport::child_token(owner, passport, &peer_did, life, now)
            .expect("child token");

        let cert = Certificate::from_pem(self.cert_pem.as_bytes()).expect("cert pem");
        let client =
            reqwest::Client::builder().add_root_certificate(cert).build().expect("https client");

        let ch_url = format!("{}{CHALLENGE_PATH}", self.info_url);
        let ch_res: ChallengeResponse = client
            .post(&ch_url)
            .send()
            .await
            .expect("challenge request")
            .json()
            .await
            .expect("challenge json");
        let challenge = Challenge::from_hex(&ch_res.challenge).expect("parse challenge hex");

        let signature = syneroym_passport::sign_challenge(
            &peer_key,
            self.relay.did(),
            &RELAY_HOME,
            &challenge,
            &child,
        )
        .expect("sign challenge");

        let req = RegisterRequest {
            token: child,
            challenge: ch_res.challenge,
            signer_did: peer_did,
            signature,
        };
        let reg_url = format!("{}{REGISTER_PATH}", self.info_url);
        let reg_res: RegisterResponse = client
            .post(&reg_url)
            .body(serde_json::to_vec(&req).expect("serialize register request"))
            .send()
            .await
            .expect("register request")
            .json()
            .await
            .expect("register json");
        assert_eq!(reg_res.passport_id, PassportId::of(passport).expect("passport id"));

        let relay_url: RelayUrl = self.relay_url().parse().expect("parse relay url");
        let secret_key = SecretKey::from_bytes(&peer_key.to_bytes());
        net_iroh::empty_endpoint_builder()
            .clear_ip_transports()
            .secret_key(secret_key)
            .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)))
            .bind()
            .await
            .expect("bind leased peer endpoint")
    }

    pub async fn teardown(self) {
        self.relay.teardown().await;
        self.open.teardown().await;
    }
}

pub async fn reaches(
    peer: &Endpoint,
    node: &SubstrateNode,
    relay: RelayUrl,
    wait: Duration,
) -> bool {
    let key = substrate::resolve_did_key(node.did()).expect("resolve node did");
    let node_id = EndpointId::from_bytes(key.as_bytes()).expect("endpoint id");
    let addr = EndpointAddr::from_parts(node_id, [TransportAddr::Relay(relay)]);
    let deadline = time::Instant::now() + wait;
    while time::Instant::now() < deadline {
        let dial = peer.connect(addr.clone(), SYNEROYM_ALPN);
        if let Ok(Ok(conn)) = time::timeout(Duration::from_millis(500), dial).await {
            conn.close(0u32.into(), b"reach test done");
            return true;
        }
        time::sleep(Duration::from_millis(200)).await;
    }
    false
}

pub fn owner_dir(owner: &Identity) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir for owner");
    let identities_dir = dir.path().join("identities");
    fs::create_dir_all(&identities_dir).expect("create identities dir");
    owner.save_to_path(identities_dir.join("owner.key")).expect("save owner key");
    dir
}

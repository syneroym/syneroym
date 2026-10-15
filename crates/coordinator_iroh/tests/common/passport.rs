//! Helpers for relay passport integration tests.

use std::{fs, net::SocketAddr, path::Path, time::Duration};

use anyhow::{Result, ensure};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr,
    protocol::Router,
};
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value};
use syneroym_coordinator_iroh::CoordinatorIroh;
use syneroym_core::{
    config::{
        CoordinatorAccess, CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig,
        SubstrateTlsConfig,
    },
    host_identity::HostIdentity,
};
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    CapabilityToken, Challenge, RELAY_HOME,
    wire::{ChallengeResponse, ErrorBody, RegisterRequest, RegisterResponse},
};

pub const TEST_ALPN: &[u8] = b"syneroym-test/passport-echo";

/// A coordinator with a relay, `[tls]` on its info server, and `access`.
pub async fn passport_coordinator(
    base: &Path,
    access: CoordinatorAccess,
    host: &HostIdentity,
) -> Result<(CoordinatorIroh, Client)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert_path = base.join("cert.pem");
    let key_path = base.join("key.pem");
    if !cert_path.exists() {
        super::generate_self_signed_cert(&cert_path, &key_path, "coord");
    }
    let cert_pem = fs::read(&cert_path)?;
    let reqwest_cert = reqwest::Certificate::from_pem(&cert_pem)?;
    let client = Client::builder().add_root_certificate(reqwest_cert).build()?;

    let mut config = SubstrateConfig {
        app_local_data_dir: base.join("data"),
        app_data_dir: base.join("user_data"),
        app_cache_dir: base.join("cache"),
        app_log_dir: base.join("logs"),
        tls: Some(SubstrateTlsConfig { cert_path, key_path, reload_on_sigusr1: false }),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access,
        iroh: Some(CoordinatorIrohConfig {
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    config.resolve_paths();
    let coord = CoordinatorIroh::init(&config, host).await?;
    Ok((coord, client))
}

pub struct Fixture {
    pub host_key: Identity,
    pub owner: Identity,
}

impl Fixture {
    pub fn new() -> Self {
        Self { host_key: Identity::generate().unwrap(), owner: Identity::generate().unwrap() }
    }

    pub fn with_keys(host_key: Identity, owner: Identity) -> Self {
        Self { host_key, owner }
    }

    /// A passport from the host node key to `owner`, with `limits`, living
    /// `secs`.
    pub fn passport(&self, limits: Map<String, Value>, secs: u64) -> CapabilityToken {
        let host_did = substrate::derive_did_key(&self.host_key.public_key());
        let owner_did = substrate::derive_did_key(&self.owner.public_key());
        let now = syneroym_passport::now_secs();
        syneroym_passport::issue_passport(
            &self.host_key,
            &host_did,
            &RELAY_HOME,
            &owner_did,
            limits,
            secs,
            now,
        )
        .unwrap()
    }

    /// The child token for `node`, living `secs`.
    pub fn child(&self, passport: &CapabilityToken, node: &Identity, secs: u64) -> CapabilityToken {
        let node_did = substrate::derive_did_key(&node.public_key());
        let now = syneroym_passport::now_secs();
        syneroym_passport::child_token(&self.owner, passport, &node_did, secs, now).unwrap()
    }
}

/// Challenge, sign, register. Returns the status and the parsed body.
pub async fn register(
    client: &Client,
    info: SocketAddr,
    host_did: &str,
    node: &Identity,
    token: &CapabilityToken,
) -> Result<Result<RegisterResponse, (StatusCode, ErrorBody)>> {
    let ch_url = format!("https://{info}{}", syneroym_passport::wire::CHALLENGE_PATH);
    let reg_url = format!("https://{info}{}", syneroym_passport::wire::REGISTER_PATH);

    let ch_res = client.post(&ch_url).send().await?;
    if !ch_res.status().is_success() {
        let status = ch_res.status();
        let body: ErrorBody = ch_res.json().await?;
        return Ok(Err((status, body)));
    }
    let challenge_resp: ChallengeResponse = ch_res.json().await?;
    let challenge = Challenge::from_hex(&challenge_resp.challenge)?;
    let signature =
        syneroym_passport::sign_challenge(node, host_did, &RELAY_HOME, &challenge, token)?;
    let signer_did = substrate::derive_did_key(&node.public_key());
    let challenge_hex = challenge.to_hex();
    let req =
        RegisterRequest { token: token.clone(), challenge: challenge_hex, signer_did, signature };
    let body_bytes = serde_json::to_vec(&req)?;

    let reg_res = client.post(&reg_url).body(body_bytes).send().await?;
    if !reg_res.status().is_success() {
        let status = reg_res.status();
        let body: ErrorBody = reg_res.json().await?;
        return Ok(Err((status, body)));
    }
    let resp: RegisterResponse = reg_res.json().await?;
    Ok(Ok(resp))
}

/// An Iroh endpoint with `node`'s key and only `relay` in its relay map.
/// It accepts no connections.
pub async fn node_endpoint(node: &Identity, relay: RelayUrl) -> Result<Endpoint> {
    let secret_key = SecretKey::from_bytes(&node.to_bytes());
    let endpoint = Endpoint::empty_builder()
        .secret_key(secret_key)
        .relay_mode(RelayMode::Custom(RelayMap::from(relay)))
        .bind()
        .await?;
    Ok(endpoint)
}

/// Whether `ep` goes online within `wait`.
pub async fn goes_online(ep: &Endpoint, wait: Duration) -> bool {
    tokio::time::timeout(wait, ep.online()).await.is_ok()
}

/// Like `node_endpoint`, but behind an `iroh::protocol::Router` that
/// accepts `TEST_ALPN` and answers each bidirectional stream with the bytes
/// it read (an echo). Keep the `Router` alive for as long as the node must
/// accept; `router.endpoint()` is the endpoint.
pub async fn accepting_node(node: &Identity, relay: RelayUrl) -> Result<Router> {
    let secret_key = SecretKey::from_bytes(&node.to_bytes());
    let endpoint = Endpoint::empty_builder()
        .secret_key(secret_key)
        .relay_mode(RelayMode::Custom(RelayMap::from(relay)))
        .bind()
        .await?;
    let router = Router::builder(endpoint).accept(TEST_ALPN, super::Echo).spawn();
    Ok(router)
}

/// From `from`, connect to `to` (its id plus only the relay URL, so the
/// path must go through the relay), open a stream on `TEST_ALPN`, send a
/// few bytes and read them back within `wait`. True when the echo arrives.
pub async fn echo_through_relay(
    from: &Endpoint,
    to: EndpointId,
    relay: RelayUrl,
    wait: Duration,
) -> bool {
    let addr = EndpointAddr::from_parts(to, [TransportAddr::Relay(relay)]);
    let fut = async {
        let conn = from.connect(addr, TEST_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"ping").await?;
        send.finish()?;
        let mut buf = [0u8; 4];
        recv.read_exact(&mut buf).await?;
        ensure!(&buf == b"ping", "mismatched echo");
        Ok::<(), anyhow::Error>(())
    };
    tokio::time::timeout(wait, fut).await.is_ok_and(|res| res.is_ok())
}

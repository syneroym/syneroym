use std::sync::{
    Arc, RwLock,
    atomic::{AtomicUsize, Ordering},
};

use dashmap::DashMap;
use iroh::Endpoint;
use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
use syneroym_core::{
    config::IdentityMode,
    dht_registry::{EndpointMechanism, SignedEndpointInfo},
    protocol_utils::{TargetHost, parse_target_host},
    util::short_hash,
};
use syneroym_identity::Identity;
use syneroym_router::SYNEROYM_ALPN;
use syneroym_sdk::topology::{
    AppHostResolver, CredentialWarning, Tier1Lookup, Tier2Fetch, credential_warning,
};

use super::*;

#[test]
fn parsing_an_unscoped_host_yields_the_expected_alias_and_interface() {
    let sh = short_hash("did:key:zSvc");
    let ih = short_hash("default");
    let host = format!("my-svc-s{sh}-i{ih}.localhost");

    let target = parse_target_host(&host).unwrap();
    let TargetHost::Service { lookup_alias, interface } = target else {
        panic!("expected a Service target");
    };
    assert_eq!(lookup_alias, format!("my-svc-{sh}"));
    assert_eq!(interface, ih);
}

#[derive(Debug)]
struct UnreachableTier1;

#[async_trait::async_trait]
impl Tier1Lookup for UnreachableTier1 {
    async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
        Err(anyhow::anyhow!("Tier1Lookup::lookup must not be called here"))
    }
}

fn test_state(fetcher: Option<Box<dyn Tier2Fetch>>) -> GatewayState {
    let identity = Identity::generate().unwrap();
    GatewayState {
        registry_url: String::new(),
        clients: DashMap::new(),
        identity,
        app_host_resolver: AppHostResolver::new(
            Box::new(UnreachableTier1),
            fetcher,
            LogicalResolver::new(Arc::new(StaticInventory::new())),
        ),
        identity_mode: IdentityMode::Open,
        fixed_identity_did: None,
        fixed_delegation: None,
        connection_auth_gate: false,
        auth_service_did: Arc::new(RwLock::new(None)),
    }
}

#[tokio::test]
async fn resolve_target_passes_an_unscoped_host_through_unresolved() {
    let state = test_state(None);
    let target = TargetHost::Service {
        lookup_alias: "my-svc-alias".to_string(),
        interface: "some-interface-hash".to_string(),
    };

    let (service_id, interface) = resolve_target(&state, target, None).await.unwrap();
    assert_eq!(service_id, "my-svc-alias");
    assert_eq!(interface, "some-interface-hash");
}

#[tokio::test]
async fn resolve_target_routes_an_app_scoped_host_through_the_resolver_and_surfaces_its_error() {
    let state = test_state(None);
    let target = TargetHost::App {
        app_lookup_alias: "my-app-abcdefgh".to_string(),
        app_did_hash: "abcdefgh".to_string(),
        service_name_hash: "ijklmnop".to_string(),
        interface: String::new(),
    };

    let err = resolve_target(&state, target, None).await.unwrap_err();
    assert!(err.to_string().contains("no community registry configured"), "{err}");
}

#[test]
fn a_gateway_with_neither_credential_warns_at_init_naming_both_config_keys() {
    assert_eq!(
        credential_warning(false, false),
        Some(CredentialWarning::NeitherConfigured),
        "neither `resolve_ucan` nor the same-node gate: must warn"
    );
    assert_eq!(
        credential_warning(false, true),
        Some(CredentialWarning::OnlyTheSameNodeGate),
        "the same-node gate alone: a quieter debug, not a warning"
    );
    assert_eq!(credential_warning(true, false), None, "a resolve_ucan token: no warning");
    assert_eq!(credential_warning(true, true), None, "both configured: no warning");
}

async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept());
    (client.unwrap(), server.unwrap().0)
}

#[tokio::test]
async fn forward_to_service_returns_bad_gateway_on_failed_connect() {
    let state = test_state(None);
    let identity = Identity::generate().unwrap();
    let client = Arc::new(Mutex::new(SyneroymClient::new_with_identity(
        "non-existent-svc".to_string(),
        "".to_string(),
        identity,
    )));
    state.clients.insert("non-existent-svc".to_string(), client);

    let (mut client_stream, mut gw_stream) = tcp_pair().await;
    let forward_task = tokio::spawn(async move {
        forward_to_service(
            &mut gw_stream,
            &state,
            "non-existent-svc",
            "default",
            false,
            b"GET / HTTP/1.1\r\n\r\n",
        )
        .await
    });

    let mut buf = [0u8; 1024];
    let n = client_stream.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(response.contains("HTTP/1.1 502"), "must return 502 Bad Gateway: {response}");
    assert!(response.contains("Bad Gateway"));
    assert!(forward_task.await.unwrap().is_ok());
}

// Proves the gateway retry/replay and 502 paths; does not re-home to a new host
// because the mechanism-backed client has no registry URL (full re-homing is
// covered by the SDK e2e tests).
#[tokio::test]
async fn forward_to_service_retries_on_not_hosted_here() {
    let state = test_state(None);
    let server_ep =
        Endpoint::empty_builder().alpns(vec![SYNEROYM_ALPN.to_vec()]).bind().await.unwrap();
    let server_addr = server_ep.addr();

    let mechanisms = vec![EndpointMechanism::Iroh {
        endpoint_addr_bytes: serde_json::to_vec(&server_addr).unwrap(),
        relay_url: None,
    }];
    let client = Arc::new(Mutex::new(
        SyneroymClient::new_with_mechanisms("target-svc".to_string(), mechanisms)
            .with_registry_dht(false),
    ));
    state.clients.insert("target-svc".to_string(), client);

    let attempts = Arc::new(AtomicUsize::new(0));
    let server_attempts = Arc::clone(&attempts);

    let server_task = tokio::spawn(async move {
        let incoming = server_ep.accept().await.unwrap();
        let connecting = incoming.accept().unwrap();
        let conn = connecting.await.unwrap();

        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let attempt = server_attempts.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let _ = recv.read(&mut buf).await;
            if attempt == 0 {
                let _ = send
                    .write_all(
                        b"HTTP/1.1 421 Misdirected Request\r\nx-syneroym-refusal: not_hosted_here\r\n\r\n",
                    )
                    .await;
                let _ = send.finish();
            } else {
                let _ = send
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\nreplayed-backend")
                    .await;
                let _ = send.finish();
            }
        }
    });

    let (mut client_stream, mut gw_stream) = tcp_pair().await;
    let forward_task = tokio::spawn(async move {
        forward_to_service(
            &mut gw_stream,
            &state,
            "target-svc",
            "http",
            false,
            b"GET / HTTP/1.1\r\nHost: target-svc\r\n\r\n",
        )
        .await
    });

    client_stream.shutdown().await.unwrap();
    let mut buf = [0u8; 1024];
    let n = client_stream.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(response.contains("HTTP/1.1 200 OK"), "must return 200 OK: {response}");
    assert!(response.contains("replayed-backend"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2, "must have made 2 attempts");

    assert!(forward_task.await.unwrap().is_ok());
    server_task.abort();
}

#[tokio::test]
async fn forward_to_service_returns_bad_gateway_when_retry_also_refused() {
    let state = test_state(None);
    let server_ep =
        Endpoint::empty_builder().alpns(vec![SYNEROYM_ALPN.to_vec()]).bind().await.unwrap();
    let server_addr = server_ep.addr();

    let mechanisms = vec![EndpointMechanism::Iroh {
        endpoint_addr_bytes: serde_json::to_vec(&server_addr).unwrap(),
        relay_url: None,
    }];
    let client = Arc::new(Mutex::new(
        SyneroymClient::new_with_mechanisms("target-svc-2".to_string(), mechanisms)
            .with_registry_dht(false),
    ));
    state.clients.insert("target-svc-2".to_string(), client);

    let attempts = Arc::new(AtomicUsize::new(0));
    let server_attempts = Arc::clone(&attempts);

    let server_task = tokio::spawn(async move {
        let incoming = server_ep.accept().await.unwrap();
        let connecting = incoming.accept().unwrap();
        let conn = connecting.await.unwrap();

        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let _ = server_attempts.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let _ = recv.read(&mut buf).await;
            let _ = send
                .write_all(
                    b"HTTP/1.1 421 Misdirected Request\r\nx-syneroym-refusal: not_hosted_here\r\n\r\n",
                )
                .await;
            let _ = send.finish();
        }
    });

    let (mut client_stream, mut gw_stream) = tcp_pair().await;
    let forward_task = tokio::spawn(async move {
        forward_to_service(
            &mut gw_stream,
            &state,
            "target-svc-2",
            "http",
            false,
            b"GET / HTTP/1.1\r\nHost: target-svc-2\r\n\r\n",
        )
        .await
    });

    client_stream.shutdown().await.unwrap();
    let mut buf = [0u8; 1024];
    let n = client_stream.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(response.contains("HTTP/1.1 502"), "must return 502 Bad Gateway: {response}");
    assert!(response.contains("Bad Gateway"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2, "must have made 2 attempts");

    assert!(forward_task.await.unwrap().is_ok());
    server_task.abort();
}

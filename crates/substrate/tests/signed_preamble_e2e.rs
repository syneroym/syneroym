#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for signed route preambles, caller
//! authentication, refusal on unhosted services, and SDK/gateway re-homing.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use common::SubstrateNode;
use ed25519_dalek::VerifyingKey;
use futures::future::join_all;
use iroh::{EndpointAddr, TransportAddr};
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{
    config::MailboxRole,
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient,
        SignedEndpointInfo,
    },
    mailbox::{
        MAILBOX_INTERFACE, METHOD_ACK, METHOD_CLAIM, METHOD_DEPOSIT, METHOD_REGISTER,
        METHOD_STATUS, METHOD_UNREGISTER,
    },
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport, SYNEROYM_ALPN, net_iroh};
use syneroym_rpc::{JsonRpcError, STREAM_UNAUTHENTICATED_RPC_CODE, framing};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, PassthroughOutcome, ServiceConfig, ServiceType,
    SyneroymClient, TcpManifest, Visibility,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time,
};

mod common;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs()
}

fn far_future_not_after() -> u64 {
    now_secs().saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

async fn wait_for_node_addr(node: &SubstrateNode) -> EndpointAddr {
    let deadline = time::Instant::now() + Duration::from_secs(10);
    while time::Instant::now() < deadline {
        let addr = node.home_relay().endpoint_addr();
        let ips: Vec<TransportAddr> =
            addr.addrs.iter().filter(|a| matches!(a, TransportAddr::Ip(_))).cloned().collect();
        if !ips.is_empty() {
            return EndpointAddr::from_parts(addr.id, ips);
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    node.home_relay().endpoint_addr()
}

async fn spawn_echo_backend(port: u16, backend_id: u32) -> tokio::task::JoinHandle<()> {
    common::release_held_ports(&[port]);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind echo backend");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                while let Ok(frame) = framing::read_frame(&mut socket).await {
                    let req: Value = serde_json::from_slice(&frame).unwrap_or_default();
                    let id = req.get("id").cloned().unwrap_or(json!(1));
                    let params = req.get("params").cloned().unwrap_or(json!({}));
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "backend": backend_id,
                            "echo": params
                        },
                        "id": id
                    });
                    let bytes = serde_json::to_vec(&resp).unwrap();
                    if framing::write_frame(&mut socket, &bytes).await.is_err() {
                        break;
                    }
                }
            });
        }
    })
}

async fn spawn_http_backend(port: u16, backend_id: u32) -> tokio::task::JoinHandle<()> {
    common::release_held_ports(&[port]);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind http backend");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                if let Ok(n) = socket.read(&mut buf).await
                    && n > 0
                {
                    let body = format!("HTTP backend {backend_id} response");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: \
                         {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    })
}

async fn spawn_slow_large_http_backend(
    port: u16,
    delay: Duration,
    body_len: usize,
) -> tokio::task::JoinHandle<()> {
    common::release_held_ports(&[port]);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind slow http backend");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                if let Ok(n) = socket.read(&mut buf).await
                    && n > 0
                {
                    time::sleep(delay).await;
                    let body = "a".repeat(body_len);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: \
                         {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    })
}

async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind pair listener");
    let addr = listener.local_addr().expect("local addr");
    let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept(),);
    (client.expect("connect client"), server.expect("accept server").0)
}

fn bare_tcp_manifest(
    port: u16,
    registry_certificate: Option<String>,
    instance_certificate: Option<String>,
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
            visibility: Some(Visibility::Internal),
        },
        service_type: ServiceType::Tcp(TcpManifest {
            endpoints: vec![
                NetworkEndpoint {
                    interface_name: "default".to_string(),
                    host: "127.0.0.1".to_string(),
                    port,
                },
                NetworkEndpoint {
                    interface_name: "http".to_string(),
                    host: "127.0.0.1".to_string(),
                    port,
                },
            ],
        }),
        registry_certificate,
        instance_certificate,
        mailbox: None,
    }
}

async fn setup_two_nodes(operator: &Identity) -> (SubstrateNode, SubstrateNode) {
    let node1 = SubstrateNode::builder().owner(operator).inject_kek_bytes([0x77; 32]).boot().await;

    let node2 = SubstrateNode::builder()
        .owner(operator)
        .shared_registry(node1.registry_url())
        .shared_relay(node1.relay_url())
        .inject_kek_bytes([0x77; 32])
        .boot()
        .await;

    (node1, node2)
}

async fn deploy_target_service(
    node: &SubstrateNode,
    operator: &Identity,
    member_master: &Identity,
    backend_port: u16,
) -> SignedEndpointInfo {
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let mut op_client = node.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client.connect().await.expect("connect operator client");

    let instance_info =
        op_client.instance_identity(&member_master_did).await.expect("instance identity query");
    let pubkey_bytes: [u8; 32] = hex::decode(&instance_info.pubkey_hex)
        .expect("hex decode pubkey")
        .try_into()
        .expect("32 bytes pubkey");
    let pubkey = VerifyingKey::from_bytes(&pubkey_bytes).expect("verifying key");

    let cert = DelegationCertificate::issue(
        member_master,
        pubkey,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .expect("issue delegation cert");

    let record = EndpointInfo {
        service_id: member_master_did.clone(),
        substrate_id: node.did().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: true,
        ttl: None,
        not_after: far_future_not_after(),
        generation: 0,
        mailbox: None,
    }
    .sign(member_master)
    .expect("sign endpoint record");

    let manifest = bare_tcp_manifest(
        backend_port,
        Some(serde_json::to_string(&record).expect("serialize record")),
        Some(cert.to_json().expect("serialize cert")),
    );

    common::try_deploy_app(&op_client, &member_master_did, manifest).await.expect("deploy app");

    record
}

#[tokio::test]
async fn mailbox_refuses_forged_preamble_signatures_at_handshake() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let owner = Identity::generate()?;
    let node = SubstrateNode::builder()
        .owner(&owner)
        .configure(|c| {
            c.roles.mailbox = Some(MailboxRole {
                visibility_secs: 2,
                sweep_interval_secs: 1,
                republish_interval_secs: 2,
                ..Default::default()
            });
        })
        .boot()
        .await;

    let methods = [
        METHOD_REGISTER,
        METHOD_UNREGISTER,
        METHOD_DEPOSIT,
        METHOD_CLAIM,
        METHOD_ACK,
        METHOD_STATUS,
    ];

    let victim_master = Identity::generate()?;
    let victim_master_did = substrate::derive_did_key(&victim_master.public_key());
    let reg_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&victim_master_did, vec![], None, &victim_master, true)
        .await?;

    let victim_instance = Identity::generate()?;
    let cert = DelegationCertificate::issue(
        &victim_master,
        victim_instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )?;

    let attacker = Identity::generate()?;
    let client_ep = net_iroh::empty_endpoint_builder().bind().await?;
    let node_addr = wait_for_node_addr(&node).await;

    for method in methods {
        let conn = client_ep.connect(node_addr.clone(), SYNEROYM_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;

        let mut preamble = RoutePreamble::new(
            RouteTransport::Binary,
            RouteProtocol::JsonRpc,
            node.did(),
            MAILBOX_INTERFACE,
        );
        preamble.sign(&attacker, now_secs());
        preamble.pubkey = Some(hex::encode(victim_instance.public_key().to_bytes()));
        preamble.delegation = Some(cert.clone());

        send.write_all(preamble.to_preamble_line().as_bytes()).await?;

        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": {},
            "id": 1
        });
        framing::write_frame(&mut send, &serde_json::to_vec(&req)?).await?;

        let frame = framing::read_frame(&mut recv).await?;
        let resp: Value = serde_json::from_slice(&frame)?;
        assert_eq!(
            resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE,
            "method {method} must be refused with STREAM_UNAUTHENTICATED_RPC_CODE when forged"
        );
    }

    // Control case: the real key holder's status request is not refused by the
    // router.
    {
        let conn = client_ep.connect(node_addr.clone(), SYNEROYM_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;

        let mut preamble = RoutePreamble::new(
            RouteTransport::Binary,
            RouteProtocol::JsonRpc,
            node.did(),
            MAILBOX_INTERFACE,
        );
        preamble.sign(&victim_instance, now_secs());
        preamble.delegation = Some(cert);

        send.write_all(preamble.to_preamble_line().as_bytes()).await?;

        let req = json!({
            "jsonrpc": "2.0",
            "method": METHOD_STATUS,
            "params": {},
            "id": 2
        });
        framing::write_frame(&mut send, &serde_json::to_vec(&req)?).await?;

        let frame = framing::read_frame(&mut recv).await?;
        let resp: Value = serde_json::from_slice(&frame)?;
        assert_ne!(
            resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE,
            "real key holder must not be refused with STREAM_UNAUTHENTICATED_RPC_CODE: {resp:?}"
        );
    }

    client_ep.close().await;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn sdk_rehomes_when_target_service_moves_between_nodes() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port1, port2] = common::alloc_ports::<2>();
    let _backend1 = spawn_echo_backend(port1, 1).await;
    let _backend2 = spawn_echo_backend(port2, 2).await;

    let operator = Identity::generate()?;
    let (node1, node2) = setup_two_nodes(&operator).await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node1.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record1 = deploy_target_service(&node1, &operator, &member_master, port1).await;

    let mut client =
        SyneroymClient::new(member_master_did.clone(), node1.registry_url().to_string())
            .with_registry_dht(false);
    client.connect().await?;

    let res1 = client.request("default", "echo", json!({"step": 1})).await?;
    assert_eq!(res1.result["backend"], 1);
    assert_eq!(res1.result["echo"]["step"], 1);

    let mut op_client1 = node1.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client1.connect().await?;
    op_client1.undeploy(member_master_did.clone(), 0).await?;

    let _record2 = deploy_target_service(&node2, &operator, &member_master, port2).await;

    let res2 = client.request("default", "echo", json!({"step": 2})).await?;
    assert_eq!(res2.result["backend"], 2);
    assert_eq!(res2.result["echo"]["step"], 2);

    let _ = client.shutdown().await;
    node1.teardown().await;
    node2.teardown().await;
    Ok(())
}

#[tokio::test]
async fn sdk_rehomes_large_request_without_stream_hang() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port1, port2] = common::alloc_ports::<2>();
    let _backend1 = spawn_echo_backend(port1, 1).await;
    let _backend2 = spawn_echo_backend(port2, 2).await;

    let operator = Identity::generate()?;
    let (node1, node2) = setup_two_nodes(&operator).await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node1.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record1 = deploy_target_service(&node1, &operator, &member_master, port1).await;

    let mut client =
        SyneroymClient::new(member_master_did.clone(), node1.registry_url().to_string())
            .with_registry_dht(false);
    client.connect().await?;

    let res1 = client.request("default", "echo", json!({"step": 1})).await?;
    assert_eq!(res1.result["backend"], 1);

    let mut op_client1 = node1.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client1.connect().await?;
    op_client1.undeploy(member_master_did.clone(), 0).await?;

    let _record2 = deploy_target_service(&node2, &operator, &member_master, port2).await;

    let large_payload = "x".repeat(4 * 1024 * 1024);
    let res2 = client.request("default", "echo", json!({"payload": large_payload})).await?;
    assert_eq!(res2.result["backend"], 2);
    assert_eq!(res2.result["echo"]["payload"].as_str().map(|s| s.len()), Some(4 * 1024 * 1024));

    let _ = client.shutdown().await;
    node1.teardown().await;
    node2.teardown().await;
    Ok(())
}

#[tokio::test]
async fn concurrent_requests_rehome_coalesced_when_service_moves() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port1, port2] = common::alloc_ports::<2>();
    let _backend1 = spawn_echo_backend(port1, 1).await;
    let _backend2 = spawn_echo_backend(port2, 2).await;

    let operator = Identity::generate()?;
    let (node1, node2) = setup_two_nodes(&operator).await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node1.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record1 = deploy_target_service(&node1, &operator, &member_master, port1).await;

    let mut client =
        SyneroymClient::new(member_master_did.clone(), node1.registry_url().to_string())
            .with_registry_dht(false);
    client.connect().await?;

    let res1 = client.request("default", "echo", json!({"step": 1})).await?;
    assert_eq!(res1.result["backend"], 1);

    let mut op_client1 = node1.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client1.connect().await?;
    op_client1.undeploy(member_master_did.clone(), 0).await?;

    let _record2 = deploy_target_service(&node2, &operator, &member_master, port2).await;

    let initial_stable_id = client.current_stable_id().expect("initial stable id");

    let client = Arc::new(client);
    let handles: Vec<_> = (0..8)
        .map(|idx| {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("default", "echo", json!({"req_idx": idx})).await })
        })
        .collect();

    let results = join_all(handles).await;
    for (idx, join_res) in results.into_iter().enumerate() {
        let rpc_res = join_res.expect("task panicked")?;
        assert_eq!(rpc_res.result["backend"], 2);
        assert_eq!(rpc_res.result["echo"]["req_idx"], idx);
    }

    assert!(client.connection().is_some(), "one live connection remains");
    let final_stable_id = client.current_stable_id().expect("final stable id");
    assert_ne!(final_stable_id, initial_stable_id, "connection moved to new host");
    assert_eq!(client.rehome_count(), 1, "exactly one coalesced rehome occurred");

    drop(client);
    node1.teardown().await;
    node2.teardown().await;
    Ok(())
}

#[tokio::test]
async fn gateway_passthrough_rehomes_on_target_relocation() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port1, port2] = common::alloc_ports::<2>();
    let _backend1 = spawn_http_backend(port1, 1).await;
    let _backend2 = spawn_http_backend(port2, 2).await;

    let operator = Identity::generate()?;
    let (node1, node2) = setup_two_nodes(&operator).await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node1.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record1 = deploy_target_service(&node1, &operator, &member_master, port1).await;

    let mut client =
        SyneroymClient::new(member_master_did.clone(), node1.registry_url().to_string())
            .with_registry_dht(false);
    client.connect().await?;

    let conn1 = client.connection().expect("connection 1");
    let caller_id = Identity::generate()?;

    let (mut client_stream, mut gw_stream) = tcp_pair().await;
    let http_req = b"GET / HTTP/1.1\r\nHost: target\r\n\r\n";

    let passthrough_task = tokio::spawn({
        let conn = conn1.clone();
        let target_did = member_master_did.clone();
        let caller = Identity::from_bytes(&caller_id.to_bytes());
        async move {
            SyneroymClient::passthrough_with_conn(
                conn,
                &target_did,
                "http",
                http_req,
                &mut gw_stream,
                &caller,
                None,
            )
            .await
        }
    });

    client_stream.shutdown().await?;
    let mut client_buf = [0u8; 1024];
    let n = client_stream.read(&mut client_buf).await?;
    let outcome1 = passthrough_task.await??;
    assert!(matches!(outcome1, PassthroughOutcome::Done));
    assert!(String::from_utf8_lossy(&client_buf[..n]).contains("HTTP backend 1 response"));

    let mut op_client1 = node1.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client1.connect().await?;
    op_client1.undeploy(member_master_did.clone(), 0).await?;

    let _record2 = deploy_target_service(&node2, &operator, &member_master, port2).await;

    let client = Arc::new(client);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let conn1 = conn1.clone();
            let client = Arc::clone(&client);
            let target_did = member_master_did.clone();
            let caller = Identity::from_bytes(&caller_id.to_bytes());
            tokio::spawn(async move {
                let (mut client_stream, mut gw_stream) = tcp_pair().await;
                client_stream.shutdown().await.unwrap();
                let outcome = SyneroymClient::passthrough_with_conn(
                    conn1.clone(),
                    &target_did,
                    "http",
                    http_req,
                    &mut gw_stream,
                    &caller,
                    None,
                )
                .await
                .unwrap();
                assert!(matches!(outcome, PassthroughOutcome::NotHostedHere { replayable: true }));

                client.rehome(&conn1).await.unwrap();
                let conn2 = client.connection().unwrap();

                let (mut client_stream2, mut gw_stream2) = tcp_pair().await;
                client_stream2.shutdown().await.unwrap();
                let outcome2 = SyneroymClient::passthrough_with_conn(
                    conn2,
                    &target_did,
                    "http",
                    http_req,
                    &mut gw_stream2,
                    &caller,
                    None,
                )
                .await
                .unwrap();
                assert!(matches!(outcome2, PassthroughOutcome::Done));
                let mut buf = [0u8; 1024];
                let n = client_stream2.read(&mut buf).await.unwrap();
                assert!(String::from_utf8_lossy(&buf[..n]).contains("HTTP backend 2 response"));
            })
        })
        .collect();

    for h in join_all(handles).await {
        h.expect("task panicked");
    }

    node1.teardown().await;
    node2.teardown().await;
    Ok(())
}

#[tokio::test]
async fn client_without_registry_receives_readable_not_hosted_here_error() -> anyhow::Result<()> {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port1, port2] = common::alloc_ports::<2>();
    let _backend1 = spawn_echo_backend(port1, 1).await;
    let _backend2 = spawn_echo_backend(port2, 2).await;

    let operator = Identity::generate()?;
    let (node1, node2) = setup_two_nodes(&operator).await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node1.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record1 = deploy_target_service(&node1, &operator, &member_master, port1).await;
    let node1_mechanisms = reg_client.lookup(&member_master_did, true).await?.info.mechanisms;

    let mut op_client1 = node1.client_as(Identity::from_bytes(&operator.to_bytes()));
    op_client1.connect().await?;
    op_client1.undeploy(member_master_did.clone(), 0).await?;

    let _record2 = deploy_target_service(&node2, &operator, &member_master, port2).await;

    let mut client =
        SyneroymClient::new_with_mechanisms(member_master_did.clone(), node1_mechanisms)
            .with_host(node1.did());
    client.connect().await?;

    let err1 = client.request("default", "echo", json!({"step": 1})).await.unwrap_err();
    let rpc_err1 = err1.downcast_ref::<JsonRpcError>().expect("jsonrpc error");
    assert_eq!(rpc_err1.code, -32096);

    let large_payload = "x".repeat(4 * 1024 * 1024);
    let err2 =
        client.request("default", "echo", json!({"payload": large_payload})).await.unwrap_err();
    let rpc_err2 = err2.downcast_ref::<JsonRpcError>().expect("jsonrpc error");
    assert_eq!(rpc_err2.code, -32096);

    let _ = client.shutdown().await;
    node1.teardown().await;
    node2.teardown().await;
    Ok(())
}

#[tokio::test]
async fn gateway_passthrough_slow_backend_and_large_response_after_half_close() -> anyhow::Result<()>
{
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let [port] = common::alloc_ports::<1>();
    let _backend =
        spawn_slow_large_http_backend(port, Duration::from_millis(2500), 16 * 1024).await;

    let operator = Identity::generate()?;
    let node = SubstrateNode::builder().owner(&operator).boot().await;

    let member_master = Identity::generate()?;
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let reg_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    reg_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await?;

    let _record = deploy_target_service(&node, &operator, &member_master, port).await;

    let mut client =
        SyneroymClient::new(member_master_did.clone(), node.registry_url().to_string())
            .with_registry_dht(false);
    client.connect().await?;
    let conn = client.connection().expect("connection");
    let caller_id = Identity::generate()?;

    let (mut client_stream, mut gw_stream) = tcp_pair().await;
    let http_req = b"GET / HTTP/1.1\r\nHost: target\r\n\r\n";

    let passthrough_task = tokio::spawn({
        let target_did = member_master_did.clone();
        let caller = Identity::from_bytes(&caller_id.to_bytes());
        async move {
            SyneroymClient::passthrough_with_conn(
                conn,
                &target_did,
                "http",
                http_req,
                &mut gw_stream,
                &caller,
                None,
            )
            .await
        }
    });

    client_stream.shutdown().await?;
    let mut response_buf = Vec::new();
    client_stream.read_to_end(&mut response_buf).await?;
    let outcome = passthrough_task.await??;
    assert!(matches!(outcome, PassthroughOutcome::Done));
    assert!(response_buf.len() > 16 * 1024, "must receive full >16 KiB response");
    assert!(String::from_utf8_lossy(&response_buf).contains("HTTP/1.1 200 OK"));

    let _ = client.shutdown().await;
    node.teardown().await;
    Ok(())
}

//! End-to-end multi-node tests for encrypted channel routing through
//! coordinators.
#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, endpoint::presets};
use reqwest::Client;
use syneroym_app_orchestration::Visibility;
use syneroym_core::{
    config::RetryPolicy, dht_registry::EndpointMechanism, local_registry::SubstrateEndpoint,
};
use syneroym_identity::{Identity, substrate};
use syneroym_router::{IrohHop, OutboundIdentity, RemoteHop, RemotePeer, RoutePreamble};
use syneroym_rpc::JsonRpcRequest;
use syneroym_sdk::{SyneroymClient, client::PassthroughOutcome, deploy::certify};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};

mod common;

use common::{
    logs::{forwarded_without_identity, init_trace_collector},
    nodes::{CapturingNativeService, InboundRelayTopology},
};

#[tokio::test]
async fn iroh_hop_through_forwarder() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let mut ep_bldr = Endpoint::builder(presets::Minimal);
    if let Some(relay_url) = topo.c_info.relay_url.as_ref() {
        let url: RelayUrl = relay_url.parse()?;
        ep_bldr = ep_bldr.relay_mode(RelayMode::Custom(RelayMap::from(url)));
    }
    let client_ep = ep_bldr.bind().await?;
    let policy = RetryPolicy::default();
    let hop = IrohHop::new(Some(client_ep.clone()), policy);

    let mut c_addr: EndpointAddr = serde_json::from_slice(&topo.c_info.endpoint_addr_bytes)?;
    if let Some(relay_url) = topo.c_info.relay_url.as_ref() {
        c_addr = c_addr.with_relay_url(relay_url.parse()?);
    }

    let target_z_key = substrate::resolve_did_key(&topo.target_z.did)?;
    let peer = RemotePeer { addr: c_addr, host_key: Some(target_z_key) };

    let route = RoutePreamble::binary_json_rpc(&topo.target_z.did, "orchestrator");
    let caller_identity = Identity::generate()?;
    let out_id = OutboundIdentity::key_only(Arc::new(caller_identity));
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "readyz".to_string(),
        params: serde_json::json!({}),
        id: Some(serde_json::json!(1)),
        idempotency_key: None,
    };

    let val = hop
        .call(&peer, &route, Some(&out_id), &request, Duration::from_secs(10))
        .await
        .map_err(|e| anyhow::anyhow!("IrohHop call failed: {e:?}"))?;

    assert_eq!(val, serde_json::json!({"status": "ok"}));
    forwarded_without_identity(&logs, &topo.target_z.did);

    client_ep.close().await;
    topo.shutdown().await
}

#[tokio::test]
async fn passthrough_through_forwarder() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let mut client = topo.new_client();
    client.connect().await?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let mut client_conn = TcpStream::connect(local_addr).await?;
    let (mut server_conn, _) = listener.accept().await?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "readyz",
        "params": {},
        "id": 1,
    })
    .to_string();
    let req = format!(
        "POST /orchestrator HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let client_task = tokio::spawn(async move {
        let mut total = Vec::new();
        let mut buf = vec![0u8; 4096];
        loop {
            let n = client_conn.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buf[..n]);
        }
        Ok::<_, anyhow::Error>(String::from_utf8_lossy(&total).to_string())
    });

    let outcome =
        client.passthrough("orchestrator", req.as_bytes(), &mut server_conn, None).await?;
    assert!(matches!(outcome, PassthroughOutcome::Done));

    let resp_str = client_task.await??;
    assert!(resp_str.contains("200 OK"), "expected 200 OK, got: {resp_str}");
    assert!(resp_str.contains("\"status\":\"ok\""), "expected status ok, got: {resp_str}");
    forwarded_without_identity(&logs, &topo.target_z.did);

    client.shutdown().await?;
    topo.shutdown().await
}

#[tokio::test]
async fn service_target_through_forwarder_looks_up_the_host() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let member_identity = Identity::generate()?;
    let member_did = substrate::derive_did_key(&member_identity.public_key());
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let not_after = now + 3600;

    let record_json = certify::member_registry_record(
        Visibility::Public,
        &member_did,
        &topo.target_z.did,
        &member_identity,
        not_after,
        None,
    )?
    .expect("member registry record");

    let reg_res = info_client
        .post(format!("{}/register", topo.r_url))
        .header("Content-Type", "application/json")
        .body(record_json)
        .send()
        .await?;
    assert!(reg_res.status().is_success());

    topo.target_z
        .route_handler
        .registry()
        .register(
            member_did.clone(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: member_did.clone() },
        )
        .await?;
    topo.target_z
        .route_handler
        .register_native_service(member_did.clone(), Arc::new(CapturingNativeService::default()));

    let caller_identity = Identity::generate()?;
    let mut client1 =
        SyneroymClient::new_with_identity(member_did.clone(), topo.r_url.clone(), caller_identity);
    client1
        .connect_with_mechanisms(vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: topo.c_info.endpoint_addr_bytes.clone(),
            relay_url: topo.c_info.relay_url.clone(),
        }])
        .await?;

    let response = client1.request("default", "test_method", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    let mut client2 = SyneroymClient::new_with_mechanisms(
        member_did.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: topo.c_info.endpoint_addr_bytes.clone(),
            relay_url: topo.c_info.relay_url.clone(),
        }],
    );
    client2.connect().await?;
    let err = client2.request("default", "test_method", serde_json::json!({})).await.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("did not prove it is host") || err_str.contains("bad handshake signature"),
        "expected bad host signature error, got: {err_str}"
    );
    assert!(err_str.contains("with_host"), "error must mention with_host: {err_str}");

    forwarded_without_identity(&logs, &member_did);

    client1.shutdown().await?;
    client2.shutdown().await?;
    topo.shutdown().await
}

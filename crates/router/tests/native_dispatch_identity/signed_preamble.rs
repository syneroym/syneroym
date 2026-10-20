use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_identity::{Identity, substrate};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_rpc::{
    AuthLevel, CallerContext, NOT_HOSTED_HERE_RPC_CODE, NativeInvocation, NativeResponse,
    NativeService, RpcResult, STREAM_UNAUTHENTICATED_RPC_CODE, framing,
};
use tokio::io::AsyncWriteExt;

use super::helpers::*;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs()
}

async fn register_test_endpoint(route_handler: &RouteHandler, service_id: &str) {
    route_handler
        .registry()
        .register(
            service_id.to_string(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() },
        )
        .await
        .unwrap();
}

async fn read_refusal_response(client: &mut tokio::io::DuplexStream) -> Value {
    let frame = framing::read_frame(client).await.expect("read refusal frame failed");
    serde_json::from_slice(&frame).expect("expected JSON-RPC response")
}

#[derive(Debug, Default)]
struct CapturingNativeService {
    caller: Mutex<Option<CallerContext>>,
}

#[async_trait::async_trait]
impl NativeService for CapturingNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        *self.caller.lock().unwrap() = Some(invocation.caller);
        Ok(NativeResponse { payload: json!({"status": "ok"}) })
    }
}

#[tokio::test]
async fn valid_signed_preamble_is_accepted_and_caller_authenticated() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-hosted-service";
    route_handler.register_native_service(service_id.to_string(), service.clone());
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs());

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let req = json!({
        "jsonrpc": "2.0",
        "method": "ping",
        "params": null,
        "id": 1
    });
    let req_bytes = serde_json::to_vec(&req).unwrap();
    framing::write_frame(&mut client, &req_bytes).await.unwrap();

    let resp_frame = framing::read_frame(&mut client).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let expected_did = substrate::derive_did_key(&identity.public_key());
    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, expected_did);
    assert_eq!(captured.auth, AuthLevel::Delegated);
    assert!(captured.proof.is_some());

    drop(client);
    let _ = handler_task.await;
}

#[tokio::test]
async fn unknown_service_returns_not_hosted_here_refusal() {
    let (route_handler, _) = test_route_handler().await;
    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "nonexistent-service",
        "default",
    );
    preamble.sign(&identity, now_secs());

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], NOT_HOSTED_HERE_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn expired_preamble_timestamp_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs() - 1000);

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn future_preamble_timestamp_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs() + 1000);

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn invalid_signature_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let now = now_secs();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now);
    preamble.sig = Some("00".repeat(64));

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn missing_signature_when_pubkey_present_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.pubkey = Some(hex::encode(identity.public_key().to_bytes()));
    preamble.sig = None;

    let (mut client, server) = tokio::io::duplex(4096);
    let handler_task = tokio::spawn(route_handler.handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

use std::sync::Arc;

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_router::{
    RoutePreamble, RouteProtocol, RouteTransport,
    e2e_channel::{Opened, Path, open_stream},
};
use syneroym_rpc::{AuthLevel, framing};
use tokio::io::duplex;

use super::helpers::*;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_secs()
}

#[tokio::test]
async fn encrypted_identity_delivered_to_native_capability() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-encrypted-service";
    route_handler.register_native_service(service_id.to_string(), service.clone());
    route_handler
        .registry()
        .register(
            service_id.to_string(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() },
        )
        .await
        .unwrap();

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = tokio::io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());
    let opened =
        open_stream(client_read, client_write, preamble, &path).await.expect("open_stream failed");

    let mut stream = match opened {
        Opened::Ready(s) => s,
        Opened::Refused(_) => panic!("stream refused unexpectedly"),
    };

    let req = json!({
        "jsonrpc": "2.0",
        "method": "ping",
        "params": null,
        "id": 1
    });
    let req_bytes = serde_json::to_vec(&req).unwrap();
    framing::write_frame(&mut stream, &req_bytes).await.unwrap();

    let resp_frame = framing::read_frame(&mut stream).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let expected_did = substrate::derive_did_key(&caller.public_key());
    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, expected_did);
    assert_eq!(captured.auth, AuthLevel::Delegated);
    assert!(captured.proof.is_some());

    drop(stream);
    let _ = handler_task.await;
}

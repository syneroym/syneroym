use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hkdf::Hkdf;
use p256::{
    EncodedPoint, PublicKey,
    ecdh::EphemeralSecret,
    elliptic_curve::rand_core::{OsRng, RngCore},
};
use serde_json::{Value, json};
use sha2::Sha256;
use syneroym_identity::{Identity, substrate};
use syneroym_router::{
    E2eError, RoutePreamble, RouteProtocol, RouteTransport,
    e2e_channel::{Opened, Path, open_stream},
};
use syneroym_rpc::{
    JsonRpcErrorResponse, NOT_HOSTED_HERE_RPC_CODE, STREAM_UNAUTHENTICATED_RPC_CODE, framing,
};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, duplex},
    task, time,
};

use super::helpers::*;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs()
}

async fn register_test_channel(
    route_handler: &RouteHandler,
    service_id: &str,
    service: Arc<CapturingNativeService>,
) {
    route_handler.register_native_service(service_id.to_string(), service);
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

enum PreambleRewrite {
    None,
    StripEncAndEpk,
    ReplaceEpk(String),
}

async fn run_spy_forwarder(
    mut client_io: DuplexStream,
    mut router_io: DuplexStream,
    recorded: Arc<Mutex<Vec<u8>>>,
    rewrite: PreambleRewrite,
) {
    let mut first_line_bytes = Vec::new();
    let mut b = [0u8; 1];
    while let Ok(1) = client_io.read(&mut b).await {
        first_line_bytes.push(b[0]);
        if b[0] == b'\n' {
            break;
        }
    }
    recorded.lock().unwrap().extend_from_slice(&first_line_bytes);
    let line = String::from_utf8_lossy(&first_line_bytes).to_string();

    let modified = match rewrite {
        PreambleRewrite::None => line,
        PreambleRewrite::StripEncAndEpk => {
            let kept: Vec<_> = line
                .trim_end()
                .split('&')
                .filter(|p| !p.starts_with("enc=") && !p.starts_with("epk="))
                .collect();
            format!("{}\n", kept.join("&"))
        }
        PreambleRewrite::ReplaceEpk(new_epk) => {
            let parts: Vec<_> =
                line.trim_end()
                    .split('&')
                    .map(|p| {
                        if p.starts_with("epk=") { format!("epk={new_epk}") } else { p.to_string() }
                    })
                    .collect();
            format!("{}\n", parts.join("&"))
        }
    };

    if router_io.write_all(modified.as_bytes()).await.is_err() || router_io.flush().await.is_err() {
        return;
    }

    let (mut cr, mut cw) = io::split(client_io);
    let (mut rr, mut rw) = io::split(router_io);
    let rec = recorded.clone();
    let c2r = async move {
        let mut buf = [0u8; 1024];
        while let Ok(n) = cr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            rec.lock().unwrap().extend_from_slice(&buf[..n]);
            if rw.write_all(&buf[..n]).await.is_err() || rw.flush().await.is_err() {
                break;
            }
        }
        let _ = rw.shutdown().await;
    };
    let r2c = async move {
        let mut buf = [0u8; 1024];
        while let Ok(n) = rr.read(&mut buf).await {
            if n == 0 || cw.write_all(&buf[..n]).await.is_err() || cw.flush().await.is_err() {
                break;
            }
        }
        let _ = cw.shutdown().await;
    };
    tokio::join!(c2r, r2c);
}

struct ChannelKeys {
    c2s: [u8; 32],
    s2c: [u8; 32],
}

fn derive_test_channel_keys(
    secret: &EphemeralSecret,
    server_point_bytes: &[u8],
    client_epk_bytes: &[u8],
) -> anyhow::Result<ChannelKeys> {
    let server_point = PublicKey::from_sec1_bytes(server_point_bytes)?;
    let shared = secret.diffie_hellman(&server_point);
    let hkdf = Hkdf::<Sha256>::new(Some(b"syneroym-e2e-v1"), shared.raw_secret_bytes());

    let mut c2s_info = Vec::with_capacity(3 + 65 + 65);
    c2s_info.extend_from_slice(b"c2s");
    c2s_info.extend_from_slice(client_epk_bytes);
    c2s_info.extend_from_slice(server_point_bytes);

    let mut s2c_info = Vec::with_capacity(3 + 65 + 65);
    s2c_info.extend_from_slice(b"s2c");
    s2c_info.extend_from_slice(client_epk_bytes);
    s2c_info.extend_from_slice(server_point_bytes);

    let mut keys = ChannelKeys { c2s: [0u8; 32], s2c: [0u8; 32] };
    hkdf.expand(&c2s_info, &mut keys.c2s).map_err(|e| anyhow::anyhow!("{e}"))?;
    hkdf.expand(&s2c_info, &mut keys.s2c).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(keys)
}

fn encrypt_channel_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    use aes_gcm::{AeadInPlace, Aes256Gcm, KeyInit, Nonce};
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut in_out = plaintext.to_vec();
    cipher.encrypt_in_place(nonce, b"", &mut in_out).unwrap();
    let total_len = (12 + in_out.len()) as u16;
    let mut out = total_len.to_be_bytes().to_vec();
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&in_out);
    out
}

async fn read_and_decrypt_channel_chunk<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    key: &[u8; 32],
) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{AeadInPlace, Aes256Gcm, KeyInit, Nonce};
    let mut len_bytes = [0u8; 2];
    reader.read_exact(&mut len_bytes).await?;
    let len = u16::from_be_bytes(len_bytes) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(&payload[..12]);
    let mut in_out = payload[12..].to_vec();
    cipher
        .decrypt_in_place(nonce, b"", &mut in_out)
        .map_err(|e| anyhow::anyhow!("aes-gcm decryption failed: {e}"))?;
    Ok(in_out)
}

#[tokio::test]
async fn encrypted_identity_delivered_to_native_capability() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-encrypted-service";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = io::split(client_stream);
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
    framing::write_frame(&mut stream, &serde_json::to_vec(&req).unwrap()).await.unwrap();

    let resp_frame = framing::read_frame(&mut stream).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let expected_did = substrate::derive_did_key(&caller.public_key());
    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, expected_did);
    assert!(captured.proof.is_some());

    drop(stream);
    let _ = handler_task.await;
}

#[tokio::test]
async fn encrypted_identity_through_spy_forwarder_keeps_caller_key_out_of_stream() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-encrypted-service-spy";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();
    let caller_did = substrate::derive_did_key(&caller.public_key());
    let caller_hex = hex::encode(caller.public_key().as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, spy_client_side) = duplex(4096);
    let (spy_router_side, router_stream) = duplex(4096);

    let recorded_bytes = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(run_spy_forwarder(
        spy_client_side,
        spy_router_side,
        recorded_bytes.clone(),
        PreambleRewrite::None,
    ));
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(router_stream));

    let (client_read, client_write) = io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());
    let opened = open_stream(client_read, client_write, preamble, &path)
        .await
        .expect("open_stream should succeed");

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
    framing::write_frame(&mut stream, &serde_json::to_vec(&req).unwrap()).await.unwrap();

    let resp_frame = framing::read_frame(&mut stream).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, caller_did);
    assert!(captured.proof.is_some());

    drop(stream);
    let _ = handler_task.await;

    let recorded = recorded_bytes.lock().unwrap().clone();
    let recorded_text = String::from_utf8_lossy(&recorded);
    assert!(recorded_text.contains(service_id));
    assert!(recorded_text.contains("enc=ecdh-p256"));
    assert!(!recorded_text.contains("pubkey="), "recorded bytes leaked pubkey=: {recorded_text}");
    assert!(
        !recorded.windows(caller_hex.len()).any(|w| w == caller_hex.as_bytes()),
        "caller key hex leaked"
    );
}

#[tokio::test]
async fn direct_path_sends_identity_in_preamble_without_handshake() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-direct-service";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let caller = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = io::split(client_stream);
    let opened = open_stream(client_read, client_write, preamble, &Path::Direct)
        .await
        .expect("open_stream direct should succeed");

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
    framing::write_frame(&mut stream, &serde_json::to_vec(&req).unwrap()).await.unwrap();

    let resp_frame = framing::read_frame(&mut stream).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, substrate::derive_did_key(&caller.public_key()));

    drop(stream);
    let _ = handler_task.await;
}

#[tokio::test(start_paused = true)]
async fn stripped_enc_and_epk_causes_client_timeout_and_no_identity_sent() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-stripped-enc";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();
    let caller_hex = hex::encode(caller.public_key().as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, spy_client_side) = duplex(4096);
    let (spy_router_side, router_stream) = duplex(4096);

    let recorded_bytes = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(run_spy_forwarder(
        spy_client_side,
        spy_router_side,
        recorded_bytes.clone(),
        PreambleRewrite::StripEncAndEpk,
    ));
    let _handler_task = tokio::spawn(route_handler.clone().handle_stream(router_stream));

    let (client_read, client_write) = io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());

    let (res, _) = tokio::join!(open_stream(client_read, client_write, preamble, &path), async {
        time::advance(Duration::from_secs(11)).await;
    });

    assert!(matches!(res, Err(E2eError::Timeout(_))));

    let recorded = recorded_bytes.lock().unwrap().clone();
    assert!(
        !recorded.windows(caller_hex.len()).any(|w| w == caller_hex.as_bytes()),
        "identity bytes must not be sent"
    );
}

#[tokio::test]
async fn swapped_epk_fails_bad_host_signature_and_sends_nothing_more() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-swapped-epk";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&caller, now_secs());

    let (client_stream, spy_client_side) = duplex(4096);
    let (spy_router_side, router_stream) = duplex(4096);

    let fake_secret = EphemeralSecret::random(&mut OsRng);
    let fake_epk_hex = hex::encode(EncodedPoint::from(fake_secret.public_key()).as_bytes());

    let recorded_bytes = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(run_spy_forwarder(
        spy_client_side,
        spy_router_side,
        recorded_bytes.clone(),
        PreambleRewrite::ReplaceEpk(fake_epk_hex),
    ));
    let _handler_task = tokio::spawn(route_handler.clone().handle_stream(router_stream));

    let (client_read, client_write) = io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());
    let res = open_stream(client_read, client_write, preamble, &path).await;

    assert!(matches!(res, Err(E2eError::BadHostSignature { .. })));

    let recorded = recorded_bytes.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&recorded);
    assert_eq!(
        recorded.iter().filter(|&&b| b == b'\n').count(),
        1,
        "only preamble line sent: {text}"
    );
}

#[tokio::test]
async fn plain_identity_fields_on_encrypted_stream_refused_before_handshake() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-plain-caller-refused";
    let service = Arc::new(CapturingNativeService::default());
    register_test_channel(&route_handler, service_id, service).await;

    let caller = Identity::generate().unwrap();
    let caller_hex = hex::encode(caller.public_key().as_bytes());
    let fake_secret = EphemeralSecret::random(&mut OsRng);
    let epk_hex = hex::encode(EncodedPoint::from(fake_secret.public_key()).as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(epk_hex);
    let preamble_line = format!("{preamble}&pubkey={caller_hex}\n");

    let (mut client_io, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    client_io.write_all(preamble_line.as_bytes()).await.unwrap();
    client_io.flush().await.unwrap();

    let mut buf = [0u8; 1024];
    let n = client_io.read(&mut buf).await.unwrap();
    assert!(n > 0);
    assert_eq!(buf[0], 0x00, "refusal response must be a binary frame, not 0x04");

    let frame = framing::read_frame(&mut &buf[..n]).await.expect("valid binary frame");
    let err_val: Value = serde_json::from_slice(&frame).expect("json-rpc error response");
    assert_eq!(err_val["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn route_key_in_identity_line_gives_encrypted_refusal() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-route-key-in-identity";
    let service = Arc::new(CapturingNativeService::default());
    register_test_channel(&route_handler, service_id, service).await;

    let client_secret = EphemeralSecret::random(&mut OsRng);
    let client_point = EncodedPoint::from(client_secret.public_key());
    let epk_hex = hex::encode(client_point.as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(epk_hex);
    let preamble_line = format!("{preamble}\n");

    let (mut client_io, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    client_io.write_all(preamble_line.as_bytes()).await.unwrap();
    client_io.flush().await.unwrap();

    let mut server_reply = [0u8; 129];
    client_io.read_exact(&mut server_reply).await.unwrap();
    assert_eq!(server_reply[0], 0x04);

    let keys =
        derive_test_channel_keys(&client_secret, &server_reply[..65], client_point.as_bytes())
            .unwrap();

    let bad_identity_line = b"enc=ecdh-p256\n";
    let enc_chunk = encrypt_channel_chunk(&keys.c2s, bad_identity_line);
    client_io.write_all(&enc_chunk).await.unwrap();
    client_io.flush().await.unwrap();

    let decrypted = read_and_decrypt_channel_chunk(&mut client_io, &keys.s2c).await.unwrap();
    let frame = framing::read_frame(&mut &decrypted[..]).await.expect("valid framed response");
    let err_val: JsonRpcErrorResponse = serde_json::from_slice(&frame).unwrap();
    assert_eq!(err_val.error.code, STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test(start_paused = true)]
async fn no_identity_line_within_timeout_gives_encrypted_refusal() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-identity-timeout";
    let service = Arc::new(CapturingNativeService::default());
    register_test_channel(&route_handler, service_id, service).await;

    let client_secret = EphemeralSecret::random(&mut OsRng);
    let client_point = EncodedPoint::from(client_secret.public_key());
    let epk_hex = hex::encode(client_point.as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(epk_hex);
    let preamble_line = format!("{preamble}\n");

    let (mut client_io, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    client_io.write_all(preamble_line.as_bytes()).await.unwrap();
    client_io.flush().await.unwrap();

    let mut server_reply = [0u8; 129];
    client_io.read_exact(&mut server_reply).await.unwrap();

    let keys =
        derive_test_channel_keys(&client_secret, &server_reply[..65], client_point.as_bytes())
            .unwrap();

    time::advance(Duration::from_secs(6)).await;
    task::yield_now().await;

    let decrypted = read_and_decrypt_channel_chunk(&mut client_io, &keys.s2c).await.unwrap();
    let frame = framing::read_frame(&mut &decrypted[..]).await.expect("valid framed response");
    let err_val: JsonRpcErrorResponse = serde_json::from_slice(&frame).unwrap();
    assert_eq!(err_val.error.code, STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn empty_identity_line_treated_as_anonymous() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-empty-identity-anon";
    register_test_channel(&route_handler, service_id, service.clone()).await;

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");

    let (client_stream, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = io::split(client_stream);
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
    framing::write_frame(&mut stream, &serde_json::to_vec(&req).unwrap()).await.unwrap();

    let resp_frame = framing::read_frame(&mut stream).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unauthenticated caller for native interface")
    );

    assert!(service.caller.lock().unwrap().is_none());

    drop(stream);
    let _ = handler_task.await;
}

#[tokio::test]
async fn http_stream_with_empty_interface_verifies_sent_interface() {
    let (route_handler, http_routes) = test_route_handler().await;
    let service_id = "test-http-service";
    let target_iface = "http-native";

    route_handler
        .registry()
        .register(
            service_id.to_string(),
            "".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: service_id.to_string() },
        )
        .await
        .unwrap();
    route_handler
        .registry()
        .register(
            service_id.to_string(),
            target_iface.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() },
        )
        .await
        .unwrap();

    http_routes.insert(service_id.to_string(), vec![]);

    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();

    let mut preamble =
        RoutePreamble::new(RouteTransport::Http, RouteProtocol::JsonRpc, service_id, "");
    preamble.sign(&caller, now_secs());

    let (client_stream, server_stream) = duplex(4096);
    let _handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());
    let opened = open_stream(client_read, client_write, preamble, &path)
        .await
        .expect("open_stream should succeed");

    assert!(matches!(opened, Opened::Ready(_)));
}

#[tokio::test]
async fn refusal_before_handshake_through_open_stream_returns_service_not_found() {
    let (route_handler, _) = test_route_handler().await;
    let host_identity = Identity::from_bytes(&[7u8; 32]);
    let caller = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "unknown-service",
        "default",
    );
    preamble.sign(&caller, now_secs());

    let (client_stream, server_stream) = duplex(4096);
    let _handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    let (client_read, client_write) = io::split(client_stream);
    let path = Path::Indirect(host_identity.public_key());
    let opened = open_stream(client_read, client_write, preamble, &path)
        .await
        .expect("open_stream returns refusal without transport error");

    match opened {
        Opened::Refused(mut stream) => {
            let frame = framing::read_frame(&mut stream).await.expect("valid refusal frame");
            let err_val: JsonRpcErrorResponse = serde_json::from_slice(&frame).unwrap();
            assert_eq!(err_val.error.code, NOT_HOSTED_HERE_RPC_CODE);
        }
        Opened::Ready(_) => panic!("expected stream to be refused"),
    }
}

#[tokio::test]
async fn directional_reflection_fails_decryption() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-reflection-fail";
    let service = Arc::new(CapturingNativeService::default());
    register_test_channel(&route_handler, service_id, service).await;

    let client_secret = EphemeralSecret::random(&mut OsRng);
    let client_point = EncodedPoint::from(client_secret.public_key());
    let epk_hex = hex::encode(client_point.as_bytes());

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(epk_hex);
    let preamble_line = format!("{preamble}\n");

    let (mut client_io, server_stream) = duplex(4096);
    let _handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    client_io.write_all(preamble_line.as_bytes()).await.unwrap();
    client_io.flush().await.unwrap();

    let mut server_reply = [0u8; 129];
    client_io.read_exact(&mut server_reply).await.unwrap();

    let keys =
        derive_test_channel_keys(&client_secret, &server_reply[..65], client_point.as_bytes())
            .unwrap();

    let id_line_enc = encrypt_channel_chunk(&keys.c2s, b"\n");
    client_io.write_all(&id_line_enc).await.unwrap();
    client_io.flush().await.unwrap();

    let server_reflected_chunk = encrypt_channel_chunk(&keys.s2c, b"reflected-data");
    client_io.write_all(&server_reflected_chunk).await.unwrap();
    client_io.flush().await.unwrap();

    let res = read_and_decrypt_channel_chunk(&mut client_io, &keys.s2c).await;
    assert!(
        res.is_err(),
        "server must terminate or refuse when reflected chunk cannot be decrypted"
    );
}

#[tokio::test]
async fn invalid_epk_point_rejected_during_handshake() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-invalid-epk";
    let service = Arc::new(CapturingNativeService::default());
    register_test_channel(&route_handler, service_id, service).await;

    let compressed_epk = "02".to_string() + &hex::encode([1u8; 32]);
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(compressed_epk);
    let preamble_line = format!("{preamble}\n");

    let (mut client_io, server_stream) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server_stream));

    client_io.write_all(preamble_line.as_bytes()).await.unwrap();
    client_io.flush().await.unwrap();

    let mut reply = [0u8; 129];
    let n = client_io.read(&mut reply).await.unwrap();
    assert!(n == 0 || reply[0] != 0x04);

    let _ = handler_task.await;
}

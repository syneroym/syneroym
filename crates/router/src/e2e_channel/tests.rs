use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use syneroym_identity::Identity;
use syneroym_rpc::{NOT_HOSTED_HERE_RPC_CODE, framing};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

use super::{E2eError, Opened, Path, SEC1_UNCOMPRESSED, derive as derive_kdf, open_stream, path};
use crate::{
    RoutePreamble,
    route_handler::{encryption::apply_encryption_stage, io::read_line_bounded},
    routing::EncryptionStage,
};

#[test]
fn kdf_known_answer() {
    let shared_x = [1u8; 32];
    let mut client_epk = [2u8; 65];
    client_epk[0] = SEC1_UNCOMPRESSED;
    let mut server_pk = [3u8; 65];
    server_pk[0] = SEC1_UNCOMPRESSED;

    let keys = derive_kdf(&shared_x, &client_epk, &server_pk).expect("derive keys");
    assert_ne!(keys.client_to_server, keys.server_to_client);

    let c2s_hex = hex::encode(keys.client_to_server);
    let s2c_hex = hex::encode(keys.server_to_client);

    // Pinning the exact derived keys as hex literals to fix the wire contract with
    // peer-proxy.js
    assert_eq!(c2s_hex, "d2f3bfc8d7382442a0d68fadfa725144e589164a57a1cd7de9f4fd37723bfc86");
    assert_eq!(s2c_hex, "edf347eb9a7ea22d897d206517cf4ff7055d9eab2759cbc4cdd6e2ff197790c9");
}

#[test]
fn path_rules() {
    let key1 = Identity::generate().expect("key1").public_key();
    let vk1 = VerifyingKey::from_bytes(&key1.to_bytes()).expect("vk1");
    let key2 = Identity::generate().expect("key2").public_key();
    let vk2 = VerifyingKey::from_bytes(&key2.to_bytes()).expect("vk2");

    let dialed = *key1.as_bytes();

    // Equal key -> Direct
    let p_direct = path(&dialed, Some(&vk1), "target-svc").expect("path");
    assert!(matches!(p_direct, Path::Direct));

    // Other key -> Indirect
    let p_indirect = path(&dialed, Some(&vk2), "target-svc").expect("path");
    assert!(matches!(p_indirect, Path::Indirect(vk) if vk == vk2));

    // None -> HostKeyUnknown naming target
    let err = path(&dialed, None, "target-svc").expect_err("unknown host");
    assert!(matches!(err, E2eError::HostKeyUnknown { ref target } if target == "target-svc"));
    assert!(err.to_string().contains("target-svc"));
}

#[tokio::test]
async fn open_stream_direct_writes_identity_in_preamble() {
    let id = Identity::generate().expect("identity");
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.sign(&id, 1_700_000_000);

    let (client_stream, mut server_stream) = duplex(1024);
    let (cr, cw) = tokio::io::split(client_stream);

    let path = Path::Direct;
    let opened = open_stream(cr, cw, preamble, &path).await.expect("open stream");
    assert!(matches!(opened, Opened::Ready(_)));

    let mut line = String::new();
    let mut reader = tokio::io::BufReader::new(&mut server_stream);
    tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read line");

    assert!(line.contains("pubkey="));
    assert!(line.contains("sig="));
    assert!(!line.contains("enc="));
}

#[tokio::test]
async fn open_stream_indirect_round_trip() {
    let server_id = Identity::generate().expect("server id");
    let host_vk = VerifyingKey::from_bytes(&server_id.public_key().to_bytes()).expect("host vk");

    let caller_id = Identity::generate().expect("caller id");
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.sign(&caller_id, 1_700_000_000);
    let expected_pubkey = preamble.pubkey.clone();
    let expected_sig = preamble.sig.clone();

    let (client_stream, server_stream) = duplex(4096);
    let (cr, cw) = tokio::io::split(client_stream);
    let (sr, sw) = tokio::io::split(server_stream);

    let server_task = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(sr);
        let parsed_preamble =
            crate::route_handler::io::read_preamble(&mut reader).await.expect("read preamble");
        assert_eq!(parsed_preamble.enc.as_deref(), Some("ecdh-p256"));
        assert!(parsed_preamble.pubkey.is_none());

        let mut stream = apply_encryption_stage(
            reader,
            sw,
            &EncryptionStage::TerminateEcdhP256,
            &parsed_preamble,
            &server_id,
        )
        .await
        .expect("encryption stage");

        let mut r = tokio::io::BufReader::new(std::mem::replace(
            &mut stream.reader,
            Box::new(tokio::io::empty()),
        ));
        let id_line = read_line_bounded(&mut r).await.expect("read id line");
        stream.reader = Box::new(r);

        let fields = crate::preamble::IdentityFields::parse_line(&id_line).expect("parse id line");
        assert_eq!(fields.pubkey, expected_pubkey);
        assert_eq!(fields.sig, expected_sig);

        // Echo a frame back
        let req_frame = framing::read_frame(&mut stream.reader).await.expect("read req frame");
        framing::write_frame(&mut stream.writer, &req_frame).await.expect("write resp frame");
    });

    let path = Path::Indirect(host_vk);
    let opened = open_stream(cr, cw, preamble, &path).await.expect("open stream");
    let mut client_stream = match opened {
        Opened::Ready(s) => s,
        Opened::Refused(_) => panic!("expected Ready"),
    };

    let msg = b"hello e2e channel";
    framing::write_frame(&mut client_stream.writer, msg).await.expect("write frame");
    let resp = framing::read_frame(&mut client_stream.reader).await.expect("read frame");
    assert_eq!(resp, msg);

    server_task.await.expect("server task");
}

#[tokio::test]
async fn open_stream_refused_before_handshake() {
    let server_id = Identity::generate().expect("server id");
    let host_vk = VerifyingKey::from_bytes(&server_id.public_key().to_bytes()).expect("host vk");

    // Case 1: Binary refusal
    let (client_stream, mut server_stream) = duplex(1024);
    let (cr, cw) = tokio::io::split(client_stream);

    let server_task = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(&mut server_stream);
        let _preamble =
            crate::route_handler::io::read_preamble(&mut reader).await.expect("preamble");
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": NOT_HOSTED_HERE_RPC_CODE, "message": "not hosted here" }
        });
        framing::write_frame(&mut server_stream, &serde_json::to_vec(&body).unwrap())
            .await
            .unwrap();
    });

    let caller_id = Identity::generate().expect("caller id");
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.sign(&caller_id, 1_700_000_000);

    let path = Path::Indirect(host_vk);
    let opened = open_stream(cr, cw, preamble, &path).await.expect("open stream");
    let mut refused_stream = match opened {
        Opened::Refused(s) => s,
        Opened::Ready(_) => panic!("expected Refused"),
    };

    let frame = framing::read_frame(&mut refused_stream.reader).await.expect("read frame");
    assert!(String::from_utf8_lossy(&frame).contains("not hosted here"));
    server_task.await.expect("server");

    // Case 2: HTTP refusal (first byte 'H')
    let (client_stream2, mut server_stream2) = duplex(1024);
    let (cr2, cw2) = tokio::io::split(client_stream2);

    let server_task2 = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(&mut server_stream2);
        let _preamble =
            crate::route_handler::io::read_preamble(&mut reader).await.expect("preamble");
        server_stream2.write_all(b"HTTP/1.1 421 Misdirected Request\r\n\r\n").await.unwrap();
    });

    let mut preamble2 = RoutePreamble::new(
        crate::RouteTransport::Http,
        crate::RouteProtocol::JsonRpc,
        "svc",
        "api",
    );
    preamble2.sign(&caller_id, 1_700_000_000);

    let opened2 = open_stream(cr2, cw2, preamble2, &path).await.expect("open stream");
    let mut refused_stream2 = match opened2 {
        Opened::Refused(s) => s,
        Opened::Ready(_) => panic!("expected Refused"),
    };

    let mut buf = vec![0u8; 12];
    refused_stream2.reader.read_exact(&mut buf).await.expect("read http status");
    assert_eq!(&buf, b"HTTP/1.1 421");
    server_task2.await.expect("server 2");
}

#[tokio::test]
async fn open_stream_bad_host_signature_sends_nothing_more() {
    let wrong_id = Identity::generate().expect("wrong id");
    let expected_id = Identity::generate().expect("expected id");
    let expected_vk =
        VerifyingKey::from_bytes(&expected_id.public_key().to_bytes()).expect("expected vk");

    let (client_stream, server_stream) = duplex(1024);
    let (cr, cw) = tokio::io::split(client_stream);
    let (sr, mut sw) = tokio::io::split(server_stream);

    let server_task = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(sr);
        let preamble =
            crate::route_handler::io::read_preamble(&mut reader).await.expect("preamble");
        // Server signs with wrong_id instead of expected_id
        let client_secret = p256::ecdh::EphemeralSecret::random(&mut aes_gcm::aead::OsRng);
        let server_pk = p256::EncodedPoint::from(client_secret.public_key());
        let client_epk_hex = preamble.epk.expect("epk");
        let client_epk = hex::decode(client_epk_hex).expect("decode");
        let signed = [server_pk.as_bytes(), client_epk.as_slice()].concat();
        let signature = wrong_id.sign(&signed);
        sw.write_all(server_pk.as_bytes()).await.expect("write pk");
        sw.write_all(&signature.to_bytes()).await.expect("write sig");
        sw.flush().await.expect("flush");

        let mut remaining = Vec::new();
        reader.read_to_end(&mut remaining).await.expect("read remaining");
        // Nothing should be sent by client after bad signature
        assert!(remaining.is_empty(), "client sent bytes after bad signature: {remaining:?}");
    });

    let caller_id = Identity::generate().expect("caller id");
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.sign(&caller_id, 1_700_000_000);

    let path = Path::Indirect(expected_vk);
    let err = open_stream(cr, cw, preamble, &path).await.expect_err("expected bad signature");
    assert!(matches!(err, E2eError::BadHostSignature { .. }));

    server_task.await.expect("server task");
}

#[tokio::test(start_paused = true)]
async fn open_stream_times_out() {
    let expected_id = Identity::generate().expect("expected id");
    let expected_vk =
        VerifyingKey::from_bytes(&expected_id.public_key().to_bytes()).expect("expected vk");

    let (client_stream, _server_stream) = duplex(1024);
    let (cr, cw) = tokio::io::split(client_stream);

    let caller_id = Identity::generate().expect("caller id");
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.sign(&caller_id, 1_700_000_000);

    let path = Path::Indirect(expected_vk);
    let open_fut = open_stream(cr, cw, preamble, &path);

    tokio::pin!(open_fut);
    // Advance time past HANDSHAKE_TIMEOUT
    tokio::time::advance(Duration::from_secs(11)).await;

    let err = open_fut.await.expect_err("must timeout");
    assert!(matches!(err, E2eError::Timeout(_)));
}

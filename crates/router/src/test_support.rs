//! Fixtures shared by the tests of this crate that need a registry.

use std::{
    future,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) mod mailbox;

use syneroym_core::dht_registry::{
    EndpointInfo, EndpointMailbox, EndpointType, SignedEndpointInfo,
};
use syneroym_identity::{Identity, substrate};
use syneroym_mailbox_format::MailboxPublicKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn did_of(master: &Identity) -> String {
    substrate::derive_did_key(&master.public_key())
}

/// A service record with a mailbox, whose substrate has no record.
pub(crate) fn sleeping_peer_record() -> SignedEndpointInfo {
    let master = Identity::generate().unwrap();
    let not_after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600;
    EndpointInfo {
        service_id: did_of(&master),
        substrate_id: did_of(&Identity::generate().unwrap()),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after,
        generation: 0,
        mailbox: Some(EndpointMailbox {
            service_id: did_of(&Identity::generate().unwrap()),
            key: MailboxPublicKey::from_bytes([3; 32]),
        }),
    }
    .sign(&master)
    .unwrap()
}

/// A registry URL nobody listens on: bind a port, then drop the socket.
pub(crate) async fn dead_registry_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// What `registry_serving_only` does with a lookup for any other id.
#[derive(Clone, Copy)]
pub(crate) enum Substrate {
    /// Keep the socket open and never answer.
    Hangs,
    /// Answer `404 Not Found`.
    Missing,
}

/// A registry that answers `GET /lookup/<id of record>` with `record`, and
/// every other request as `others` says.
pub(crate) async fn registry_serving_only(
    record: &SignedEndpointInfo,
    others: Substrate,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let wanted = format!("GET /lookup/{} ", record.info.service_id);
    let body = serde_json::to_string(record).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let (wanted, body) = (wanted.clone(), body.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if String::from_utf8_lossy(&buf[..n]).starts_with(&wanted) {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
                         {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                match others {
                    Substrate::Hangs => future::pending::<()>().await,
                    Substrate::Missing => {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    }
                }
            });
        }
    });
    url
}

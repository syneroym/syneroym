//! `RegistryMailboxDirectory` against a registry that answers over real HTTP.

use syneroym_app_orchestration::models::MailboxChoice;
use syneroym_core::dht_registry::EndpointMailbox;
use syneroym_mailbox_format::MailboxSecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::*;

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";

/// A registry that answers each request by the start of its path.
async fn registry(routes: Vec<(String, u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = routes
                    .iter()
                    .find(|(prefix, ..)| path.starts_with(prefix.as_str()))
                    .map_or((404, String::new()), |(_, status, body)| (*status, body.clone()));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    url
}

/// A signed record of a new member master that names `host` as its mailbox,
/// valid until `not_after`.
fn record(host: &str, not_after: u64) -> (String, SignedEndpointInfo) {
    let master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&master.public_key());
    let route = EndpointMailbox {
        service_id: host.to_string(),
        key: MailboxSecretKey::derive_from_master(&master).public_key(),
    };
    let json = member_registry_record(
        Visibility::Public,
        &service_id,
        "did:key:zSub",
        &master,
        not_after,
        Some(route),
    )
    .unwrap()
    .unwrap();
    (service_id, serde_json::from_str(&json).unwrap())
}

const FAR_FUTURE: u64 = 9_999_999_999;

fn route_for(service_id: &str, status: u16, body: &impl serde::Serialize) -> (String, u16, String) {
    (format!("/lookup/{service_id}"), status, serde_json::to_string(body).unwrap())
}

#[tokio::test]
async fn a_good_record_names_its_mailbox_over_http() {
    let (id, signed) = record(DID_A, FAR_FUTURE);
    let url = registry(vec![route_for(&id, 200, &signed)]).await;

    let found = RegistryMailboxDirectory::new(url).named_mailbox(&id).await.unwrap();

    assert_eq!(found.map(|h| h.to_string()).as_deref(), Some(DID_A));
}

#[tokio::test]
async fn a_record_of_another_service_names_no_mailbox() {
    let (_, signed) = record(DID_A, FAR_FUTURE);
    let asked = "did:key:hasked";
    let url = registry(vec![route_for(asked, 200, &signed)]).await;

    let found = RegistryMailboxDirectory::new(url).named_mailbox(asked).await.unwrap();

    assert_eq!(found, None);
}

#[tokio::test]
async fn a_record_that_does_not_verify_names_no_mailbox() {
    let (id, mut signed) = record(DID_A, FAR_FUTURE);
    signed.info.mailbox.as_mut().unwrap().service_id = DID_B.to_string();
    let url = registry(vec![route_for(&id, 200, &signed)]).await;

    let found = RegistryMailboxDirectory::new(url).named_mailbox(&id).await.unwrap();

    assert_eq!(found, None);
}

#[tokio::test]
async fn an_expired_record_still_names_its_mailbox() {
    let (id, signed) = record(DID_A, 1);
    assert!(signed.verify().is_err(), "the record is past its not_after");
    let url = registry(vec![route_for(&id, 200, &signed)]).await;

    let found = RegistryMailboxDirectory::new(url).named_mailbox(&id).await.unwrap();

    assert_eq!(found.map(|h| h.to_string()).as_deref(), Some(DID_A));
}

#[tokio::test]
async fn no_record_is_none_and_a_server_error_is_an_error() {
    let url = registry(vec![("/lookup/broken".to_string(), 500, String::new())]).await;
    let directory = RegistryMailboxDirectory::new(url);

    assert_eq!(directory.named_mailbox("did:key:hmissing").await.unwrap(), None);
    assert!(directory.named_mailbox("broken").await.is_err());
}

#[tokio::test]
async fn the_open_list_is_read_with_its_json_shape() {
    let body = serde_json::json!({
        "max_age_secs": 60,
        "mailboxes": [{ "service_id": DID_A, "operator": "op" }],
    })
    .to_string();
    let url = registry(vec![("/mailboxes".to_string(), 200, body)]).await;

    let entries = RegistryMailboxDirectory::new(url).open_mailboxes().await.unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].service_id, DID_A);
}

#[tokio::test]
async fn a_members_current_record_is_kept_through_the_real_directory() {
    let (id, signed) = record(DID_B, FAR_FUTURE);
    let url = registry(vec![route_for(&id, 200, &signed)]).await;
    let directory = RegistryMailboxDirectory::new(url);
    let mut member = service("web", None);
    member.service_id = ServiceId::new(id);
    member.mailbox = Some(MailboxChoice::Auto);
    let mut plan = plan(vec![member]);
    let sources =
        PinSources { directory: Some(&directory), members_are_masters: true, ..Default::default() };

    let pins = pin_auto_mailboxes(&mut plan, sources).await.unwrap();

    assert_eq!(pins[0].source, PinSource::CurrentRecord);
    assert_eq!(pins[0].mailbox.as_str(), DID_B);
}

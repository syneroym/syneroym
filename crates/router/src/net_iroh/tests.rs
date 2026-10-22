use super::*;

#[test]
fn relay_mode_without_url_is_an_empty_custom_map() {
    let mode = relay_mode(None);
    assert!(matches!(mode, RelayMode::Custom(_)));
    assert_eq!(mode.relay_map().len(), 0);
}

#[test]
fn relay_mode_with_url_has_only_that_relay() {
    let url: RelayUrl = "http://127.0.0.1:7964".parse().unwrap();
    let mode = relay_mode(Some(url.clone()));
    assert!(matches!(mode, RelayMode::Custom(_)));
    assert_eq!(mode.relay_map().len(), 1);
    assert!(mode.relay_map().contains(&url));
}

#[test]
fn parse_relay_url_accepts_http_and_https() {
    let http = parse_relay_url("test", "http://example.org:7964").unwrap();
    assert_eq!(http.as_str(), "http://example.org:7964/");

    let https = parse_relay_url("test", "https://example.org:7964").unwrap();
    assert_eq!(https.as_str(), "https://example.org:7964/");
}

#[test]
fn parse_relay_url_rejects_a_value_without_scheme() {
    let err =
        parse_relay_url("parent_coordinator.iroh.url", "localhost:7964").unwrap_err().to_string();
    assert!(err.contains("parent_coordinator.iroh.url"), "expected setting name: {err}");
    assert!(err.contains("localhost:7964"), "expected bad value: {err}");
}

#[test]
fn parse_relay_url_rejects_other_schemes() {
    let err = parse_relay_url("test", "ftp://x").unwrap_err().to_string();
    assert!(err.contains("must be an http:// or https:// URL"), "expected error: {err}");
}

#[test]
fn parse_relay_url_rejects_garbage() {
    let err = parse_relay_url("test", "not a url").unwrap_err().to_string();
    assert!(err.contains("test = \"not a url\""), "expected error: {err}");
}

#[test]
fn parent_relay_url_is_none_without_url() {
    let config = IrohParentConfig { url: None };
    assert_eq!(parent_relay_url(&config).unwrap(), None);
}

// ── `resolve_peer` ─────────────────────────────────────────────────────

mod resolve_peer {
    use std::time::Instant;

    use super::super::*;
    use crate::{
        peer_records::PeerRecordStore,
        test_support::{Substrate, dead_registry_url, registry_serving_only, sleeping_peer_record},
    };

    #[tokio::test]
    async fn an_unreachable_registry_falls_back_to_the_stored_record() {
        let record = sleeping_peer_record();
        let store = PeerRecordStore::open_in_memory().unwrap();
        store.remember(&record).await.unwrap();
        let client = RegistryClient::new(false, Some(dead_registry_url().await));

        let peer = resolve_peer(&client, Some(&store), &record.info.service_id).await.unwrap();

        assert_eq!(peer.mailbox, record.info.mailbox);
        assert!(peer.addr.is_none());
    }

    #[tokio::test]
    async fn without_a_store_an_unreachable_registry_is_an_error() {
        let record = sleeping_peer_record();
        let client = RegistryClient::new(false, Some(dead_registry_url().await));

        assert!(resolve_peer(&client, None, &record.info.service_id).await.is_err());
        assert!(resolve_iroh_addr(&client, &record.info.service_id).await.is_err());
    }

    #[tokio::test]
    async fn a_peer_with_a_mailbox_but_no_substrate_record_still_resolves() {
        let record = sleeping_peer_record();
        let url = registry_serving_only(&record, Substrate::Hangs).await;
        let client = RegistryClient::new(false, Some(url));

        let started = Instant::now();
        let peer = resolve_peer(&client, None, &record.info.service_id).await.unwrap();

        assert_eq!(peer.mailbox, record.info.mailbox);
        assert_eq!(peer.host_key, substrate::resolve_did_key(&record.info.substrate_id).ok());
        assert!(peer.addr.is_none());
        // The registry never answers the substrate lookup, and the HTTP
        // client waits 10 s; only the bound makes this fast.
        assert!(started.elapsed() < Duration::from_secs(6), "{:?}", started.elapsed());
    }
}

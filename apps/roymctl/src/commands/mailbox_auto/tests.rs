use std::sync::atomic::{AtomicUsize, Ordering};

use syneroym_core::curated_list::MailboxEntry;

use super::*;

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";
const REGISTRY: &str = "http://registry.example.org:7961";

#[derive(Debug, Default)]
struct Fake {
    entries: Vec<MailboxEntry>,
    current: Option<ServiceId>,
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl MailboxDirectory for Fake {
    async fn open_mailboxes(&self) -> anyhow::Result<Vec<MailboxEntry>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.entries.clone())
    }

    async fn named_mailbox(&self, _service_id: &str) -> anyhow::Result<Option<ServiceId>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.current.clone())
    }
}

fn listing(dids: &[&str]) -> Fake {
    let entries = dids
        .iter()
        .map(|d| MailboxEntry { service_id: (*d).to_string(), operator: "op".to_string() })
        .collect();
    Fake { entries, ..Fake::default() }
}

#[tokio::test]
async fn auto_keeps_the_mailbox_the_current_record_names() {
    let fake = Fake { current: Some(ServiceId::new(DID_B)), ..listing(&[DID_A]) };

    let host = pick_svc_mailbox("did:key:hsvc", &fake, REGISTRY, HostRegistry::Url(REGISTRY))
        .await
        .unwrap();

    assert_eq!(host.as_str(), DID_B);
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1, "the list is not read");
}

#[tokio::test]
async fn auto_without_a_current_record_takes_the_first_ranked_mailbox() {
    let fake = listing(&[DID_A, DID_B]);
    let expected = deploy::pick_open_mailbox("did:key:hsvc", fake.entries.clone()).unwrap();

    let host = pick_svc_mailbox("did:key:hsvc", &fake, REGISTRY, HostRegistry::Url(REGISTRY))
        .await
        .unwrap();

    assert_eq!(host, expected);
}

#[tokio::test]
async fn auto_without_a_registry_url_is_refused_before_any_request() {
    let err = resolve_svc_mailbox(
        Some(&MailboxChoice::Auto),
        "did:key:hsvc",
        None,
        HostRegistry::Unknown,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("--registry-url"), "{err}");
}

#[tokio::test]
async fn a_named_mailbox_needs_no_registry() {
    let choice = MailboxChoice::Service(ServiceId::new(DID_A));

    let host = resolve_svc_mailbox(Some(&choice), "did:key:hsvc", None, HostRegistry::Unknown)
        .await
        .unwrap();
    let none =
        resolve_svc_mailbox(None, "did:key:hsvc", None, HostRegistry::Unknown).await.unwrap();

    assert_eq!(host.unwrap().as_str(), DID_A);
    assert_eq!(none, None);
}

#[tokio::test]
async fn a_host_on_another_registry_is_refused() {
    let fake = listing(&[DID_A]);

    let err = pick_svc_mailbox(
        "did:key:hsvc",
        &fake,
        REGISTRY,
        HostRegistry::Url("http://other.example.org:7961"),
    )
    .await
    .unwrap_err();

    let text = err.to_string();
    assert!(text.contains("other.example.org") && text.contains("registry.example.org"), "{text}");
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1, "only the record was read");
}

#[test]
fn a_host_with_no_registry_is_refused() {
    let err = check_same_registry("web", REGISTRY, HostRegistry::None).unwrap_err();

    assert!(err.to_string().contains("no registry"), "{err}");
}

#[test]
fn an_unknown_host_registry_only_prints_a_note() {
    check_same_registry("web", REGISTRY, HostRegistry::Unknown).unwrap();
}

#[test]
fn the_same_registry_in_another_spelling_passes() {
    check_same_registry("web", "http://R:80/", HostRegistry::Url("http://r")).unwrap();
}

//! `auto` mailbox pinning: stored plan, current record, then the list.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use syneroym_app_orchestration::models::MailboxChoice;
use syneroym_core::{curated_list::MailboxEntry, dht_registry::EndpointMailbox};
use syneroym_mailbox_format::MailboxSecretKey;

use super::*;

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";
const DID_C: &str = "did:key:h7wy7c8txi9eay56m9m53wco4nrstgt6hxbxp1snrdu5c8r69nre6ara";

#[derive(Debug, Default)]
struct FakeDirectory {
    entries: Vec<MailboxEntry>,
    records: HashMap<String, ServiceId>,
    list_reads: AtomicUsize,
    record_reads: AtomicUsize,
}

impl FakeDirectory {
    fn listing(dids: &[&str]) -> Self {
        Self { entries: dids.iter().map(|d| entry(d)).collect(), ..Self::default() }
    }

    fn reads(&self) -> (usize, usize) {
        (self.list_reads.load(Ordering::SeqCst), self.record_reads.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl MailboxDirectory for FakeDirectory {
    async fn open_mailboxes(&self) -> Result<Vec<MailboxEntry>> {
        self.list_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.entries.clone())
    }

    async fn named_mailbox(&self, service_id: &str) -> Result<Option<ServiceId>> {
        self.record_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.records.get(service_id).cloned())
    }
}

fn entry(did: &str) -> MailboxEntry {
    MailboxEntry { service_id: did.to_string(), operator: "op".to_string() }
}

fn auto_service(name: &str) -> PlannedService {
    let mut svc = service(name, None);
    svc.mailbox = Some(MailboxChoice::Auto);
    svc
}

fn with_host(mut svc: PlannedService, host: &str) -> PlannedService {
    svc.mailbox = Some(MailboxChoice::Service(ServiceId::new(host)));
    svc
}

fn sources<'a>(
    directory: &'a FakeDirectory,
    previous: Option<&'a DeploymentPlan>,
    members_are_masters: bool,
) -> PinSources<'a> {
    PinSources { previous, directory: Some(directory), members_are_masters }
}

fn host_of(plan: &DeploymentPlan, index: usize) -> Option<String> {
    match &plan.services[index].mailbox {
        Some(MailboxChoice::Service(host)) => Some(host.to_string()),
        _ => None,
    }
}

#[tokio::test]
async fn a_plan_without_auto_reads_nothing() {
    let dir = FakeDirectory::listing(&[DID_A]);
    let mut plan = plan(vec![service("web", None), with_host(service("db", None), DID_B)]);
    let before = plan.clone();

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, None, true)).await.unwrap();

    assert!(pins.is_empty());
    assert_eq!(plan, before);
    assert_eq!(dir.reads(), (0, 0));
}

#[tokio::test]
async fn auto_takes_the_first_ranked_open_mailbox_for_every_member() {
    let dir = FakeDirectory::listing(&[DID_A, DID_B, DID_C]);
    let mut second = auto_service("web");
    second.service_id = ServiceId::new("did:key:hweb-2");
    second.member_index = 1;
    let mut plan = plan(vec![auto_service("web"), second]);
    let expected = pick_open_mailbox("inst-1/web", dir.entries.clone()).unwrap().to_string();

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, None, false)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(expected.as_str()));
    assert_eq!(host_of(&plan, 1).as_deref(), Some(expected.as_str()));
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].source, PinSource::List);
}

#[tokio::test]
async fn a_stored_mailbox_is_kept_without_any_read() {
    let dir = FakeDirectory::listing(&[DID_A]);
    let previous = plan(vec![with_host(service("web", None), DID_B)]);
    let mut plan = plan(vec![auto_service("web")]);

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, Some(&previous), true)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(DID_B));
    assert_eq!(pins[0].source, PinSource::StoredPlan);
    assert_eq!(dir.reads(), (0, 0));
}

#[tokio::test]
async fn a_stored_mailbox_is_kept_after_it_left_the_list() {
    let dir = FakeDirectory::listing(&[DID_A, DID_C]);
    let previous = plan(vec![with_host(service("web", None), DID_B)]);
    let mut plan = plan(vec![auto_service("web")]);

    pin_auto_mailboxes(&mut plan, sources(&dir, Some(&previous), false)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(DID_B));
}

#[tokio::test]
async fn a_mailbox_in_a_members_current_record_is_kept_when_members_are_masters() {
    let mut dir = FakeDirectory::listing(&[DID_A]);
    dir.records.insert("did:key:hweb".to_string(), ServiceId::new(DID_B));
    let mut plan = plan(vec![auto_service("web")]);

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, None, true)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(DID_B));
    assert_eq!(pins[0].source, PinSource::CurrentRecord);
    assert_eq!(dir.reads(), (0, 1));
}

#[tokio::test]
async fn current_records_are_not_read_for_made_up_ids() {
    let mut dir = FakeDirectory::listing(&[DID_A]);
    dir.records.insert("did:key:hweb".to_string(), ServiceId::new(DID_B));
    let mut plan = plan(vec![auto_service("web")]);

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, None, false)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(DID_A));
    assert_eq!(pins[0].source, PinSource::List);
    assert_eq!(dir.reads(), (1, 0));
}

#[tokio::test]
async fn a_named_mailbox_is_never_changed() {
    let dir = FakeDirectory::listing(&[DID_A]);
    let previous = plan(vec![with_host(service("web", None), DID_C)]);
    let mut plan = plan(vec![with_host(service("web", None), DID_B)]);

    pin_auto_mailboxes(&mut plan, sources(&dir, Some(&previous), true)).await.unwrap();

    assert_eq!(host_of(&plan, 0).as_deref(), Some(DID_B));
}

#[tokio::test]
async fn the_list_is_read_once_for_many_services() {
    let dir = FakeDirectory::listing(&[DID_A, DID_B, DID_C]);
    let mut plan = plan(vec![auto_service("web"), auto_service("api"), auto_service("jobs")]);

    let pins = pin_auto_mailboxes(&mut plan, sources(&dir, None, false)).await.unwrap();

    assert_eq!(pins.len(), 3);
    assert_eq!(dir.reads(), (1, 0));
}

#[tokio::test]
async fn an_empty_list_is_an_error_that_names_the_service() {
    let dir = FakeDirectory::default();
    let mut plan = plan(vec![auto_service("web")]);

    let err = pin_auto_mailboxes(&mut plan, sources(&dir, None, false)).await.unwrap_err();

    let text = err.to_string();
    assert!(text.contains("inst-1/web") && text.contains("no open mailbox"), "{text}");
    assert_eq!(plan.services[0].mailbox, Some(MailboxChoice::Auto));
}

#[tokio::test]
async fn a_list_with_only_bad_entries_is_an_error() {
    let dir = FakeDirectory::listing(&["did:web:example.com", "", "did:key:not-z32"]);
    let mut plan = plan(vec![auto_service("web")]);

    let err = pin_auto_mailboxes(&mut plan, sources(&dir, None, false)).await.unwrap_err();

    assert!(err.to_string().contains("no open mailbox"), "{err}");
}

#[tokio::test]
async fn auto_without_a_directory_is_an_error() {
    let mut plan = plan(vec![auto_service("web")]);

    let err = pin_auto_mailboxes(&mut plan, PinSources::default()).await.unwrap_err();

    assert!(err.to_string().contains("no registry is given"), "{err}");
}

#[test]
fn keep_earlier_mailboxes_leaves_an_auto_with_no_earlier_mailbox() {
    let previous = plan(vec![with_host(service("api", None), DID_B)]);
    let mut plan = plan(vec![auto_service("web"), auto_service("api")]);

    let pins = keep_earlier_mailboxes(&mut plan, Some(&previous));

    assert_eq!(pins.len(), 1);
    assert_eq!(plan.services[0].mailbox, Some(MailboxChoice::Auto));
    assert_eq!(host_of(&plan, 1).as_deref(), Some(DID_B));
}

#[test]
fn verified_mailbox_ignores_a_record_that_does_not_verify_or_is_for_another_service() {
    let master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&master.public_key());
    let route = EndpointMailbox {
        service_id: DID_A.to_string(),
        key: MailboxSecretKey::derive_from_master(&master).public_key(),
    };
    let json = member_registry_record(
        Visibility::Public,
        &service_id,
        "did:key:zSub",
        &master,
        9_999_999_999,
        Some(route.clone()),
    )
    .unwrap()
    .unwrap();
    let signed: SignedEndpointInfo = serde_json::from_str(&json).unwrap();

    assert_eq!(verified_mailbox(&service_id, Some(&signed)), Some(&route));
    assert_eq!(verified_mailbox("did:key:hother", Some(&signed)), None);
    assert_eq!(verified_mailbox(&service_id, None), None);
    let mut tampered = signed.clone();
    if let Some(mailbox) = tampered.info.mailbox.as_mut() {
        mailbox.service_id = DID_B.to_string();
    }
    assert_eq!(verified_mailbox(&service_id, Some(&tampered)), None);
}

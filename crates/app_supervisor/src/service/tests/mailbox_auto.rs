//! A mailbox `auto` is pinned at `submit`, before the desired state is
//! stored.

use std::sync::atomic::{AtomicUsize, Ordering};

use syneroym_core::curated_list::MailboxEntry;
use syneroym_sdk::deploy::MailboxDirectory;

use super::{super::*, helpers::*};

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";
const INSTANCE: &str = "inst-auto";

#[derive(Debug, Default)]
struct FakeDirectory {
    dids: Vec<&'static str>,
    current: Option<&'static str>,
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl MailboxDirectory for FakeDirectory {
    async fn open_mailboxes(&self) -> anyhow::Result<Vec<MailboxEntry>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .dids
            .iter()
            .map(|d| MailboxEntry { service_id: (*d).to_string(), operator: "op".to_string() })
            .collect())
    }

    async fn named_mailbox(&self, _service_id: &str) -> anyhow::Result<Option<ServiceId>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.current.map(ServiceId::new))
    }
}

fn plan_json(mailbox: &str) -> String {
    serde_json::json!({
        "app_instance_id": INSTANCE,
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated",
            "logical_ref": format!("{INSTANCE}/backend"),
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton",
            "visibility": "public",
            "mailbox": mailbox,
        }]
    })
    .to_string()
}

/// The alias has no credential, so the apply that follows the store fails
/// fast with "no credential". The desired state is stored before it.
fn inventory_json() -> String {
    serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
        .to_string()
}

async fn submit(s: &SupervisorService, plan_json: String) -> RpcResult<NativeResponse> {
    dispatch(
        s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": INSTANCE,
            "plan_json": plan_json,
            "inventory_json": inventory_json(),
            "generation": 0,
        }]),
    )
    .await
}

/// The mailbox of the first member in the stored desired state.
fn stored_mailbox(s: &SupervisorService) -> Option<String> {
    let state = s.store.get(INSTANCE).unwrap()?;
    let plan = DeploymentPlan::from_json(&state.plan_json).unwrap();
    plan.services[0].mailbox.as_ref().map(ToString::to_string)
}

fn with_directory(directory: FakeDirectory) -> (SupervisorService, Arc<FakeDirectory>) {
    let directory = Arc::new(directory);
    (service().with_mailbox_directory(directory.clone()), directory)
}

#[tokio::test]
async fn submit_pins_auto_in_the_stored_plan() {
    let (s, _) = with_directory(FakeDirectory { dids: vec![DID_A], ..Default::default() });

    let _ = submit(&s, plan_json("auto")).await;

    assert_eq!(stored_mailbox(&s).as_deref(), Some(DID_A));
}

#[tokio::test]
async fn a_second_submit_keeps_the_pinned_mailbox_when_the_list_changes() {
    let (s, _) = with_directory(FakeDirectory { dids: vec![DID_A], ..Default::default() });
    let _ = submit(&s, plan_json("auto")).await;
    let s = s.with_mailbox_directory(Arc::new(FakeDirectory {
        dids: vec![DID_B],
        ..Default::default()
    }));

    let _ = submit(&s, plan_json("auto")).await;

    assert_eq!(stored_mailbox(&s).as_deref(), Some(DID_A));
}

#[tokio::test]
async fn a_first_submit_keeps_the_mailbox_a_members_record_names() {
    let (s, _) = with_directory(FakeDirectory {
        dids: vec![DID_A],
        current: Some(DID_B),
        ..Default::default()
    });

    let _ = submit(&s, plan_json("auto")).await;

    assert_eq!(stored_mailbox(&s).as_deref(), Some(DID_B));
}

#[tokio::test]
async fn a_named_mailbox_is_stored_as_given() {
    let (s, directory) = with_directory(FakeDirectory { dids: vec![DID_A], ..Default::default() });

    let _ = submit(&s, plan_json(DID_B)).await;

    assert_eq!(stored_mailbox(&s).as_deref(), Some(DID_B));
    assert_eq!(directory.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn auto_on_a_node_without_a_registry_is_refused_and_nothing_is_stored() {
    let s = service();

    let err = submit(&s, plan_json("auto")).await.unwrap_err();

    let text = err.to_string();
    assert!(text.contains("no registry") && text.contains("substrate.registry_url"), "{text}");
    assert!(s.store.get(INSTANCE).unwrap().is_none());
}

#[tokio::test]
async fn an_empty_open_list_refuses_the_submit_and_stores_nothing() {
    let (s, _) = with_directory(FakeDirectory::default());

    let err = submit(&s, plan_json("auto")).await.unwrap_err();

    assert!(err.to_string().contains("no open mailbox"), "{err}");
    assert!(s.store.get(INSTANCE).unwrap().is_none());
}

#[tokio::test]
async fn a_locked_vault_refuses_before_the_registry_is_read() {
    let directory = Arc::new(FakeDirectory { dids: vec![DID_A], ..Default::default() });
    let s = service_with_locked_vault().with_mailbox_directory(directory.clone());

    let err = submit(&s, plan_json("auto")).await.unwrap_err();

    assert!(err.to_string().contains("inject-kek"), "{err}");
    assert_eq!(directory.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_stored_plan_that_does_not_parse_refuses_the_submit() {
    let (s, directory) = with_directory(FakeDirectory { dids: vec![DID_A], ..Default::default() });
    s.store.submit(INSTANCE, "not json", "{}", "did:key:owner", 0).unwrap();

    let err = submit(&s, plan_json("auto")).await.unwrap_err();

    assert!(err.to_string().contains("does not parse"), "{err}");
    assert_eq!(directory.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn status_shows_the_pinned_mailbox() {
    let (s, _) = with_directory(FakeDirectory { dids: vec![DID_A], ..Default::default() });
    let _ = submit(&s, plan_json("auto")).await;

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!([INSTANCE]),
    )
    .await
    .unwrap();

    let services = status.payload.get("services").and_then(Value::as_array).unwrap();
    assert_eq!(services[0].get("mailbox").and_then(Value::as_str), Some(DID_A));
}

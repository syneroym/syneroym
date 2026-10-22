//! The outbox when a peer cannot be reached and its mailbox takes the
//! message: the state, the parked row, and what each answer does.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Duration,
};

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_rpc::{
    ConversationHost, Delivery, DepositStatus, DepositTicket, ProxyError, ProxyRequest,
    ServiceProxy,
};

use super::*;
use crate::store::SessionRow;

const ME: &str = "svc:me";
const BOB: &str = "svc:bob";
const CAROL: &str = "svc:carol";
const THIRTY_ONE_DAYS_MS: i64 = 31 * 24 * 3600 * 1000;

/// How the next call to a peer ends.
#[derive(Debug, Clone)]
enum Next {
    Deposit,
    Direct(Value),
}

/// A proxy that answers prekey requests from a second service, and ends
/// every other delivery as the test says.
#[derive(Debug, Default)]
struct ScriptedProxy {
    peer: Mutex<Option<Arc<ConversationService>>>,
    /// What a `deliver_or_deposit` to `(method, target)` does.
    next: Mutex<HashMap<(String, String), Next>>,
    statuses: Mutex<VecDeque<DepositStatus>>,
    /// `(method, target, params, max_wait_secs)` of each `deliver_or_deposit`.
    deliveries: Mutex<Vec<(String, String, Value, u64)>>,
    /// `(method, target)` of each plain `invoke`.
    invoked: Mutex<Vec<(String, String)>>,
    polled: AtomicUsize,
    /// Runs once, inside the next `deliver_or_deposit` / `poll_deposits`:
    /// what a person does while the worker waits on the network.
    during_deliver: Mutex<Option<Hook>>,
    during_poll: Mutex<Option<Hook>>,
}

struct Hook(Box<dyn FnOnce() + Send>);

impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hook")
    }
}

impl ScriptedProxy {
    fn script(&self, method: &str, target: &str, next: Next) {
        self.next.lock().unwrap().insert((method.to_string(), target.to_string()), next);
    }

    fn answers(&self, statuses: impl IntoIterator<Item = DepositStatus>) {
        self.statuses.lock().unwrap().extend(statuses);
    }

    fn deliveries_of(&self, method: &str) -> Vec<(String, Value, u64)> {
        let all = self.deliveries.lock().unwrap();
        all.iter()
            .filter(|(m, ..)| m == method)
            .map(|(_, target, params, wait)| (target.clone(), params.clone(), *wait))
            .collect()
    }
}

fn ticket() -> DepositTicket {
    DepositTicket {
        context: b"deposit-context".to_vec(),
        poll_interval: Duration::from_secs(60),
        deadline_at_ms: now_ms() + 3_600_000,
    }
}

fn ack_for(request: &ProxyRequest) -> Value {
    match request.method.as_str() {
        "group-push" => json!({ "accepted": [] }),
        _ => json!({ "message_id": request.idempotency_key }),
    }
}

#[async_trait]
impl ServiceProxy for ScriptedProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invoked.lock().unwrap().push((request.method.clone(), request.target_service.clone()));
        if request.method == "prekey-bundle" {
            let peer = self.peer.lock().unwrap().clone().expect("a peer service for prekeys");
            let bundle = peer.prekey_bundle(&request.target_service, ME).await.unwrap();
            return Ok(serde_json::from_slice(&bundle).unwrap());
        }
        Ok(ack_for(&request))
    }

    async fn deliver_or_deposit(
        &self,
        request: ProxyRequest,
        max_wait_secs: u64,
    ) -> Result<Delivery, ProxyError> {
        let key = (request.method.clone(), request.target_service.clone());
        if let Some(hook) = self.during_deliver.lock().unwrap().take() {
            (hook.0)();
        }
        self.deliveries.lock().unwrap().push((
            request.method.clone(),
            request.target_service.clone(),
            request.params.clone(),
            max_wait_secs,
        ));
        let next = self.next.lock().unwrap().get(&key).cloned();
        match next {
            Some(Next::Deposit) => Ok(Delivery::Deposited(ticket())),
            Some(Next::Direct(value)) => Ok(Delivery::Direct(value)),
            None => Ok(Delivery::Direct(ack_for(&request))),
        }
    }

    async fn poll_deposits(&self, _sender: &str, contexts: Vec<Vec<u8>>) -> Vec<DepositStatus> {
        self.polled.fetch_add(contexts.len(), Ordering::SeqCst);
        if let Some(hook) = self.during_poll.lock().unwrap().take() {
            (hook.0)();
        }
        let mut statuses = self.statuses.lock().unwrap();
        contexts.iter().map(|_| statuses.pop_front().unwrap_or(DepositStatus::Waiting)).collect()
    }
}

/// Records every delivery state the app is told about.
#[derive(Debug, Default)]
struct StateLog {
    states: Mutex<Vec<(String, ConversationDeliveryState)>>,
}

impl StateLog {
    fn of(&self, message: &str) -> Vec<ConversationDeliveryState> {
        let states = self.states.lock().unwrap();
        states.iter().filter(|(m, _)| m == message).map(|(_, s)| *s).collect()
    }
}

#[async_trait]
impl ConversationNotifier for StateLog {
    async fn notify_message(&self, _: &str, _: ConversationMessage) -> NotifyOutcome {
        NotifyOutcome::NoAnswer
    }

    async fn notify_delivery_state(
        &self,
        _: &str,
        message: String,
        state: ConversationDeliveryState,
    ) {
        self.states.lock().unwrap().push((message, state));
    }
}

struct World {
    service: Arc<ConversationService>,
    store: Arc<ConversationStore>,
    proxy: Arc<ScriptedProxy>,
    log: Arc<StateLog>,
    _peer: Arc<ConversationService>,
    _dirs: [tempfile::TempDir; 2],
}

async fn world() -> World {
    let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let service = test_service(dirs[0].path()).await;
    let peer = test_service(dirs[1].path()).await;

    let master = Identity::generate().unwrap();
    let instance = Identity::generate().unwrap();
    let cert = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    service.registry.set_owner(ME.to_string(), "did:key:zOwner".to_string()).await.unwrap();
    service.registry.set_instance_cert(ME.to_string(), cert).await.unwrap();

    let proxy = Arc::new(ScriptedProxy::default());
    *proxy.peer.lock().unwrap() = Some(peer.clone());
    service.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
    let log = Arc::new(StateLog::default());
    service.register_service_notifier(
        ME.to_string(),
        Arc::downgrade(&log) as Weak<dyn ConversationNotifier>,
    );
    let store = service.store_for(ME).await.unwrap();
    World { service, store, proxy, log, _peer: peer, _dirs: dirs }
}

impl World {
    /// A 1:1 message to `BOB`, sent but not yet delivered.
    async fn send_direct(&self) -> String {
        let conv = self.service.open_direct(ME, BOB).await.unwrap();
        self.service.send(ME, &conv, "text/plain", b"hello".to_vec()).await.unwrap()
    }

    fn state(&self, message: &str) -> ConversationDeliveryState {
        self.store.get_message(message).unwrap().unwrap().state
    }

    fn rows(&self) -> Vec<QueueItem> {
        self.store.queue().all().unwrap()
    }

    /// The poll time of every parked row has come.
    async fn parked_rows_are_due(&self) {
        for item in self.rows() {
            if let Some(parked) = item.parked {
                let queue = self.store.queue();
                queue.park(item.id, now_ms(), 1, parked.deadline_at, &parked.context).unwrap();
            }
        }
        time::sleep(Duration::from_millis(5)).await;
    }

    async fn pass(&self) {
        self.service.drain_once().await;
    }

    async fn parked_pass(&self) {
        self.parked_rows_are_due().await;
        self.service.drain_parked_once().await;
    }
}

#[tokio::test]
async fn a_deposit_marks_the_message_deposited_parks_the_row_and_tells_the_app_once() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;

    world.pass().await;
    world.pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Deposited);
    let rows = world.rows();
    assert!(rows.iter().any(|r| r.parked.is_some()), "the row waits at the mailbox");
    assert_eq!(world.proxy.deliveries_of("deliver").len(), 1, "no direct attempt while it waits");
    assert_eq!(world.log.of(&message), vec![ConversationDeliveryState::Deposited]);
}

#[tokio::test]
async fn a_signed_delivery_completes_the_message_and_removes_its_row() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    world.proxy.answers([DepositStatus::Delivered]);

    world.parked_pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Delivered);
    assert!(world.rows().iter().all(|r| r.parked.is_none()));
    assert!(world.store.outbound_envelope(&message).unwrap().is_none());
    assert_eq!(
        world.log.of(&message),
        vec![ConversationDeliveryState::Deposited, ConversationDeliveryState::Delivered]
    );
}

#[tokio::test]
async fn a_signed_rejection_fails_the_message_with_the_code() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    world.proxy.answers([DepositStatus::Rejected { code: "not_eligible".to_string() }]);

    world.parked_pass().await;

    let stored = world.store.get_message(&message).unwrap().unwrap();
    assert_eq!(stored.state, ConversationDeliveryState::Failed);
    assert!(stored.last_error.unwrap().contains("not_eligible"));
}

#[tokio::test]
async fn an_answer_that_cannot_be_trusted_sends_the_message_back_to_direct_delivery() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    let first = world.proxy.deliveries_of("deliver").remove(0).1;
    world.proxy.answers([DepositStatus::Retry { reason: "expired".to_string() }]);

    world.parked_pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Pending);
    assert_eq!(
        world.log.of(&message),
        vec![ConversationDeliveryState::Deposited, ConversationDeliveryState::Pending]
    );

    // The next pass delivers directly, with the envelope already made: the
    // ratchet does not move.
    world.proxy.script("deliver", BOB, Next::Direct(json!({ "message_id": message })));
    world.pass().await;
    let attempts = world.proxy.deliveries_of("deliver");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].1, first, "the same envelope is sent again");
    assert_eq!(world.state(&message), ConversationDeliveryState::Delivered);
}

#[tokio::test]
async fn a_deposit_still_waiting_past_its_deadline_goes_back_to_direct_delivery() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    let item = world.rows().into_iter().find(|r| r.parked.is_some()).unwrap();
    let parked = item.parked.unwrap();
    world.store.queue().park(item.id, now_ms(), 1, now_ms() + 3, &parked.context).unwrap();
    time::sleep(Duration::from_millis(10)).await;
    world.proxy.answers([DepositStatus::Waiting]);

    world.service.drain_parked_once().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Pending);
}

#[tokio::test]
async fn a_parked_message_older_than_the_age_limit_fails() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    world
        .store
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET received_at = ?1 WHERE id = ?2",
            rusqlite::params![now_ms() - THIRTY_ONE_DAYS_MS, message],
        )
        .unwrap();

    world.parked_pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Failed);
    assert_eq!(world.proxy.polled.load(Ordering::SeqCst), 0, "nothing is asked for it");
}

#[tokio::test]
async fn a_deleted_deposited_message_has_its_row_completed_without_a_poll() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    let conv = world.store.get_message(&message).unwrap().unwrap().conversation_id;
    assert!(world.store.delete_message(&conv, &message, now_ms()).unwrap());

    world.parked_pass().await;

    assert!(world.rows().is_empty());
    assert_eq!(world.proxy.polled.load(Ordering::SeqCst), 0);
    assert_eq!(world.state(&message), ConversationDeliveryState::Failed);
}

/// What a person does while the worker waits: deletes `message`.
fn deleting(world: &World, message: &str) -> Hook {
    let store = world.store.clone();
    let message = message.to_string();
    Hook(Box::new(move || {
        let conv = store.get_message(&message).unwrap().unwrap().conversation_id;
        assert!(store.delete_message(&conv, &message, now_ms()).unwrap());
    }))
}

#[tokio::test]
async fn a_message_deleted_while_the_deposit_is_made_stays_deleted() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    *world.proxy.during_deliver.lock().unwrap() = Some(deleting(&world, &message));

    world.pass().await;
    world.pass().await;

    let stored = world.store.get_message(&message).unwrap().unwrap();
    assert_eq!(stored.state, ConversationDeliveryState::Failed);
    assert_eq!(stored.last_error.as_deref(), Some("deleted before delivery"));
    assert!(world.rows().is_empty(), "nothing is left to wait for");
    assert!(world.log.of(&message).is_empty(), "the app is never told it was deposited");
    assert_eq!(world.proxy.deliveries_of("deliver").len(), 1, "nothing is sent again");
}

#[tokio::test]
async fn a_message_deleted_while_the_poll_runs_is_not_sent_again() {
    let world = world().await;
    world.proxy.script("deliver", BOB, Next::Deposit);
    let message = world.send_direct().await;
    world.pass().await;
    world.proxy.answers([DepositStatus::Retry { reason: "expired".to_string() }]);
    *world.proxy.during_poll.lock().unwrap() = Some(deleting(&world, &message));

    world.parked_pass().await;
    world.pass().await;
    world.pass().await;

    let stored = world.store.get_message(&message).unwrap().unwrap();
    assert_eq!(stored.state, ConversationDeliveryState::Failed, "the delete is not undone");
    assert!(stored.body.is_empty(), "the body stays gone");
    assert!(world.rows().is_empty());
    assert_eq!(world.proxy.deliveries_of("deliver").len(), 1, "no send of an empty body");
    assert_eq!(world.log.of(&message), vec![ConversationDeliveryState::Deposited]);
}

// -- groups -----------------------------------------------------------------

async fn group_with_bob_and_carol(world: &World) -> (String, String) {
    for (member, seed) in [(BOB, 88u8), (CAROL, 89u8)] {
        let key = SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes();
        world
            .store
            .upsert_session(
                &SessionRow {
                    peer_address: member.to_string(),
                    pinned_sig_key: key,
                    state: vec![1],
                },
                now_ms(),
            )
            .unwrap();
    }
    let group = world.service.create_group(ME).await.unwrap();
    world.service.add_member(ME, &group, BOB).await.unwrap();
    world.service.add_member(ME, &group, CAROL).await.unwrap();
    let message =
        world.service.send(ME, &group, "text/plain", b"to the group".to_vec()).await.unwrap();
    (group, message)
}

#[tokio::test]
async fn one_member_deposited_and_one_delivered_leaves_the_group_message_pending() {
    let world = world().await;
    let (_group, message) = group_with_bob_and_carol(&world).await;
    world.proxy.script("group-push", BOB, Next::Deposit);

    world.pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Pending);
    assert_eq!(
        world.store.recipient_state(&message, BOB).unwrap(),
        Some(ConversationDeliveryState::Deposited)
    );
    assert_eq!(
        world.store.recipient_state(&message, CAROL).unwrap(),
        Some(ConversationDeliveryState::Delivered)
    );

    world.proxy.answers([DepositStatus::Delivered]);
    world.parked_pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Delivered);
}

#[tokio::test]
async fn a_rejected_member_makes_the_group_message_fail_when_every_member_has_settled() {
    let world = world().await;
    let (_group, message) = group_with_bob_and_carol(&world).await;
    world.proxy.script("group-push", BOB, Next::Deposit);
    world.pass().await;
    world.proxy.answers([DepositStatus::Rejected { code: "not_eligible".to_string() }]);

    world.parked_pass().await;

    assert_eq!(world.state(&message), ConversationDeliveryState::Failed);
}

#[tokio::test]
async fn the_relay_push_never_deposits() {
    let world = world().await;
    let (group, _message) = group_with_bob_and_carol(&world).await;
    let entry_id = world.store.get_message(&_message).unwrap().unwrap().entry_id.unwrap();
    let entry = world.store.wire_entry(&entry_id).unwrap().unwrap();
    assert_eq!(entry.conversation_id, group);

    world.service.push_group_entry(&world.store, ME, BOB, &entry, None).await.unwrap();

    assert!(world.proxy.deliveries_of("group-push").is_empty(), "no deposit path");
    assert!(world.proxy.invoked.lock().unwrap().iter().any(|(m, t)| m == "group-push" && t == BOB));
}

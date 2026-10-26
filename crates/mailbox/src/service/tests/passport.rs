//! `register` with a child token: admission, and the limits a lease gives.

use syneroym_core::mailbox::{QueueLimits, RegisterAnswer};
use syneroym_mailbox_format::{DEDICATED_MAX_TTL_SECS, OPEN_MAX_TTL_SECS};
use syneroym_passport::{CapabilityToken, PassportId, facts};
use syneroym_rpc::NativeResponse;

use super::*;
use crate::test_support::{Operator, child_for};

/// Open limits of these tests: two messages in a queue, two queues.
const OPEN_MESSAGES: u32 = 2;
/// Long enough that a certificate outlives every passport in these tests.
const CERT_LIFE: u64 = 1_000_000;
const PASSPORT_LIFE: u64 = 1_000;

struct World {
    dir: tempfile::TempDir,
    service: MailboxService,
    anchors: Arc<Anchors>,
    operator: Operator,
    owner: Identity,
    role: MailboxRole,
}

impl World {
    fn new() -> Self {
        let role = MailboxRole {
            max_queue_messages: OPEN_MESSAGES,
            max_queues: 2,
            ..MailboxRole::default()
        };
        let dir = tempdir().unwrap();
        let anchors = Arc::new(Anchors::new());
        let operator = Operator::unowned();
        let service = open_service(&role, &dir, &operator, &anchors);
        Self { dir, service, anchors, operator, owner: Identity::generate().unwrap(), role }
    }

    fn member(&self) -> Member {
        let member = Member::new(CERT_LIFE);
        self.anchors.publish(&member.service_id());
        member
    }

    fn passport(&self, facts: &[(&str, u64)], now: u64) -> CapabilityToken {
        self.operator.passport(&self.owner, facts, PASSPORT_LIFE, now)
    }

    fn child(&self, passport: &CapabilityToken, m: &Member, now: u64) -> String {
        child_for(&self.owner, passport, m, PASSPORT_LIFE, now)
    }

    /// Reopens the service on the same directory, as a restart does.
    fn restart(&mut self) {
        self.service = open_service(&self.role, &self.dir, &self.operator, &self.anchors);
    }

    async fn register(
        &self,
        member: &Member,
        child_token: Option<&str>,
        now: u64,
    ) -> RpcResult<NativeResponse> {
        let params = json!({ "record": record(member, &self.operator.host.node_did),
                             "child_token": child_token });
        self.service.dispatch_at(invocation(METHOD_REGISTER, params, member.caller()), now).await
    }

    async fn limits_of(&self, member: &Member, token: Option<&str>, now: u64) -> QueueLimits {
        let res = self.register(member, token, now).await.unwrap();
        serde_json::from_value::<RegisterAnswer>(res.payload).unwrap().limits
    }

    async fn deposit(&self, sender: &Member, to: &Member, now: u64) -> RpcResult<NativeResponse> {
        let deposit = sealed_deposit(sender, to, 3600, now).deposit;
        let inv = invocation(METHOD_DEPOSIT, json!(deposit), sender.caller());
        self.service.dispatch_at(inv, now).await
    }
}

fn open_service(
    role: &MailboxRole,
    dir: &tempfile::TempDir,
    operator: &Operator,
    anchors: &Arc<Anchors>,
) -> MailboxService {
    MailboxService::open(role, dir.path(), &operator.host, anchors.clone()).unwrap()
}

fn custom_error(res: RpcResult<NativeResponse>) -> (i32, serde_json::Value) {
    match res {
        Err(RpcError::Custom(code, _, data)) => (code, data.unwrap_or_default()),
        other => panic!("expected a custom error, got {other:?}"),
    }
}

fn now() -> u64 {
    syneroym_passport::now_secs()
}

#[tokio::test]
async fn a_child_token_raises_the_limits_and_no_token_leaves_them_open() {
    let w = World::new();
    let now = now();
    let (with, without) = (w.member(), w.member());
    let passport = w.passport(&[(facts::MAX_MESSAGES, 5), (facts::MAX_BYTES, 9_000_000)], now);

    let raised = w.limits_of(&with, Some(&w.child(&passport, &with, now)), now).await;
    assert_eq!(raised.max_messages, 5);
    assert_eq!(raised.max_bytes, 9_000_000);
    assert_eq!(raised.max_ttl_secs, OPEN_MAX_TTL_SECS, "a missing fact is the open limit");

    let open = w.limits_of(&without, None, now).await;
    assert_eq!(open.max_messages, OPEN_MESSAGES);
    assert_eq!(open, w.service.store.limits().queue);
}

#[tokio::test]
async fn a_refused_child_token_answers_its_code_and_writes_no_queue() {
    let w = World::new();
    let now = now();
    let member = w.member();
    let passport = w.passport(&[], now);
    let id = PassportId::of(&passport).unwrap();
    w.service.passports.leases().deny(&id, now).unwrap();

    let res = w.register(&member, Some(&w.child(&passport, &member, now)), now).await;
    let (code, data) = custom_error(res);
    assert_eq!(code, codes::PASSPORT_REFUSED);
    assert_eq!(data["kind"], "passport_refused");
    assert_eq!(data["code"], "denied");

    // No queue was written: a deposit finds no destination.
    let sender = w.member();
    let (code, _) = custom_error(w.deposit(&sender, &member, now).await);
    assert_eq!(code, codes::UNKNOWN_DESTINATION);
}

#[tokio::test]
async fn a_lease_holds_more_messages_and_a_denied_passport_falls_back() {
    let w = World::new();
    let now = now();
    let (receiver, sender) = (w.member(), w.member());
    let passport = w.passport(&[(facts::MAX_MESSAGES, 5)], now);
    w.register(&receiver, Some(&w.child(&passport, &receiver, now)), now).await.unwrap();

    for _ in 0..4 {
        w.deposit(&sender, &receiver, now).await.expect("stored under the raised limit");
    }

    let id = PassportId::of(&passport).unwrap();
    assert_eq!(w.service.passports.leases().deny(&id, now).unwrap(), 1);
    let (code, _) = custom_error(w.deposit(&sender, &receiver, now).await);
    assert_eq!(code, codes::QUEUE_FULL, "the queue is back at the open limit");
}

#[tokio::test]
async fn a_master_with_a_lease_is_not_refused_by_the_queue_count() {
    let w = World::new();
    let now = now();
    let (a, b) = (w.member(), w.member());
    w.register(&a, None, now).await.unwrap();
    w.register(&b, None, now).await.unwrap();

    let stranger = w.member();
    let (code, _) = custom_error(w.register(&stranger, None, now).await);
    assert_eq!(code, codes::MAILBOX_FULL);

    let holder = w.member();
    let passport = w.passport(&[], now);
    w.register(&holder, Some(&w.child(&passport, &holder, now)), now)
        .await
        .expect("a lease exempts the master from the queue count");
}

#[tokio::test]
async fn a_fact_above_the_cap_is_cut_and_one_below_the_open_limit_is_ignored() {
    let w = World::new();
    let now = now();
    let (big, small) = (w.member(), w.member());

    let big_pass = w.passport(&[(facts::MAX_TTL_SECS, 90 * 24 * 3600)], now);
    let cut = w.limits_of(&big, Some(&w.child(&big_pass, &big, now)), now).await;
    assert_eq!(cut.max_ttl_secs, DEDICATED_MAX_TTL_SECS);

    let small_pass = w.passport(&[(facts::MAX_MESSAGES, 1), (facts::MAX_TTL_SECS, 3_600)], now);
    let kept = w.limits_of(&small, Some(&w.child(&small_pass, &small, now)), now).await;
    assert_eq!(kept.max_messages, OPEN_MESSAGES);
    assert_eq!(kept.max_ttl_secs, OPEN_MAX_TTL_SECS);
}

#[tokio::test]
async fn a_lease_survives_a_restart() {
    let mut w = World::new();
    let now = now();
    let (receiver, sender) = (w.member(), w.member());
    let passport = w.passport(&[(facts::MAX_MESSAGES, 5)], now);
    w.register(&receiver, Some(&w.child(&passport, &receiver, now)), now).await.unwrap();

    w.restart();
    for _ in 0..3 {
        w.deposit(&sender, &receiver, now).await.expect("the lease is read from its file");
    }
}

#[tokio::test]
async fn the_open_limits_return_when_the_lease_ends() {
    let w = World::new();
    let now = now();
    let (receiver, sender) = (w.member(), w.member());
    let passport = w.passport(&[(facts::MAX_MESSAGES, 5)], now);
    w.register(&receiver, Some(&w.child(&passport, &receiver, now)), now).await.unwrap();
    for _ in 0..3 {
        w.deposit(&sender, &receiver, now).await.unwrap();
    }

    let later = now + PASSPORT_LIFE + 1;
    let (code, _) = custom_error(w.deposit(&sender, &receiver, later).await);
    assert_eq!(code, codes::QUEUE_FULL, "the next deposit sees the open limits");
    assert_eq!(w.limits_of(&receiver, None, later).await.max_messages, OPEN_MESSAGES);
}

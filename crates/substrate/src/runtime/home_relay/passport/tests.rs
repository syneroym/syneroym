use std::{
    collections::VecDeque,
    future::{self, Future},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use iroh::RelayUrl;
use serde_json::Map;
use syneroym_control_plane::HomeRelayControl;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    CapabilityToken, PassportId, RELAY_HOME, child_token, issue_passport,
    wire::{ChallengeResponse, HomeRelayStatus, RegisterRequest, RegisterResponse},
};
use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

use super::{
    AgentInit, PassportControl,
    client::{Failure, RegistrationIo},
    start_agent,
};
pub(super) use super::{client, client::Target, store::StoredPassport};

mod boot;
mod refusals;
mod renew;
mod set;
mod store;

pub(super) const START_SECS: u64 = 1_700_000_000;

pub(super) const FIXED_RELAY: &str = "http://127.0.0.1:7964/";
pub(super) const INFO_URL: &str = "https://relay.example:7974";

pub(super) enum Scripted<T> {
    Answer(Result<T, Failure>),
    Hang,
    HangNotify(Arc<tokio::sync::Notify>, Result<T, Failure>),
}

#[derive(Clone)]
pub(super) struct FakeIo(Arc<FakeIoInner>);

struct FakeIoInner {
    challenges: Mutex<VecDeque<Scripted<ChallengeResponse>>>,
    registers: Mutex<VecDeque<Scripted<RegisterResponse>>>,
    recorded_challenges: Mutex<Vec<Target>>,
    recorded_registers: Mutex<Vec<RegisterRequest>>,
    closed: CancellationToken,
    started: Instant,
    clock_offset_secs: std::sync::atomic::AtomicU64,
}

impl FakeIo {
    pub(super) fn new() -> Self {
        Self(Arc::new(FakeIoInner {
            challenges: Mutex::new(VecDeque::new()),
            registers: Mutex::new(VecDeque::new()),
            recorded_challenges: Mutex::new(Vec::new()),
            recorded_registers: Mutex::new(Vec::new()),
            closed: CancellationToken::new(),
            started: Instant::now(),
            clock_offset_secs: std::sync::atomic::AtomicU64::new(0),
        }))
    }

    pub(super) fn advance_clock(&self, secs: u64) {
        self.0.clock_offset_secs.fetch_add(secs, Ordering::SeqCst);
    }

    pub(super) fn push_challenge(&self, item: Scripted<ChallengeResponse>) {
        self.0.challenges.lock().unwrap().push_back(item);
    }

    pub(super) fn push_register(&self, item: Scripted<RegisterResponse>) {
        self.0.registers.lock().unwrap().push_back(item);
    }

    pub(super) fn push_register_hanging(
        &self,
        notify: Arc<tokio::sync::Notify>,
        result: Result<RegisterResponse, Failure>,
    ) {
        self.push_register(Scripted::HangNotify(notify, result));
    }

    pub(super) fn push_challenge_ok(&self, hex: &str) {
        self.push_challenge(Scripted::Answer(Ok(ChallengeResponse {
            challenge: hex.to_owned(),
            expires_in_secs: 60,
        })));
    }

    pub(super) fn push_register_ok(&self, passport_id: &PassportId, lease_ends_at: u64) {
        self.push_register(Scripted::Answer(Ok(RegisterResponse {
            passport_id: passport_id.clone(),
            lease_ends_at,
        })));
    }

    pub(super) fn push_register_err(&self, failure: Failure) {
        self.push_register(Scripted::Answer(Err(failure)));
    }

    pub(super) fn push_challenge_err(&self, failure: Failure) {
        self.push_challenge(Scripted::Answer(Err(failure)));
    }

    pub(super) fn challenges_received(&self) -> usize {
        self.0.recorded_challenges.lock().unwrap().len()
    }

    pub(super) fn registers_received(&self) -> usize {
        self.0.recorded_registers.lock().unwrap().len()
    }

    pub(super) fn close(&self) {
        self.0.closed.cancel();
    }

    pub(super) fn now_secs(&self) -> u64 {
        START_SECS
            + self.0.clock_offset_secs.load(Ordering::SeqCst)
            + self.0.started.elapsed().as_secs()
    }
}

impl RegistrationIo for FakeIo {
    fn challenge(
        &self,
        target: &Target,
    ) -> impl Future<Output = Result<ChallengeResponse, Failure>> + Send {
        self.0.recorded_challenges.lock().unwrap().push(target.clone());
        let item = self.0.challenges.lock().unwrap().pop_front();
        async move {
            match item {
                Some(Scripted::Answer(res)) => res,
                Some(Scripted::Hang) => {
                    future::pending::<()>().await;
                    unreachable!()
                }
                Some(Scripted::HangNotify(notify, res)) => {
                    notify.notified().await;
                    res
                }
                None => Err(Failure::Unreachable("no scripted challenge".into())),
            }
        }
    }

    fn register(
        &self,
        _target: &Target,
        request: &RegisterRequest,
    ) -> impl Future<Output = Result<RegisterResponse, Failure>> + Send {
        self.0.recorded_registers.lock().unwrap().push(request.clone());
        let item = self.0.registers.lock().unwrap().pop_front();
        async move {
            match item {
                Some(Scripted::Answer(res)) => res,
                Some(Scripted::Hang) => {
                    future::pending::<()>().await;
                    unreachable!()
                }
                Some(Scripted::HangNotify(notify, res)) => {
                    notify.notified().await;
                    res
                }
                None => Err(Failure::Unreachable("no scripted register".into())),
            }
        }
    }

    async fn closed(&self) {
        self.0.closed.cancelled().await;
    }

    fn now_secs(&self) -> u64 {
        Self::now_secs(self)
    }
}

pub(super) fn did_of(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

pub(super) struct TestFixture {
    pub(super) signer: Identity,
    #[expect(dead_code, reason = "test harness field retained for keypair lifecycle")]
    pub(super) host: Identity,
    pub(super) owner: Identity,
    pub(super) node: Identity,
    pub(super) host_did: String,
    pub(super) node_did: String,
    pub(super) owner_did: String,
    pub(super) passport: CapabilityToken,
    pub(super) child: CapabilityToken,
    pub(super) passport_id: PassportId,
}

pub(super) fn test_fixture(life_secs: u64) -> TestFixture {
    let signer = Identity::generate().unwrap();
    let host = Identity::generate().unwrap();
    let owner = Identity::generate().unwrap();
    let node = Identity::generate().unwrap();
    let host_did = did_of(&host);
    let node_did = did_of(&node);
    let owner_did = did_of(&owner);
    let passport = issue_passport(
        &signer,
        &host_did,
        &RELAY_HOME,
        &owner_did,
        Map::new(),
        life_secs,
        START_SECS,
    )
    .unwrap();
    let passport_id = PassportId::of(&passport).unwrap();
    let child = child_token(&owner, &passport, &node_did, life_secs, START_SECS).unwrap();
    TestFixture {
        signer,
        host,
        owner,
        node,
        host_did,
        node_did,
        owner_did,
        passport,
        child,
        passport_id,
    }
}

impl TestFixture {
    #[expect(dead_code, reason = "test helper retained for child token minting")]
    pub(super) fn make_child(&self, life_secs: u64) -> CapabilityToken {
        child_token(&self.owner, &self.passport, &self.node_did, life_secs, START_SECS).unwrap()
    }

    pub(super) fn new_passport_for_node(
        &self,
        life_secs: u64,
    ) -> (CapabilityToken, PassportId, CapabilityToken) {
        let signer = Identity::generate().unwrap();
        let owner = Identity::generate().unwrap();
        let owner_did = did_of(&owner);
        let passport = issue_passport(
            &signer,
            &self.host_did,
            &RELAY_HOME,
            &owner_did,
            Map::new(),
            life_secs,
            START_SECS,
        )
        .unwrap();
        let passport_id = PassportId::of(&passport).unwrap();
        let child = child_token(&owner, &passport, &self.node_did, life_secs, START_SECS).unwrap();
        (passport, passport_id, child)
    }
}

pub(super) struct TestAgent {
    #[expect(dead_code, reason = "test harness fields retained for test lifecycle")]
    pub(super) io: FakeIo,
    pub(super) control: Arc<PassportControl>,
    #[expect(dead_code, reason = "test harness fields retained for test lifecycle")]
    pub(super) status_rx: watch::Receiver<Option<HomeRelayStatus>>,
    pub(super) boot_done: Arc<AtomicBool>,
    pub(super) path: PathBuf,
    pub(super) handle: tokio::task::JoinHandle<()>,
    pub(super) _dir: Option<tempfile::TempDir>,
}

impl TestAgent {
    pub(super) fn boot_done(&self) -> bool {
        self.boot_done.load(Ordering::SeqCst)
    }

    pub(super) fn status(&self) -> Option<HomeRelayStatus> {
        self.control.status()
    }

    fn start_inner(
        io: FakeIo,
        fixture: &TestFixture,
        path: PathBuf,
        coordinator_on_parent: bool,
        dir: Option<tempfile::TempDir>,
    ) -> Self {
        let (boot_tx, boot_rx) = tokio::sync::oneshot::channel();
        let boot_done = Arc::new(AtomicBool::new(false));
        let boot_flag = boot_done.clone();
        tokio::spawn(async move {
            if boot_rx.await.is_ok() {
                boot_flag.store(true, Ordering::SeqCst);
            }
        });
        let fixed_url: RelayUrl = FIXED_RELAY.parse().unwrap();
        let io_clock = io.clone();
        let (control, status_rx, handle) = start_agent(
            io.clone(),
            AgentInit {
                fixed_url,
                path: path.clone(),
                node_did: fixture.node_did.clone(),
                secret_key: fixture.node.to_bytes(),
                coordinator_on_parent,
                boot_done: boot_tx,
            },
            Arc::new(move || io_clock.now_secs()),
        );
        Self { io, control, status_rx, boot_done, path, handle, _dir: dir }
    }

    pub(super) fn start(
        io: FakeIo,
        fixture: &TestFixture,
        stored: Option<StoredPassport>,
        coordinator_on_parent: bool,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home_relay_passport.json");
        if let Some(stored) = &stored {
            super::store::save(&path, stored).unwrap();
        }
        Self::start_inner(io, fixture, path, coordinator_on_parent, Some(dir))
    }

    pub(super) fn start_at_path(
        io: FakeIo,
        fixture: &TestFixture,
        path: PathBuf,
        coordinator_on_parent: bool,
    ) -> Self {
        Self::start_inner(io, fixture, path, coordinator_on_parent, None)
    }
}

//! The background agent that holds the home-relay passport, registers with
//! the private relay, and renews the lease.

use std::{future, path::PathBuf, sync::Arc, time::Duration};

use client::{Failure, LiveIo, RegistrationIo, Target, register_once};
use iroh::RelayUrl;
use metrics::gauge;
use store::StoredPassport;
use syneroym_control_plane::HomeRelayControl;
use syneroym_identity::Identity;
use syneroym_passport::{
    PresentedToken, RELAY_HOME, RegisterError, inspect_child_token,
    wire::{
        ErrorBody, HomeRelaySetRequest, HomeRelayStatus, RegisterResponse, RetryClass,
        UNREACHABLE_CODE,
    },
};
use syneroym_router::HomeRelay;
use syneroym_rpc::{RpcError, RpcResult};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time,
};
use tracing::{debug, info, warn};

use super::{Backoff, WarnLimiter};

mod client;
mod store;

#[cfg(test)]
mod tests;

/// Registration tries at boot before the relay is inserted anyway.
const BOOT_TRIES: usize = 3;
/// The longest the relay waits for the boot registration.
const BOOT_BUDGET: Duration = Duration::from_secs(30);
/// Retry delays after a failure that may pass.
const RETRY_INITIAL: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(600);
/// Retry delay after a refusal that will not change soon.
const SETTLED_RETRY_SECS: u64 = 3_600;
/// The soonest a renewal runs after the last attempt.
const MIN_RENEW_DELAY_SECS: u64 = 60;
/// Warn when the access end is this close.
const EXPIRY_WARNING_SECS: u64 = 7 * 24 * 3_600;
/// The gauge of the access end, in Unix seconds.
const ACCESS_END_GAUGE: &str = "substrate.home_relay.access_ends_at_secs";
/// Sleep cap so wall-clock jumps or host suspend/sleep are handled within 5
/// minutes.
const SLEEP_CAP: Duration = Duration::from_secs(300);
/// When the agent channel is closed.
const STOPPED: &str = "the home-relay task has stopped (the endpoint is closed)";

pub(super) struct Start {
    pub(super) home_relay: HomeRelay,
    pub(super) fixed_url: RelayUrl,
    pub(super) path: PathBuf,
    pub(super) node_did: String,
    pub(super) secret_key: [u8; 32],
    pub(super) coordinator_on_parent: bool,
    pub(super) boot_done: oneshot::Sender<()>,
}

pub(super) struct AgentInit {
    pub(super) fixed_url: RelayUrl,
    pub(super) path: PathBuf,
    pub(super) node_did: String,
    pub(super) secret_key: [u8; 32],
    pub(super) coordinator_on_parent: bool,
    pub(super) boot_done: oneshot::Sender<()>,
}

/// Spawns the agent with `LiveIo` and returns its control handle.
pub(super) fn start(start: Start) -> Arc<dyn HomeRelayControl> {
    let io = LiveIo::new(start.home_relay);
    let (control, _, _) = start_agent(
        io,
        AgentInit {
            fixed_url: start.fixed_url,
            path: start.path,
            node_did: start.node_did,
            secret_key: start.secret_key,
            coordinator_on_parent: start.coordinator_on_parent,
            boot_done: start.boot_done,
        },
        Arc::new(syneroym_passport::now_secs),
    );
    control
}

fn start_agent<I: RegistrationIo>(
    io: I,
    init: AgentInit,
    now_secs: Arc<dyn Fn() -> u64 + Send + Sync>,
) -> (Arc<PassportControl>, watch::Receiver<Option<HomeRelayStatus>>, tokio::task::JoinHandle<()>) {
    let (commands, rx) = mpsc::channel(4);
    let (status_tx, status_rx) = watch::channel(None);
    let agent = Agent::new(
        io,
        &init.secret_key,
        init.node_did.clone(),
        init.fixed_url.clone(),
        init.path,
        init.coordinator_on_parent,
        status_tx,
        rx,
    );
    let handle = tokio::spawn(agent.run(init.boot_done));
    (
        Arc::new(PassportControl {
            node_did: init.node_did,
            fixed_url: init.fixed_url,
            commands,
            status: status_rx.clone(),
            now_secs,
        }),
        status_rx,
        handle,
    )
}

enum Command {
    Set {
        stored: StoredPassport,
        presented: PresentedToken,
        reply: oneshot::Sender<RpcResult<HomeRelayStatus>>,
    },
}

pub(super) struct PassportControl {
    node_did: String,
    fixed_url: RelayUrl,
    commands: mpsc::Sender<Command>,
    status: watch::Receiver<Option<HomeRelayStatus>>,
    now_secs: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for PassportControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassportControl")
            .field("node_did", &self.node_did)
            .field("fixed_url", &self.fixed_url)
            .finish()
    }
}

#[async_trait::async_trait]
impl HomeRelayControl for PassportControl {
    async fn set_passport(&self, request: HomeRelaySetRequest) -> RpcResult<HomeRelayStatus> {
        let (stored, presented) = StoredPassport::from_request(
            request,
            &self.node_did,
            &self.fixed_url,
            (self.now_secs)(),
        )?;
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Set { stored, presented, reply })
            .await
            .map_err(|_| RpcError::InternalError(STOPPED.into()))?;
        answer.await.map_err(|_| RpcError::InternalError(STOPPED.into()))?
    }

    fn status(&self) -> Option<HomeRelayStatus> {
        self.status.borrow().clone()
    }
}

/// The stored passport and what is known about it.
struct Current {
    stored: StoredPassport,
    presented: PresentedToken,
    active: bool,
    lease_ends_at: Option<u64>,
    last_attempt_at: Option<u64>,
    last_error: Option<ErrorBody>,
    /// Unix seconds of the next attempt. `None`: no attempt is planned.
    next_attempt_at: Option<u64>,
    ended_reported: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Leased,
    Settled,
    Retry,
}

struct Closed;

struct Agent<I> {
    io: I,
    node: Identity,
    node_did: String,
    fixed_url: RelayUrl,
    path: PathBuf,
    coordinator_on_parent: bool,
    current: Option<Current>,
    retry: Backoff,
    warn_retry: WarnLimiter,
    last_ending_warn_secs: Option<u64>,
    status_tx: watch::Sender<Option<HomeRelayStatus>>,
    commands: mpsc::Receiver<Command>,
}

impl<I: RegistrationIo> Agent<I> {
    #[expect(clippy::too_many_arguments, reason = "internal agent construction wires all state")]
    fn new(
        io: I,
        secret_key: &[u8; 32],
        node_did: String,
        fixed_url: RelayUrl,
        path: PathBuf,
        coordinator_on_parent: bool,
        status_tx: watch::Sender<Option<HomeRelayStatus>>,
        commands: mpsc::Receiver<Command>,
    ) -> Self {
        Self {
            io,
            node: Identity::from_bytes(secret_key),
            node_did,
            fixed_url,
            path,
            coordinator_on_parent,
            current: None,
            retry: Backoff::new(RETRY_INITIAL, RETRY_MAX),
            warn_retry: WarnLimiter::default(),
            last_ending_warn_secs: None,
            status_tx,
            commands,
        }
    }

    async fn run(mut self, boot_done: oneshot::Sender<()>) {
        self.load_current();
        let now = self.io.now_secs();
        self.warn_if_ending(now);
        let can_boot =
            self.current.as_ref().is_some_and(|c| c.active && now < c.presented.access_ends_at);
        if can_boot && self.boot_round().await.is_err() {
            return;
        }
        let _ = boot_done.send(());
        loop {
            let wait_dur = self.wait();
            let sleep = async {
                match wait_dur {
                    Some(d) => time::sleep(d).await,
                    None => future::pending::<()>().await,
                }
            };
            tokio::select! {
                () = self.io.closed() => return,
                command = self.commands.recv() => match command {
                    None => return,
                    Some(Command::Set { stored, presented, reply }) => {
                        if self.apply_set(stored, presented, reply).await.is_err() {
                            return;
                        }
                    }
                },
                () = sleep => {
                    let now = self.io.now_secs();
                    let due = self
                        .current
                        .as_ref()
                        .and_then(|c| c.next_attempt_at)
                        .is_some_and(|target| now >= target);
                    if due && self.attempt_or_close().await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    fn wait(&self) -> Option<Duration> {
        let current = self.current.as_ref()?;
        let target = current.next_attempt_at?;
        let now = self.io.now_secs();
        let remaining = Duration::from_secs(target.saturating_sub(now));
        Some(remaining.min(SLEEP_CAP))
    }

    async fn boot_round(&mut self) -> Result<(), Closed> {
        let budget = time::sleep(BOOT_BUDGET);
        tokio::pin!(budget);
        for try_no in 1..=BOOT_TRIES {
            let outcome = tokio::select! {
                () = &mut budget => {
                    self.boot_budget_spent();
                    return Ok(());
                }
                outcome = self.attempt_or_close() => outcome?,
            };
            if outcome != Outcome::Retry || try_no == BOOT_TRIES {
                return Ok(());
            }
            tokio::select! {
                () = &mut budget => return Ok(()),
                () = self.io.closed() => return Err(Closed),
                () = time::sleep(self.wait().unwrap_or(Duration::ZERO)) => {}
            }
        }
        Ok(())
    }

    fn boot_budget_spent(&mut self) {
        let now = self.io.now_secs();
        if let Some(current) = self.current.as_mut() {
            current.last_error = Some(ErrorBody {
                code: UNREACHABLE_CODE.to_owned(),
                message: "no answer within 30 s at boot".to_owned(),
            });
            current.last_attempt_at = Some(now);
            let next = now + self.retry.next().as_secs().max(1);
            current.next_attempt_at = Some(next).filter(|t| *t < current.presented.access_ends_at);
        }
        self.publish();
    }

    async fn attempt_or_close(&mut self) -> Result<Outcome, Closed> {
        if let Some(outcome) = self.end_if_access_ended() {
            return Ok(outcome);
        }
        let result = tokio::select! {
            () = self.io.closed() => return Err(Closed),
            result = self.registration() => result,
        };
        Ok(result.map_or(Outcome::Settled, |r| self.record(r)))
    }

    fn end_if_access_ended(&mut self) -> Option<Outcome> {
        let Some(current) = self.current.as_mut() else {
            return Some(Outcome::Settled);
        };
        if !current.active {
            return Some(Outcome::Settled);
        }
        let now = self.io.now_secs();
        if now >= current.presented.access_ends_at {
            current.next_attempt_at = None;
            current.last_error = Some(ErrorBody::from(&RegisterError::Expired));
            if !current.ended_reported {
                current.ended_reported = true;
                gauge!(ACCESS_END_GAUGE).set(0.0);
                warn!(
                    "the home-relay passport access has ended; ask the relay operator for a new \
                     one"
                );
                self.publish();
            }
            return Some(Outcome::Settled);
        }
        None
    }

    async fn registration(&self) -> Option<Result<RegisterResponse, Failure>> {
        let current = self.current.as_ref()?;
        if !current.active {
            return None;
        }
        let target = Target {
            info_url: current.stored.info_url.clone(),
            ca_cert_pem: current.stored.ca_cert_pem.clone(),
        };
        Some(
            register_once(
                &self.io,
                &target,
                &self.node,
                &self.node_did,
                &current.presented.host_node_did,
                &current.stored.token,
            )
            .await,
        )
    }

    fn record(&mut self, result: Result<RegisterResponse, Failure>) -> Outcome {
        let now = self.io.now_secs();
        if let Some(current) = self.current.as_mut() {
            current.last_attempt_at = Some(now);
        }
        let outcome = match result {
            Ok(resp) => self.on_lease(resp, now),
            Err(failure) => self.on_failure(failure, now),
        };
        self.warn_if_ending(now);
        self.publish();
        outcome
    }

    fn on_lease(&mut self, resp: RegisterResponse, now: u64) -> Outcome {
        self.retry.reset();
        let Some(current) = self.current.as_mut() else {
            return Outcome::Settled;
        };
        if resp.passport_id != current.presented.passport_id {
            warn!(
                got = %resp.passport_id,
                expected = %current.presented.passport_id,
                "coordinator returned different passport id in lease response"
            );
        }
        current.lease_ends_at = Some(resp.lease_ends_at);
        current.last_error = None;
        let renew = renew_at(now, resp.lease_ends_at, current.presented.access_ends_at);
        current.next_attempt_at = renew.or_else(|| {
            (now < current.presented.access_ends_at).then_some(current.presented.access_ends_at)
        });
        info!(lease_ends_at = resp.lease_ends_at, "home-relay lease granted");
        Outcome::Leased
    }

    fn on_failure(&mut self, failure: Failure, now: u64) -> Outcome {
        let body = failure.body();
        let class = failure.class();
        let Some(current) = self.current.as_mut() else {
            return Outcome::Settled;
        };
        if body.code == RegisterError::Denied.code() {
            current.lease_ends_at = None;
            gauge!(ACCESS_END_GAUGE).set(0.0);
        }
        current.last_error = Some(body.clone());
        let (next, outcome) = match class {
            RetryClass::Settled => {
                warn!(
                    code = %body.code,
                    message = %body.message,
                    "the coordinator refused the home-relay passport; trying again in an hour"
                );
                (now + SETTLED_RETRY_SECS, Outcome::Settled)
            }
            RetryClass::NewChallenge | RetryClass::Backoff => {
                let delay = self.retry.next().as_secs().max(1);
                if self.warn_retry.should_warn() {
                    warn!(code = %body.code, message = %body.message, "registration failed; retrying");
                } else {
                    debug!(code = %body.code, message = %body.message, "registration failed; retrying");
                }
                (now + delay, Outcome::Retry)
            }
        };
        current.next_attempt_at = if next < current.presented.access_ends_at {
            Some(next)
        } else if now < current.presented.access_ends_at {
            Some(current.presented.access_ends_at)
        } else {
            None
        };
        outcome
    }

    async fn apply_set(
        &mut self,
        stored: StoredPassport,
        presented: PresentedToken,
        reply: oneshot::Sender<RpcResult<HomeRelayStatus>>,
    ) -> Result<(), Closed> {
        let path = self.path.clone();
        let to_save = stored.clone();
        let save_res = tokio::task::spawn_blocking(move || store::save(&path, &to_save)).await;
        match save_res {
            Err(join_err) => {
                let _ = reply.send(Err(RpcError::InternalError(format!(
                    "cannot store the passport: {join_err}"
                ))));
                return Ok(());
            }
            Ok(Err(e)) => {
                let _ = reply.send(Err(RpcError::InternalError(format!(
                    "cannot store the passport: {e:#}"
                ))));
                return Ok(());
            }
            Ok(Ok(())) => {}
        }
        let active = self.install(stored, presented);
        if active && self.attempt_or_close().await.is_err() {
            return Err(Closed);
        }
        if let Some(status) = self.status_now() {
            let _ = reply.send(Ok(status));
        }
        Ok(())
    }

    fn install(&mut self, stored: StoredPassport, presented: PresentedToken) -> bool {
        let active = stored.is_for(&self.fixed_url);
        let was_active = self.current.as_ref().is_some_and(|c| c.active);
        let now = self.io.now_secs();
        let access_ends_at = presented.access_ends_at;
        let ended_already = active && now >= access_ends_at;
        let relay_url = stored.relay_url.clone();
        self.current = Some(Current {
            stored,
            presented,
            active,
            lease_ends_at: None,
            last_attempt_at: None,
            last_error: None,
            next_attempt_at: (active && !ended_already).then_some(now),
            ended_reported: ended_already,
        });
        self.retry.reset();
        self.last_ending_warn_secs = None;
        if active && now < access_ends_at {
            gauge!(ACCESS_END_GAUGE).set(access_ends_at as f64);
        } else if was_active || ended_already {
            gauge!(ACCESS_END_GAUGE).set(0.0);
        }
        if !active {
            info!(
                "the home-relay passport is for {relay_url}, but parent_coordinator.iroh.url is \
                 {}: it is kept and not used. Set the url to that relay and restart to use it",
                self.fixed_url,
            );
        } else if self.coordinator_on_parent {
            warn!(
                "the coordinator role also uses parent_coordinator.iroh.url as its parent relay, \
                 and a passport relay refuses its key: run the coordinator role on another node, \
                 or give it an open parent relay"
            );
        }
        self.publish();
        active
    }

    fn load_current(&mut self) {
        let Some(stored) = store::load(&self.path) else {
            return;
        };
        let now = self.io.now_secs();
        match inspect_child_token(&stored.token, &RELAY_HOME, &self.node_did, now) {
            Ok(presented) => {
                self.install(stored, presented);
            }
            Err(RegisterError::Expired) => {
                let start = stored.token.not_before_secs;
                match inspect_child_token(&stored.token, &RELAY_HOME, &self.node_did, start) {
                    Ok(presented) => {
                        self.install(stored, presented);
                        if let Some(current) = self.current.as_mut() {
                            current.last_error = Some(ErrorBody::from(&RegisterError::Expired));
                            current.next_attempt_at = None;
                            current.ended_reported = true;
                        }
                        gauge!(ACCESS_END_GAUGE).set(0.0);
                        warn!(
                            "the stored home-relay passport has ended; ask the relay operator for \
                             a new one"
                        );
                        self.publish();
                    }
                    Err(e) => {
                        warn!(
                            "the stored home-relay passport cannot be used: {e}; run `roymctl \
                             substrate home-relay set` again"
                        );
                    }
                }
            }
            Err(e) => {
                warn!(
                    "the stored home-relay passport cannot be used: {e}; run `roymctl substrate \
                     home-relay set` again"
                );
            }
        }
    }

    fn warn_if_ending(&mut self, now: u64) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        if !current.active {
            return;
        }
        let access_ends_at = current.presented.access_ends_at;
        if now >= access_ends_at {
            return;
        }
        if ending_soon(now, access_ends_at) {
            let day_elapsed =
                self.last_ending_warn_secs.is_none_or(|last| now.saturating_sub(last) >= 86_400);
            if day_elapsed {
                self.last_ending_warn_secs = Some(now);
                let days_left = access_ends_at.saturating_sub(now) / 86_400;
                warn!(
                    access_ends_at,
                    days_left,
                    "the home-relay passport ends soon; ask the relay operator for a new one and \
                     run `roymctl substrate home-relay set`"
                );
            }
        }
    }

    fn status_now(&self) -> Option<HomeRelayStatus> {
        let current = self.current.as_ref()?;
        Some(HomeRelayStatus {
            relay_url: current.stored.relay_url.clone(),
            info_url: current.stored.info_url.clone(),
            active: current.active,
            passport_id: current.presented.passport_id.clone(),
            host_node_did: current.presented.host_node_did.clone(),
            owner_did: current.presented.owner_did.clone(),
            access_ends_at: current.presented.access_ends_at,
            lease_ends_at: current.lease_ends_at,
            last_attempt_at: current.last_attempt_at,
            last_error: current.last_error.clone(),
            next_attempt_at: current.next_attempt_at,
        })
    }

    fn publish(&mut self) {
        let _ = self.status_tx.send_replace(self.status_now());
    }
}

/// A third of the lease left, never sooner than `MIN_RENEW_DELAY_SECS`.
/// `None` when the lease already reaches the access end: a renewal cannot
/// extend it.
fn renew_at(now: u64, lease_ends_at: u64, access_ends_at: u64) -> Option<u64> {
    if lease_ends_at >= access_ends_at {
        return None;
    }
    let third = lease_ends_at.saturating_sub(now) / 3;
    Some(lease_ends_at.saturating_sub(third).max(now + MIN_RENEW_DELAY_SECS))
}

/// True when the access end is `EXPIRY_WARNING_SECS` away or closer.
fn ending_soon(now: u64, access_ends_at: u64) -> bool {
    access_ends_at.saturating_sub(now) <= EXPIRY_WARNING_SECS
}

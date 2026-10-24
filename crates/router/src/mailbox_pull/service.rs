//! One hosted service: its inputs, its registration at the mailbox, and its
//! claim rounds.

use std::mem;

use serde_json::Value;
use sha2::{Digest, Sha256};
use syneroym_core::{
    dht_registry::SignedEndpointInfo,
    mailbox::{
        AckAnswer, AckEntry, AckOutcome, AckParams, ClaimAnswer, ClaimParams, HostedMailbox,
        InfoAnswer, MAX_CLAIM_BATCH, METHOD_ACK, METHOD_CLAIM, METHOD_INFO, METHOD_REGISTER,
        PullSummary, QueueLimits, RegisterAnswer, RegisterParams, codes,
    },
};
use syneroym_identity::DelegationCertificate;
use syneroym_mailbox_format::{
    DEDICATED_MAX_TTL_SECS, MAX_CLOCK_SKEW_SECS, MailboxSecretKey, OPEN_MAX_TTL_SECS, OpenPolicy,
};
use syneroym_rpc::{DEFAULT_PROXY_CALL_TIMEOUT, ProxyError};
use tokio::time::{self, Duration, Instant};
use tracing::{debug, warn};

use super::{
    MAX_CLAIM_ROUNDS, MailboxPuller, REREGISTER_INTERVAL, SERVED_RECORD_CHECK_INTERVAL,
    SERVED_RECORD_LOOKUP_TIMEOUT,
    message::Count,
    notes::{Note, Notes},
};
use crate::proxy::{AnchorCache, ServiceInstance};

/// How long a claimed message stays hidden at a mailbox, when the mailbox
/// did not say.
const DEFAULT_VISIBILITY_SECS: u64 = 300;
/// How long the `info` call after a register may take.
const INFO_TIMEOUT: Duration = Duration::from_secs(5);
/// A round stops this long before the visibility time ends, to leave room
/// for the acks.
const ACK_MARGIN_SECS: u64 = 30;
/// Acks are sent after this many handled messages.
pub(super) const ACK_FLUSH: usize = 8;

/// What the puller remembers about one service between passes.
#[derive(Default)]
pub(super) struct ServiceState {
    registered: Option<Registration>,
    /// The certificate the mailbox refused, and why. The service is skipped
    /// until the installed certificate differs in any field. A renewal keeps
    /// the instance key and may be issued in the same second, so the key and
    /// the issue time alone do not tell it apart.
    blocked_on: Option<(DelegationCertificate, Note)>,
    served_checked_at: Option<Instant>,
}

/// The last register of a service: accepted, or refused as an expired
/// record. Either way it is not sent again until the record or the mailbox
/// changes, or an hour passes.
struct Registration {
    mailbox: String,
    /// `pkarr_packet_hex` of the record registered.
    record_hex: String,
    at: Instant,
    /// `None` when the mailbox refused the record as expired. A queue made
    /// with an earlier record keeps the lifetime it had.
    max_ttl_secs: Option<u64>,
    /// From the mailbox's `info`, read at register; a default when it
    /// failed.
    visibility_secs: u64,
    /// The digest of the child token the service had at this register, also
    /// when the mailbox refused it, so a refused token is not sent on every
    /// pass.
    child_token_digest: Option<[u8; 32]>,
}

impl ServiceState {
    fn needs_register(&self, inputs: &Inputs) -> bool {
        match &self.registered {
            None => true,
            Some(r) => {
                r.mailbox != inputs.mailbox
                    || r.record_hex != inputs.record.pkarr_packet_hex
                    || r.child_token_digest != inputs.child_token_digest()
                    || r.at.elapsed() >= REREGISTER_INTERVAL
            }
        }
    }

    /// Whether the mailbox refused the record of the last register.
    fn record_refused(&self) -> bool {
        self.registered.as_ref().is_some_and(|r| r.max_ttl_secs.is_none())
    }

    /// Whether the mailbox accepted the last register.
    fn holds_queue(&self) -> bool {
        self.registered.as_ref().is_some_and(|r| r.max_ttl_secs.is_some())
    }
}

/// Everything one service needs for one pass.
pub(super) struct Inputs {
    pub(super) instance: ServiceInstance,
    pub(super) key: MailboxSecretKey,
    record: SignedEndpointInfo,
    pub(super) mailbox: String,
    /// The owner's child token for the mailbox passport, if the service has
    /// one.
    child_token: Option<String>,
}

impl Inputs {
    fn child_token_digest(&self) -> Option<[u8; 32]> {
        self.child_token.as_ref().map(|token| Sha256::digest(token.as_bytes()).into())
    }
}

/// The state of one service during one pass.
pub(super) struct ServicePass<'a> {
    pub(super) service_id: &'a str,
    /// The hosted-mailbox fact this pass started with.
    pub(super) fact: &'a HostedMailbox,
    pub(super) entry: &'a mut ServiceState,
    pub(super) notes: &'a mut Notes,
    pub(super) anchors: &'a mut AnchorCache,
    pub(super) summary: &'a mut PullSummary,
}

impl ServicePass<'_> {
    fn note(&mut self, note: Note, detail: &str) {
        let _warned = self.notes.note(self.service_id, note, detail);
    }

    /// Counts one message by what the mailbox did with its receipt.
    pub(super) fn count_ack(&mut self, outcome: AckOutcome, count: Count) {
        let result = match (outcome, count) {
            (AckOutcome::Stored, Count::Delivered) => {
                self.summary.delivered += 1;
                "delivered"
            }
            (AckOutcome::Stored, Count::Rejected) => {
                self.summary.rejected += 1;
                "rejected"
            }
            // Acked before, or closed by the mailbox.
            (AckOutcome::NotFound, _) => {
                debug!(service_id = self.service_id, "the mailbox no longer holds a message");
                "not_found"
            }
            (AckOutcome::Refused, _) => {
                warn!(service_id = self.service_id, "the mailbox refused a receipt");
                self.count_not_acked(1);
                return;
            }
        };
        metrics::counter!("substrate.mailbox_receiver.messages", "result" => result).increment(1);
    }

    /// Counts messages left for a later claim, in the summary and the
    /// metric alike.
    pub(super) fn count_not_acked(&mut self, messages: usize) {
        let messages = u32::try_from(messages).unwrap_or(u32::MAX);
        self.summary.not_acked = self.summary.not_acked.saturating_add(messages);
        metrics::counter!("substrate.mailbox_receiver.messages", "result" => "not_acked")
            .increment(u64::from(messages));
    }

    /// Ends this pass for the service, and counts it as blocked.
    fn stop(&mut self, note: Note) {
        self.summary.blocked += 1;
        self.note(note, "");
    }

    /// Stops pulling for this service until its certificate changes. The
    /// next certificate registers again.
    fn block(&mut self, instance: &ServiceInstance, note: Note) {
        self.entry.blocked_on = Some((instance.certificate.clone(), note));
        self.entry.registered = None;
        self.stop(note);
    }

    /// The limits a message must meet to be opened. The longest
    /// time-to-live of the queue is what the mailbox said at the last
    /// register, else the stored fact.
    fn open_policy(&self) -> OpenPolicy {
        let learned = self.entry.registered.as_ref().and_then(|r| r.max_ttl_secs);
        OpenPolicy {
            max_ttl_secs: learned.unwrap_or(self.fact.max_ttl_secs),
            max_clock_skew_secs: MAX_CLOCK_SKEW_SECS,
        }
    }

    /// How long a round may run on this node's clock.
    fn round_budget(&self) -> Duration {
        let visibility =
            self.entry.registered.as_ref().map_or(DEFAULT_VISIBILITY_SECS, |r| r.visibility_secs);
        Duration::from_secs(visibility.saturating_sub(ACK_MARGIN_SECS).max(1))
    }
}

/// The mailbox error code of `error`, when the mailbox answered one.
fn code_of(error: &ProxyError) -> Option<i32> {
    match error {
        ProxyError::Callee { code, .. } => Some(*code),
        _ => None,
    }
}

/// The passport refusal code (`data.code`) of a refused `register`.
fn refusal_code(error: &ProxyError) -> String {
    match error {
        ProxyError::Callee { data: Some(data), .. } => {
            data.get("code").and_then(Value::as_str).unwrap_or_default().to_string()
        }
        _ => String::new(),
    }
}

impl MailboxPuller {
    pub(super) async fn pull_service(&self, pass: &mut ServicePass<'_>) {
        let Some(inputs) = self.inputs(pass).await else {
            pass.summary.blocked += 1;
            return;
        };
        self.check_served_record(pass, &inputs.mailbox).await;
        // A different mailbox shows up in `needs_register`; the control
        // plane already removed the old queue.
        let may_claim = !pass.entry.needs_register(&inputs) || self.register(pass, &inputs).await;
        if may_claim {
            // After a restart, a service has no routes in this process until
            // it is deployed again. A message claimed before that would fail
            // and use up one of its claims.
            if self.dispatch.is_ready(pass.service_id) {
                self.claim_rounds(pass, &inputs).await;
            } else {
                pass.stop(Note::NotStarted);
            }
        }
        if pass.entry.holds_queue() {
            pass.summary.registered += 1;
        }
    }

    /// The inputs of a pass, or `None` after a note.
    async fn inputs(&self, pass: &mut ServicePass<'_>) -> Option<Inputs> {
        let Some(instance) = self.proxy.service_instance(pass.service_id) else {
            pass.note(Note::NoCertificate, "");
            return None;
        };
        match pass.entry.blocked_on.take() {
            Some((blocked, why)) if blocked == instance.certificate => {
                pass.entry.blocked_on = Some((blocked, why));
                pass.note(why, "");
                return None;
            }
            _ => {}
        }
        let key = match self.vault.mailbox_key(pass.service_id).await {
            Ok(Some(key)) => key,
            Ok(None) => {
                pass.note(Note::NoKey, "");
                return None;
            }
            Err(e) => {
                pass.note(Note::KeyUnreadable, &e.to_string());
                return None;
            }
        };
        let child_token = match self.vault.mailbox_passport(pass.service_id).await {
            Ok(token) => token,
            Err(e) => {
                pass.note(Note::KeyUnreadable, &e.to_string());
                return None;
            }
        };
        let (record, mailbox) = self.stored_record(pass).await?;
        if mailbox != pass.fact.mailbox {
            let detail = format!("record {mailbox}, fact {}", pass.fact.mailbox);
            pass.note(Note::RecordAndFactDiffer, &detail);
        }
        Some(Inputs { instance, key, record, mailbox, child_token })
    }

    /// The stored deploy record and the mailbox it names.
    async fn stored_record(
        &self,
        pass: &mut ServicePass<'_>,
    ) -> Option<(SignedEndpointInfo, String)> {
        let record = match self.vault.deploy_record(pass.service_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                pass.note(Note::NoRecord, "");
                return None;
            }
            Err(e) => {
                pass.note(Note::NoRecord, &e.to_string());
                return None;
            }
        };
        let Some(mailbox) = record.info.mailbox.as_ref().map(|m| m.service_id.clone()) else {
            pass.note(Note::NoRecord, "the stored record names no mailbox");
            return None;
        };
        Some((record, mailbox))
    }

    /// `true` when claiming may go on. Every register of a pass goes
    /// through here, also the one after a `-32040`.
    ///
    /// When the mailbox refuses the child token, the same pass registers
    /// again without it, so the queue lives with the open limits. The token
    /// is tried again at the next register, or when it changes.
    async fn register(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) -> bool {
        let Some(mut answer) = self.send_register(pass, inputs, inputs.child_token.clone()).await
        else {
            return false;
        };
        if inputs.child_token.is_some()
            && let Err(e) = &answer
            && code_of(e) == Some(codes::PASSPORT_REFUSED)
        {
            pass.note(Note::PassportRefused, &refusal_code(e));
            let Some(plain) = self.send_register(pass, inputs, None).await else { return false };
            answer = plain;
        }
        match answer {
            Ok(value) => match serde_json::from_value::<RegisterAnswer>(value) {
                Ok(answer) => {
                    self.learn(pass, inputs, answer.limits).await;
                    true
                }
                Err(e) => {
                    debug!(error = %e, "the register answer does not parse");
                    false
                }
            },
            Err(e) if code_of(&e) == Some(codes::BAD_RECORD) => {
                // A queue made with an earlier record may still hold
                // messages, so claiming goes on.
                pass.note(Note::RecordExpired, "");
                self.keep_registration(pass, inputs, None).await;
                true
            }
            Err(e) => {
                Self::register_failed(pass, inputs, &e);
                false
            }
        }
    }

    /// One `register` call. Under the gate, it registers only while the fact
    /// is the one this pass started with. The control plane changes the fact
    /// before it removes the queue, so a removed queue is never registered
    /// again. `None` when the fact changed.
    async fn send_register(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        child_token: Option<String>,
    ) -> Option<Result<Value, ProxyError>> {
        let params = RegisterParams { record: inputs.record.clone(), child_token };
        let params = serde_json::to_value(params).ok()?;
        let gate = self.register_gate(pass.service_id);
        let mut registered_at = gate.lock().await;
        let current = self.registry.mailbox_of(pass.service_id).map(|f| f.mailbox);
        if current.as_deref() != Some(pass.fact.mailbox.as_str()) {
            debug!(
                service_id = pass.service_id,
                "the mailbox changed during the pass; not registering"
            );
            pass.entry.registered = None;
            return None;
        }
        let answer = self
            .proxy
            .call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_REGISTER, params)
            .await;
        if answer.is_ok() {
            *registered_at = Some(inputs.mailbox.clone());
        }
        Some(answer)
    }

    fn register_failed(pass: &mut ServicePass<'_>, inputs: &Inputs, error: &ProxyError) {
        match code_of(error) {
            Some(codes::NOT_AN_INSTANCE) => pass.block(&inputs.instance, Note::InstanceRefused),
            Some(codes::STALE_INSTANCE) => pass.block(&inputs.instance, Note::StaleInstance),
            Some(codes::MAILBOX_FULL) => pass.stop(Note::MailboxFull),
            _ => debug!(error = %error, "could not register at the mailbox"),
        }
    }

    /// Keeps the registration and writes the queue's real time-to-live back
    /// into the fact. A mailbox may answer any number: above the limit it
    /// would turn off the age check of a message and make the receiver fence
    /// window endless, so it is cut. The limit is what a passport may give
    /// when the owner stored a child token for this service, else the open
    /// limit: a mailbox the owner chose without a passport cannot widen it.
    async fn learn(&self, pass: &mut ServicePass<'_>, inputs: &Inputs, limits: QueueLimits) {
        let cap =
            if inputs.child_token.is_some() { DEDICATED_MAX_TTL_SECS } else { OPEN_MAX_TTL_SECS };
        let max_ttl_secs = limits.max_ttl_secs.min(cap);
        self.keep_registration(pass, inputs, Some(max_ttl_secs)).await;
        self.write_back_ttl(pass.service_id, &inputs.mailbox, max_ttl_secs).await;
    }

    /// Remembers the register, with the visibility time read by one `info`
    /// call.
    async fn keep_registration(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        max_ttl_secs: Option<u64>,
    ) {
        let visibility_secs = self.read_visibility(inputs).await;
        pass.entry.registered = Some(Registration {
            mailbox: inputs.mailbox.clone(),
            record_hex: inputs.record.pkarr_packet_hex.clone(),
            at: Instant::now(),
            max_ttl_secs,
            visibility_secs,
            child_token_digest: inputs.child_token_digest(),
        });
    }

    async fn read_visibility(&self, inputs: &Inputs) -> u64 {
        let call = self.proxy.call_mailbox(
            &inputs.instance,
            &inputs.mailbox,
            METHOD_INFO,
            serde_json::json!({}),
        );
        match time::timeout(INFO_TIMEOUT, call).await {
            Ok(Ok(value)) => serde_json::from_value::<InfoAnswer>(value)
                .map_or(DEFAULT_VISIBILITY_SECS, |info| info.visibility_secs),
            _ => DEFAULT_VISIBILITY_SECS,
        }
    }

    /// Only while the fact still names the mailbox registered at, and
    /// atomically with the control plane's own writes of the fact: a deploy
    /// or undeploy meanwhile is never overwritten.
    async fn write_back_ttl(&self, service_id: &str, mailbox: &str, max_ttl_secs: u64) {
        if let Err(e) = self.registry.set_mailbox_ttl(service_id, mailbox, max_ttl_secs).await {
            warn!(service_id, error = %e, "could not record the mailbox queue lifetime");
        }
    }

    async fn claim_rounds(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) {
        let mut registered_again = false;
        for _ in 0..MAX_CLAIM_ROUNDS {
            // Built for each round: a register after a `-32040` may have
            // learned another lifetime.
            let policy = pass.open_policy();
            let params = serde_json::json!(ClaimParams { max: Some(MAX_CLAIM_BATCH) });
            // Taken before the call, so the local window is never longer
            // than the mailbox's. The mailbox's own times are never compared
            // with this node's clock.
            let claimed_at = Instant::now();
            let claim = self
                .proxy
                .call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_CLAIM, params)
                .await;
            let answer = match claim {
                Ok(value) => match serde_json::from_value::<ClaimAnswer>(value) {
                    Ok(answer) => answer,
                    Err(_) => return,
                },
                Err(e) if code_of(&e) == Some(codes::UNKNOWN_DESTINATION) && !registered_again => {
                    registered_again = true;
                    // A refused record cannot make the queue again.
                    if pass.entry.record_refused() {
                        return;
                    }
                    pass.entry.registered = None;
                    if self.register(pass, inputs).await {
                        continue;
                    }
                    return;
                }
                Err(e) => {
                    Self::claim_failed(pass, inputs, &e);
                    return;
                }
            };
            let full = answer.messages.len() >= MAX_CLAIM_BATCH as usize;
            let finished = self.handle_round(pass, inputs, &answer, claimed_at, policy).await;
            if !finished || !full {
                return;
            }
        }
    }

    fn claim_failed(pass: &mut ServicePass<'_>, inputs: &Inputs, error: &ProxyError) {
        match code_of(error) {
            Some(codes::NOT_AN_INSTANCE) => pass.block(&inputs.instance, Note::InstanceRefused),
            Some(codes::STALE_INSTANCE) => pass.block(&inputs.instance, Note::StaleInstance),
            _ => debug!(error = %error, "could not claim from the mailbox"),
        }
    }

    /// Handles one claimed batch inside its visibility time, on this node's
    /// clock only. Before each message except the first, it stops when
    /// another delivery could run past the budget. It sends the acks it has
    /// after every `ACK_FLUSH` handled messages and at the end. `true` when
    /// every message was handled and every ack call went through.
    async fn handle_round(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        answer: &ClaimAnswer,
        claimed_at: Instant,
        policy: OpenPolicy,
    ) -> bool {
        let budget = pass.round_budget();
        let mut acks = Vec::new();
        for (i, claimed) in answer.messages.iter().enumerate() {
            if i > 0 && claimed_at.elapsed() + DEFAULT_PROXY_CALL_TIMEOUT >= budget {
                // The rest come back after the visibility time.
                pass.count_not_acked(answer.messages.len() - i);
                self.send_acks(pass, inputs, acks).await;
                return false;
            }
            if let Some(entry) = self.handle_one(pass, inputs, claimed, policy).await {
                acks.push(entry);
            }
            if acks.len() >= ACK_FLUSH && !self.send_acks(pass, inputs, mem::take(&mut acks)).await
            {
                return false;
            }
        }
        self.send_acks(pass, inputs, acks).await
    }

    /// One `ack` call. `false` when the call failed: the messages count as
    /// not acked, come back after the visibility time, and the fence
    /// replays them. An empty list makes no call. A message is counted as
    /// delivered or rejected only when the mailbox stored its receipt.
    async fn send_acks(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        handled: Vec<(AckEntry, Count)>,
    ) -> bool {
        if handled.is_empty() {
            return true;
        }
        let (acks, counts): (Vec<_>, Vec<_>) = handled.into_iter().unzip();
        let Some(results) = self.ack(inputs, acks).await else {
            pass.count_not_acked(counts.len());
            return false;
        };
        for (outcome, count) in results.into_iter().zip(counts) {
            pass.count_ack(outcome, count);
        }
        true
    }

    /// The mailbox's answer to one `ack` call, or `None` when there is none.
    async fn ack(&self, inputs: &Inputs, acks: Vec<AckEntry>) -> Option<Vec<AckOutcome>> {
        let params = serde_json::to_value(AckParams { acks }).ok()?;
        let answer =
            self.proxy.call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_ACK, params).await;
        match answer.map(serde_json::from_value::<AckAnswer>) {
            Ok(Ok(answer)) => Some(answer.results),
            Ok(Err(e)) => {
                debug!(error = %e, "the ack answer does not parse");
                None
            }
            Err(e) => {
                debug!(error = %e, "could not ack at the mailbox");
                None
            }
        }
    }

    /// At most once an hour, looks up the record the registry serves for
    /// the service and notes when it names another mailbox than the stored
    /// record. Pulling goes on from the stored record.
    async fn check_served_record(&self, pass: &mut ServicePass<'_>, mailbox: &str) {
        let due = pass
            .entry
            .served_checked_at
            .is_none_or(|at| at.elapsed() >= SERVED_RECORD_CHECK_INTERVAL);
        if !due {
            return;
        }
        pass.entry.served_checked_at = Some(Instant::now());
        let lookup = self.proxy.registry_client().lookup(pass.service_id, false);
        let Ok(Ok(served)) = time::timeout(SERVED_RECORD_LOOKUP_TIMEOUT, lookup).await else {
            return;
        };
        let served_mailbox = served.info.mailbox.map(|m| m.service_id);
        if served_mailbox.as_deref() != Some(mailbox) {
            let detail =
                format!("served {}, stored {mailbox}", served_mailbox.as_deref().unwrap_or("none"));
            pass.note(Note::ServedRecordDiffers, &detail);
        }
    }
}

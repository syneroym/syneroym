//! One hosted service: its inputs, its registration at the mailbox, and its
//! claim rounds.

use std::mem;

use syneroym_core::{
    dht_registry::SignedEndpointInfo,
    mailbox::{
        AckAnswer, AckEntry, AckOutcome, AckParams, ClaimAnswer, ClaimParams, HostedMailbox,
        InfoAnswer, MAX_CLAIM_BATCH, METHOD_ACK, METHOD_CLAIM, METHOD_INFO, METHOD_REGISTER,
        PullSummary, QueueLimits, RegisterAnswer, RegisterParams, codes,
    },
};
use syneroym_identity::DelegationCertificate;
use syneroym_mailbox_format::{MAX_CLOCK_SKEW_SECS, MailboxSecretKey, OpenPolicy};
use syneroym_rpc::{DEFAULT_PROXY_CALL_TIMEOUT, ProxyError};
use tokio::time::{self, Duration, Instant};
use tracing::{debug, warn};

use super::{
    MAX_CLAIM_ROUNDS, MailboxPuller, REREGISTER_INTERVAL, SERVED_RECORD_CHECK_INTERVAL,
    SERVED_RECORD_LOOKUP_TIMEOUT,
    notes::{Note, Notes},
};
use crate::proxy::{AnchorCache, ServiceInstance};

/// How long a claimed message stays hidden at a mailbox, when the mailbox
/// did not say.
const DEFAULT_VISIBILITY_SECS: u64 = 300;
/// A round stops this long before the visibility time ends, to leave room
/// for the acks.
const ACK_MARGIN_SECS: u64 = 30;
/// Acks are sent after this many handled messages.
pub(super) const ACK_FLUSH: usize = 8;

/// The identity of an installed certificate, to tell a renewed one apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CertId {
    temporary_did: String,
    issued_at_secs: u64,
}

impl CertId {
    fn of(certificate: &DelegationCertificate) -> Self {
        Self {
            temporary_did: certificate.temporary_did.clone(),
            issued_at_secs: certificate.issued_at_secs,
        }
    }
}

/// What the puller remembers about one service between passes.
#[derive(Default)]
pub(super) struct ServiceState {
    registered: Option<Registration>,
    /// The certificate the mailbox refused, and why. The service is skipped
    /// until the installed certificate differs.
    blocked_on: Option<(CertId, Note)>,
    served_checked_at: Option<Instant>,
}

struct Registration {
    mailbox: String,
    /// `pkarr_packet_hex` of the record registered.
    record_hex: String,
    at: Instant,
    max_ttl_secs: u64,
    /// From the mailbox's `info`, read at register; a default when it
    /// failed.
    visibility_secs: u64,
}

impl ServiceState {
    fn needs_register(&self, inputs: &Inputs) -> bool {
        match &self.registered {
            None => true,
            Some(r) => {
                r.mailbox != inputs.mailbox
                    || r.record_hex != inputs.record.pkarr_packet_hex
                    || r.at.elapsed() >= REREGISTER_INTERVAL
            }
        }
    }
}

/// Everything one service needs for one pass.
pub(super) struct Inputs {
    pub(super) instance: ServiceInstance,
    pub(super) key: MailboxSecretKey,
    record: SignedEndpointInfo,
    pub(super) mailbox: String,
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

    /// Stops pulling for this service until its certificate changes.
    fn block(&mut self, instance: &ServiceInstance, note: Note) {
        self.entry.blocked_on = Some((CertId::of(&instance.certificate), note));
        self.note(note, "");
    }

    /// The longest time-to-live of the queue: what the mailbox said at the
    /// last register, else the stored fact.
    fn max_ttl_secs(&self) -> u64 {
        self.entry.registered.as_ref().map_or(self.fact.max_ttl_secs, |r| r.max_ttl_secs)
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

impl MailboxPuller {
    pub(super) async fn pull_service(&self, pass: &mut ServicePass<'_>) {
        let Some(inputs) = self.inputs(pass).await else {
            pass.summary.blocked += 1;
            return;
        };
        self.check_served_record(pass, &inputs.mailbox).await;
        // A different mailbox shows up in `needs_register`; the control
        // plane already removed the old queue.
        if pass.entry.needs_register(&inputs) && !self.register(pass, &inputs).await {
            return;
        }
        self.claim_rounds(pass, &inputs).await;
    }

    /// The inputs of a pass, or `None` after a note.
    async fn inputs(&self, pass: &mut ServicePass<'_>) -> Option<Inputs> {
        let Some(instance) = self.proxy.service_instance(pass.service_id) else {
            pass.note(Note::NoCertificate, "");
            return None;
        };
        let cert_id = CertId::of(&instance.certificate);
        match pass.entry.blocked_on.take() {
            Some((blocked, why)) if blocked == cert_id => {
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
        let (record, mailbox) = self.stored_record(pass).await?;
        if mailbox != pass.fact.mailbox {
            let detail = format!("record {mailbox}, fact {}", pass.fact.mailbox);
            pass.note(Note::RecordAndFactDiffer, &detail);
        }
        Some(Inputs { instance, key, record, mailbox })
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
    async fn register(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) -> bool {
        let Ok(params) = serde_json::to_value(RegisterParams { record: inputs.record.clone() })
        else {
            return false;
        };
        // Under the gate, register only while the fact is the one this pass
        // started with. The control plane changes the fact before it removes
        // the queue, so a removed queue is never registered again.
        let answer = {
            let gate = self.register_gate(pass.service_id);
            let _held = gate.lock().await;
            let current = self.registry.mailbox_of(pass.service_id).map(|f| f.mailbox);
            if current.as_deref() != Some(pass.fact.mailbox.as_str()) {
                debug!(
                    service_id = pass.service_id,
                    "the mailbox changed during the pass; not registering"
                );
                pass.entry.registered = None;
                return false;
            }
            self.proxy
                .call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_REGISTER, params)
                .await
        };
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
            Err(e) => Self::register_failed(pass, inputs, &e),
        }
    }

    /// `true` when claiming may go on despite the error.
    fn register_failed(pass: &mut ServicePass<'_>, inputs: &Inputs, error: &ProxyError) -> bool {
        match code_of(error) {
            Some(codes::BAD_RECORD) => {
                pass.note(Note::RecordExpired, "");
                true
            }
            Some(codes::NOT_AN_INSTANCE) => {
                pass.block(&inputs.instance, Note::InstanceRefused);
                false
            }
            Some(codes::STALE_INSTANCE) => {
                pass.block(&inputs.instance, Note::StaleInstance);
                false
            }
            Some(codes::MAILBOX_FULL) => {
                pass.note(Note::MailboxFull, "");
                false
            }
            _ => {
                debug!(error = %error, "could not register at the mailbox");
                false
            }
        }
    }

    /// Keeps the registration, reads the visibility time with one `info`
    /// call, and writes the queue's real time-to-live back into the fact.
    async fn learn(&self, pass: &mut ServicePass<'_>, inputs: &Inputs, limits: QueueLimits) {
        let visibility_secs = self.read_visibility(inputs).await;
        pass.entry.registered = Some(Registration {
            mailbox: inputs.mailbox.clone(),
            record_hex: inputs.record.pkarr_packet_hex.clone(),
            at: Instant::now(),
            max_ttl_secs: limits.max_ttl_secs,
            visibility_secs,
        });
        self.write_back_ttl(pass.service_id, &inputs.mailbox, limits.max_ttl_secs).await;
    }

    async fn read_visibility(&self, inputs: &Inputs) -> u64 {
        let call = self.proxy.call_mailbox(
            &inputs.instance,
            &inputs.mailbox,
            METHOD_INFO,
            serde_json::json!({}),
        );
        match time::timeout(SERVED_RECORD_LOOKUP_TIMEOUT, call).await {
            Ok(Ok(value)) => serde_json::from_value::<InfoAnswer>(value)
                .map_or(DEFAULT_VISIBILITY_SECS, |info| info.visibility_secs),
            _ => DEFAULT_VISIBILITY_SECS,
        }
    }

    /// Only while the fact still names the mailbox registered at: a deploy
    /// that changed the mailbox meanwhile must not be overwritten.
    async fn write_back_ttl(&self, service_id: &str, mailbox: &str, max_ttl_secs: u64) {
        let Some(current) = self.registry.mailbox_of(service_id) else { return };
        if current.mailbox != mailbox || current.max_ttl_secs == max_ttl_secs {
            return;
        }
        let updated = HostedMailbox { max_ttl_secs, ..current };
        if let Err(e) = self.registry.set_mailbox(service_id.to_string(), updated).await {
            warn!(service_id, error = %e, "could not record the mailbox queue lifetime");
        }
    }

    async fn claim_rounds(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) {
        let policy = OpenPolicy {
            max_ttl_secs: pass.max_ttl_secs(),
            max_clock_skew_secs: MAX_CLOCK_SKEW_SECS,
        };
        let mut registered_again = false;
        for _ in 0..MAX_CLAIM_ROUNDS {
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
                    pass.entry.registered = None;
                    registered_again = true;
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
                pass.summary.not_acked +=
                    u32::try_from(answer.messages.len() - i).unwrap_or(u32::MAX);
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

    /// One `ack` call. `false` when the call failed: the messages come back
    /// after the visibility time and the fence replays them. An empty list
    /// makes no call.
    async fn send_acks(
        &self,
        pass: &ServicePass<'_>,
        inputs: &Inputs,
        acks: Vec<AckEntry>,
    ) -> bool {
        if acks.is_empty() {
            return true;
        }
        let Ok(params) = serde_json::to_value(AckParams { acks }) else { return false };
        let answer =
            self.proxy.call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_ACK, params).await;
        let results = match answer.map(serde_json::from_value::<AckAnswer>) {
            Ok(Ok(answer)) => answer.results,
            Ok(Err(e)) => {
                debug!(error = %e, "the ack answer does not parse");
                return false;
            }
            Err(e) => {
                debug!(error = %e, "could not ack at the mailbox");
                return false;
            }
        };
        for outcome in results {
            match outcome {
                AckOutcome::Stored => {}
                AckOutcome::NotFound => {
                    debug!(service_id = pass.service_id, "the mailbox no longer holds a message");
                }
                AckOutcome::Refused => {
                    warn!(service_id = pass.service_id, "the mailbox refused a receipt");
                }
            }
        }
        true
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

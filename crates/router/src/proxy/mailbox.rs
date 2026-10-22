//! The sender side of a receiver's mailbox (ADR-0026).
//!
//! A call that may wait makes one direct attempt. If the receiver cannot be
//! reached, the call is sealed and handed to the mailbox its signed record
//! names. The outbox then stops sending direct attempts and only asks the
//! mailbox about the deposit (`poll`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use syneroym_core::{
    dht_registry::EndpointMailbox,
    mailbox::{self as wire, MAILBOX_INTERFACE, codes},
};
use syneroym_identity::DelegationCertificate;
use syneroym_mailbox_format::{
    MessageContent, OPEN_MAX_TTL_SECS, Recipient, SealError, Sender, SentDeposit, seal,
};
use syneroym_rpc::{AuthLevel, Delivery, DepositTicket, STREAM_UNAUTHENTICATED_RPC_CODE};

use super::*;
use crate::{handshake::now_secs, net_iroh::ResolvedPeer};

mod poll;

/// One direct attempt at a receiver that has a mailbox. Short: if it fails,
/// the message waits at the mailbox instead. It covers connect, the
/// end-to-end handshake on an indirect connection, the request and the
/// answer together.
const MAILBOX_DIRECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// Every call to a mailbox (`deposit`, `status`). Well under the outbox
/// visibility timeout, so a slow mailbox never lets a second claim start.
const MAILBOX_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a sender asks about a deposit. The receiver pulls about this
/// often, so asking more often learns nothing new.
const DEPOSIT_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Time after a deposit's time-to-live before a deposit that still reads
/// `stored` is treated as lost. The mailbox sweeps expired messages on its
/// own schedule.
const DEPOSIT_DEADLINE_GRACE_MS: i64 = 3_600_000;

/// What the sender keeps in the parked row: the mailbox and what is needed
/// to check the receipt. Never the sealed bytes.
#[derive(Debug, Serialize, Deserialize)]
struct DepositContext {
    mailbox: String,
    sent: SentDeposit,
}

/// The sending instance: what it seals with and what it shows the mailbox.
pub(in crate::proxy) struct DepositSender {
    pub(in crate::proxy) service_id: String,
    pub(in crate::proxy) key: Arc<Identity>,
    pub(in crate::proxy) certificate: DelegationCertificate,
}

#[derive(Debug, thiserror::Error)]
enum DepositFailure {
    #[error("cannot seal: {0}")]
    Seal(SealError),
    #[error("cannot encode: {0}")]
    Encode(String),
    #[error("mailbox call failed: {0}")]
    Call(ProxyError),
    #[error("mailbox refused with code {0}")]
    Refused(i32),
}

/// Only an error that says "not delivered, and nothing ran" is worth a
/// deposit: the receiver could not be reached, or it refused the stream
/// before any dispatch. Anything else is an answer, and the outbox handles
/// it as before. The match names every variant, so a new `ProxyError`
/// variant is a compile error here instead of silently "not worth a
/// deposit".
fn worth_a_deposit(error: &ProxyError) -> bool {
    match error {
        ProxyError::Transport(_)
        | ProxyError::Timeout(_)
        | ProxyError::ServiceNotFound(_)
        | ProxyError::HostMismatch(_) => true,
        ProxyError::Callee { code, .. } => matches!(
            *code,
            SERVICE_NOT_FOUND_RPC_CODE | NOT_HOSTED_HERE_RPC_CODE | STREAM_UNAUTHENTICATED_RPC_CODE
        ),
        ProxyError::UnsupportedProtocol(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::PermissionDenied(_)
        | ProxyError::Internal(_) => false,
    }
}

/// `data.max_ttl_secs` of a `ttl_too_long` answer, when it is above zero
/// and shorter than what was asked.
fn shorter_ttl(data: Option<&Value>, asked: u64) -> Option<u64> {
    let limit = data?.get("max_ttl_secs")?.as_u64()?;
    (limit > 0 && limit < asked).then_some(limit)
}

impl ProxyRouter {
    /// The sending service of `req`, when `req` may use a mailbox at all.
    fn deposit_sender(&self, req: &ProxyRequest) -> Option<DepositSender> {
        req.idempotency_key.as_ref()?;
        // A call made for a person passes that person's proof on. A sealed
        // message cannot carry it, so the receiver would lose the person.
        if req.caller.proof.is_some() {
            return None;
        }
        let service_id = match &req.origin {
            CallOrigin::Guest { service_id } => service_id,
            CallOrigin::Native { service_id: Some(id) } => id,
            CallOrigin::Native { service_id: None } => return None,
        };
        self.sender_identity(service_id)
    }

    /// The same key and certificate a remote call of `service_id` presents,
    /// so the seal and the transport always agree.
    pub(in crate::proxy) fn sender_identity(&self, service_id: &str) -> Option<DepositSender> {
        let identity = self.instance_identity(service_id)?;
        let certificate = identity.delegation?;
        Some(DepositSender { service_id: service_id.to_string(), key: identity.key, certificate })
    }

    /// The caller every mailbox call carries. A same-node mailbox reads the
    /// proof from this context, because there is no stream to sign. On a
    /// remote call the proof is unsigned, so it is never passed on: the hop
    /// presents the instance identity of the origin and signs it.
    fn instance_caller(sender: &DepositSender) -> CallerContext {
        let mut caller = CallerContext::verified(
            &sender.certificate.master_did,
            AuthLevel::Delegated,
            Vec::new(),
        );
        caller.proof = Some(CallerProof {
            pubkey_hex: hex::encode(sender.key.public_key().to_bytes()),
            delegation_json: sender.certificate.to_json().ok(),
            ..Default::default()
        });
        caller
    }

    pub(in crate::proxy) fn mailbox_request(
        sender: &DepositSender,
        mailbox: &str,
        method: &str,
        params: Value,
    ) -> ProxyRequest {
        ProxyRequest {
            target_service: mailbox.to_string(),
            interface: MAILBOX_INTERFACE.to_string(),
            method: method.to_string(),
            params,
            caller: Self::instance_caller(sender),
            origin: CallOrigin::Native { service_id: Some(sender.service_id.clone()) },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotent: false,
            idempotency_key: None,
            timeout: Some(MAILBOX_CALL_TIMEOUT),
        }
    }

    /// Delivers `req` directly, or seals it and deposits it at the
    /// receiver's mailbox when the receiver cannot be reached.
    pub(super) async fn deliver_or_deposit_inner(
        &self,
        req: &ProxyRequest,
        max_wait_secs: u64,
    ) -> Result<Delivery, ProxyError> {
        // The same gates as `invoke_inner`, in the same order.
        if req.protocol != ProxyProtocol::JsonRpcV1 {
            return Err(ProxyError::UnsupportedProtocol(format!("{:?}", req.protocol)));
        }
        self.check_native_capability_gate(req)?;
        metrics::counter!("substrate.proxy.calls").increment(1);

        // A receiver on this node is never asleep.
        if let Some((endpoint, interface)) =
            self.registry.lookup(&req.target_service, &req.interface)
        {
            return self.invoke_local_guarded(req, endpoint, interface).await.map(Delivery::Direct);
        }
        let peer = net_iroh::resolve_peer(
            &self.registry_client,
            self.peer_records.as_deref(),
            &req.target_service,
        )
        .await
        .map_err(|_| ProxyError::ServiceNotFound(req.target_service.clone()))?;
        self.deliver_to_peer(req, peer, max_wait_secs).await
    }

    /// Split out so tests can pass a synthetic `ResolvedPeer`.
    pub(super) async fn deliver_to_peer(
        &self,
        req: &ProxyRequest,
        peer: ResolvedPeer,
        max_wait_secs: u64,
    ) -> Result<Delivery, ProxyError> {
        let remote = peer.addr.map(|addr| RemotePeer { addr, host_key: peer.host_key });

        // No mailbox, or a call that may not use one: today's path, with
        // its retry loop and its re-homing.
        let (Some(mailbox), Some(sender)) = (peer.mailbox.as_ref(), self.deposit_sender(req))
        else {
            let remote =
                remote.ok_or_else(|| ProxyError::ServiceNotFound(req.target_service.clone()))?;
            return self.invoke_remote_resolved(remote, req).await.map(Delivery::Direct);
        };

        let direct_error = match &remote {
            Some(remote) => {
                match self.invoke_remote_once(remote, req, MAILBOX_DIRECT_ATTEMPT_TIMEOUT).await {
                    Ok(value) => return Ok(Delivery::Direct(value)),
                    Err(e) if !worth_a_deposit(&e) => return Err(e),
                    Err(e) => e,
                }
            }
            None => ProxyError::ServiceNotFound(req.target_service.clone()),
        };

        let ttl_secs = OPEN_MAX_TTL_SECS.min(max_wait_secs).max(1);
        match self.deposit(req, &sender, mailbox, ttl_secs).await {
            Ok(ticket) => {
                metrics::counter!("substrate.proxy.mailbox.deposited").increment(1);
                Ok(Delivery::Deposited(ticket))
            }
            Err(why) => {
                metrics::counter!("substrate.proxy.mailbox.deposit_failed").increment(1);
                warn!(
                    target = %req.target_service,
                    error = %why,
                    "could not deposit at the receiver's mailbox; will try directly again"
                );
                Err(direct_error)
            }
        }
    }

    /// Seals `req` for its receiver and hands it to the receiver's mailbox.
    /// Each call seals again, with a new deposit id and send time.
    async fn deposit(
        &self,
        req: &ProxyRequest,
        sender: &DepositSender,
        mailbox: &EndpointMailbox,
        mut ttl_secs: u64,
    ) -> Result<DepositTicket, DepositFailure> {
        let content = MessageContent {
            interface: req.interface.clone(),
            method: req.method.clone(),
            params: req.params.clone(),
            idempotency_key: req.idempotency_key.clone().unwrap_or_default(),
        };
        let recipient = Recipient { service_id: &req.target_service, mailbox_key: &mailbox.key };
        let seal_as = Sender { instance: &sender.key, certificate: &sender.certificate };
        // At most two tries: the second only for a mailbox that allows a
        // shorter time-to-live than asked.
        for _ in 0..2 {
            let sealed = seal(&content, recipient, seal_as, ttl_secs, now_secs())
                .map_err(DepositFailure::Seal)?;
            let params = serde_json::to_value(&sealed.deposit)
                .map_err(|e| DepositFailure::Encode(e.to_string()))?;
            let call =
                Self::mailbox_request(sender, &mailbox.service_id, wire::METHOD_DEPOSIT, params);
            match self.invoke_inner(&call).await {
                // Any answer means the mailbox took it; `status` tells the rest.
                Ok(_) => return Self::ticket(&mailbox.service_id, sealed.sent, ttl_secs),
                Err(ProxyError::Callee { code, data, .. }) if code == codes::TTL_TOO_LONG => {
                    ttl_secs = shorter_ttl(data.as_ref(), ttl_secs)
                        .ok_or(DepositFailure::Refused(code))?;
                }
                Err(e) => return Err(DepositFailure::Call(e)),
            }
        }
        Err(DepositFailure::Refused(codes::TTL_TOO_LONG))
    }

    fn ticket(
        mailbox: &str,
        sent: SentDeposit,
        ttl_secs: u64,
    ) -> Result<DepositTicket, DepositFailure> {
        let ends_at_secs =
            i64::try_from(sent.sent_at_secs.saturating_add(ttl_secs)).unwrap_or(i64::MAX);
        let deadline_at_ms =
            ends_at_secs.saturating_mul(1000).saturating_add(DEPOSIT_DEADLINE_GRACE_MS);
        let context = serde_json::to_vec(&DepositContext { mailbox: mailbox.to_string(), sent })
            .map_err(|e| DepositFailure::Encode(e.to_string()))?;
        Ok(DepositTicket { context, poll_interval: DEPOSIT_POLL_INTERVAL, deadline_at_ms })
    }
}

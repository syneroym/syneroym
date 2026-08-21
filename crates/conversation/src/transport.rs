//! The peer-facing verbs (`prekey-bundle`, `deliver`) and the outbound
//! call they travel over (`D-B4-6`, `D-B4-23`).

use std::time::Duration;

use syneroym_rpc::{
    CallOrigin, CallerContext, ConversationError, ProxyError, ProxyProtocol, ProxyRequest,
};

use crate::{
    ConversationService,
    crypto::{Envelope, PrekeyBundle},
    envelope::DeliveryPayload,
    ids::derive_conversation_id,
    store::{StoredMessage, now_ms},
};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DeliveryAck {
    pub message_id: String,
}

/// What one delivery attempt's failure actually means for the outbox item
/// -- mirrors `syneroym_router::proxy_outbox::Disposition` (F1), not
/// re-derived independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// The peer is not reachable right now -- re-defer without charging
    /// the attempt budget (`D-B4-20`).
    Unreachable,
    /// A settled refusal: no certificate, malformed envelope, permission
    /// denied. Never retried.
    Terminal,
    /// A real transport failure -- the attempt budget applies.
    Retry,
}

impl ConversationService {
    /// `D-B4-23`: builds the delivery worker's outbound `CallerContext`
    /// with `proof: None` and `CallOrigin::Native`, and refuses up front if
    /// this service holds no unexpired instance certificate or recorded
    /// owner -- both required for `invoke_remote_at`'s instance-certificate
    /// branch to present this *service's* identity to the peer rather than
    /// silently falling back to the node's own key.
    fn check_outbound_identity(&self, svc: &str) -> Result<(), Disposition> {
        let Some(cert) = self.registry.instance_cert(svc) else {
            return Err(Disposition::Terminal);
        };
        if cert.is_expired() {
            return Err(Disposition::Terminal);
        }
        if self.registry.owner_of(svc).is_none() {
            return Err(Disposition::Terminal);
        }
        Ok(())
    }

    pub(crate) async fn call_peer(
        &self,
        svc: &str,
        peer_address: &str,
        method: &str,
        params: serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<serde_json::Value, Disposition> {
        self.check_outbound_identity(svc)?;
        let Some(proxy) = self.current_service_proxy().upgrade() else {
            return Err(Disposition::Retry);
        };
        let caller = CallerContext::service_system(svc);
        let request = ProxyRequest {
            target_service: peer_address.to_string(),
            interface: "conversation".to_string(),
            method: method.to_string(),
            params,
            caller,
            origin: CallOrigin::Native { service_id: Some(svc.to_string()) },
            protocol: ProxyProtocol::default(),
            idempotent: idempotency_key.is_some(),
            idempotency_key,
            timeout: Some(Duration::from_secs(30)),
        };
        proxy.invoke(request).await.map_err(classify)
    }

    /// The sending side of one delivery attempt (`deliver()` in the plan).
    /// Never called on the hot dispatch path -- only from the outbox
    /// worker.
    pub(crate) async fn deliver_one(
        &self,
        svc: &str,
        peer_address: &str,
        msg: &StoredMessage,
    ) -> Result<(), Disposition> {
        let store = self.store_for(svc).await.map_err(|_| Disposition::Retry)?;

        let existing_session =
            self.crypto.session_for(&store, peer_address).await.map_err(|_| Disposition::Retry)?;
        let mut session = match existing_session {
            Some(session) => session,
            None => {
                let bundle_json = self
                    .call_peer(svc, peer_address, "prekey-bundle", serde_json::json!({}), None)
                    .await?;
                let bundle: PrekeyBundle =
                    serde_json::from_value(bundle_json).map_err(|_| Disposition::Terminal)?;
                self.crypto
                    .begin_session(&store, peer_address, &bundle)
                    .await
                    .map_err(|_| Disposition::Terminal)?
            }
        };

        let payload = DeliveryPayload {
            message_id: msg.id.clone(),
            conversation_id: msg.conversation_id.clone(),
            author: msg.author.clone(),
            sender_timestamp_ms: msg.sender_timestamp_ms,
            content_type: msg.content_type.clone(),
            body: msg.body.clone(),
            signature: msg.signature,
        };
        let env = self.crypto.encrypt(&mut session, &payload).map_err(|_| Disposition::Terminal)?;
        let env_json = serde_json::to_value(&env).map_err(|_| Disposition::Terminal)?;

        let ack_json =
            self.call_peer(svc, peer_address, "deliver", env_json, Some(msg.id.clone())).await?;
        let _ack: DeliveryAck =
            serde_json::from_value(ack_json).map_err(|_| Disposition::Terminal)?;

        // Ratchet-commit ordering (`D-B4-18`): only after a real `Ok` from
        // the peer, so a failed call leaves the sender able to retry under
        // the same key rather than a step ahead of a receiver that never
        // saw it.
        self.crypto.commit(&store, &session).await.map_err(|_| Disposition::Retry)?;
        Ok(())
    }

    /// The receiving side (`peer_deliver()` in the plan) -- reached only
    /// from `dispatch_conversation`, never from a guest.
    pub(crate) async fn peer_deliver_impl(
        &self,
        svc: &str,
        requester_did: &str,
        env: Envelope,
    ) -> Result<DeliveryAck, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store =
            self.store_for(svc).await.map_err(|e| ConversationError::Internal(e.to_string()))?;

        let mut session = self
            .crypto
            .session_for_envelope(&store, &env)
            .await
            .map_err(|_| ConversationError::PermissionDenied)?;
        let payload = self
            .crypto
            .decrypt(&mut session, &env)
            .map_err(|_| ConversationError::PermissionDenied)?;

        let author = payload.author.clone();
        if author == svc {
            // `D-B4-26`: the same-service exemption in
            // `check_native_capability_gate` lets a guest reach this arm on
            // its own service id, but it cannot sign as a peer.
            return Err(ConversationError::PermissionDenied);
        }
        if !crate::envelope::verify(
            &ed25519_dalek::VerifyingKey::from_bytes(&session.peer_sig_key)
                .map_err(|_| ConversationError::PermissionDenied)?,
            &payload,
        ) {
            return Err(ConversationError::PermissionDenied);
        }
        let now = now_ms();
        let max_skew_ms = (self.max_clock_skew_secs as i64).saturating_mul(1000);
        if payload.sender_timestamp_ms > now.saturating_add(max_skew_ms) {
            return Err(ConversationError::InvalidArgument(
                "sender timestamp implausibly far in the future".to_string(),
            ));
        }
        let expected_conv_id = derive_conversation_id(svc, &author);
        if payload.conversation_id != expected_conv_id {
            return Err(ConversationError::InvalidArgument(
                "conversation id does not match the verified author".to_string(),
            ));
        }

        store
            .queue()
            .transaction(|tx, _txq| {
                store.insert_incoming_if_absent(
                    tx,
                    &payload.conversation_id,
                    &payload.message_id,
                    &author,
                    payload.sender_timestamp_ms,
                    &payload.content_type,
                    &payload.body,
                    &payload.signature,
                    now,
                )?;
                self.crypto
                    .commit_in(tx, &session)
                    .map_err(|e| anyhow::anyhow!("session commit failed: {e}"))?;
                Ok(())
            })
            .map_err(|e| ConversationError::Internal(e.to_string()))?;

        if let Ok(Some(stored)) = store.get_message(&payload.message_id) {
            self.notify_message(svc, stored.into_wire()).await;
        }

        Ok(DeliveryAck { message_id: payload.message_id })
    }
}

/// Mirrors `syneroym_router::proxy_outbox::disposition_of` (F1): not
/// re-derived independently, adapted to this crate's own `Disposition`.
fn classify(error: ProxyError) -> Disposition {
    match error {
        ProxyError::ServiceNotFound(_) => Disposition::Unreachable,
        ProxyError::Timeout(_) | ProxyError::Transport(_) => Disposition::Unreachable,
        ProxyError::PermissionDenied(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::UnsupportedProtocol(_) => Disposition::Terminal,
        ProxyError::Callee { .. } => Disposition::Terminal,
        ProxyError::Internal(_) => Disposition::Retry,
    }
}

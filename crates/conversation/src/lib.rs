#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym:conversation`: durable, ordered, end-to-end-encrypted 1:1
//! messaging with outbox `pending`/`delivered`/`failed` state, plus the
//! peer-facing transport underneath it. Not `syneroym-sandbox-wasm`: the
//! store, the ratchet, the outbox, and the delivery worker need no
//! `wasmtime` and are driven from three places (the `HostState` impl, the
//! native shim's delegation, the substrate's own worker loop) — a
//! dependency `syneroym-sandbox-wasm` would drag into all three.

pub mod crypto;
pub mod dag;
pub mod envelope;
pub mod group;
pub mod ids;
mod outbox;
pub mod store;
mod transport;
mod wire;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, RwLock, Weak},
};

use ids::derive_conversation_id;
use rand::RngCore;
use store::{ConversationConfig as StoreConfig, ConversationStore};
use syneroym_async_queue::QueueConfig;
use syneroym_core::local_registry::EndpointRegistry;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{
    ConversationDeliveryState, ConversationError, ConversationHistoryPage, ConversationHost,
    ConversationKind, ConversationMembershipEvent, ConversationMessage, ConversationNotifier,
    ConversationSummary, ServiceProxy,
};

/// Node-level configuration, converted from `AppSandboxRole`'s
/// `conversation_*` fields by the crate's own caller
/// (`crates/substrate/src/runtime.rs`); this crate does not depend on
/// `syneroym-core::config::AppSandboxRole` beyond the plain values it
/// carries.
#[derive(Debug, Clone, Default)]
pub struct ConversationConfig {
    pub store: StoreConfig,
}

pub struct ConversationService {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    /// `OnceLock`, not a constructor parameter: `ConversationService` is
    /// built alongside the blob provider/logical resolver
    /// (`build_route_handler_deps`), before the real `ServiceProxy`
    /// (`ProxyRouter`) exists -- the same ordering `AppSandboxEngine.
    /// service_proxy`/`ControlPlaneService.service_proxy` are already
    /// `OnceLock` for.
    service_proxy: std::sync::OnceLock<Weak<dyn ServiceProxy>>,
    registry: EndpointRegistry,
    crypto: Arc<dyn crypto::SessionCrypto>,
    queue_config: QueueConfig,
    conversation_config: StoreConfig,
    max_clock_skew_secs: u64,
    stores: Mutex<HashMap<String, Arc<ConversationStore>>>,
    open_lock: tokio::sync::Mutex<()>,
    default_notifier: RwLock<Weak<dyn ConversationNotifier>>,
    service_notifiers: Mutex<HashMap<String, Weak<dyn ConversationNotifier>>>,
}

impl std::fmt::Debug for ConversationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationService").finish_non_exhaustive()
    }
}

fn internal(e: impl std::fmt::Display) -> ConversationError {
    ConversationError::Internal(e.to_string())
}

// Lock-poisoning from a panicking holder is a programming error that
// leaves the data in an inconsistent state; there is no safe recovery
// path, matching `syneroym-async-queue`'s own precedent for its `Queue`.
#[allow(clippy::expect_used)]
impl ConversationService {
    pub fn new(
        storage_provider: Arc<dyn StorageProvider>,
        key_store: Arc<KeyStore>,
        registry: EndpointRegistry,
        queue_config: QueueConfig,
        config: ConversationConfig,
    ) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            storage_provider,
            key_store,
            service_proxy: std::sync::OnceLock::new(),
            registry,
            crypto: Arc::new(crypto::X3dhDoubleRatchetCrypto::new()),
            queue_config,
            max_clock_skew_secs: config.store.max_clock_skew_secs,
            conversation_config: config.store,
            stores: Mutex::new(HashMap::new()),
            open_lock: tokio::sync::Mutex::new(()),
            default_notifier: RwLock::new(empty_notifier()),
            service_notifiers: Mutex::new(HashMap::new()),
        }))
    }

    /// Wires the real `ServiceProxy` in after construction (see the field's
    /// own doc). Called at most once, from the node's composition root.
    pub fn set_service_proxy(&self, proxy: Weak<dyn ServiceProxy>) {
        let _ = self.service_proxy.set(proxy);
    }

    pub(crate) fn current_service_proxy(&self) -> Weak<dyn ServiceProxy> {
        self.service_proxy.get().cloned().unwrap_or_else(empty_service_proxy)
    }

    /// The default notification target -- the WASM engine, reached for any
    /// service with no override registered (`§8.2`).
    pub fn set_notifier(&self, notifier: Weak<dyn ConversationNotifier>) {
        #[allow(clippy::expect_used)]
        {
            *self.default_notifier.write().expect("notifier lock poisoned") = notifier;
        }
    }

    /// A natively-linked service's own notification target, registered by
    /// its `NativeHostFactory` (`§6.5`) -- tried before the default.
    pub fn register_service_notifier(
        &self,
        service_id: String,
        notifier: Weak<dyn ConversationNotifier>,
    ) {
        self.service_notifiers.lock().expect("notifier map poisoned").insert(service_id, notifier);
    }

    fn notifier_for(&self, service_id: &str) -> Weak<dyn ConversationNotifier> {
        if let Some(n) =
            self.service_notifiers.lock().expect("notifier map poisoned").get(service_id)
        {
            return n.clone();
        }
        self.default_notifier.read().expect("notifier lock poisoned").clone()
    }

    async fn notify_message(&self, service_id: &str, msg: ConversationMessage) {
        if let Some(n) = self.notifier_for(service_id).upgrade() {
            n.notify_message(service_id, msg).await;
        }
    }

    async fn notify_state(
        &self,
        service_id: &str,
        message_id: String,
        state: ConversationDeliveryState,
    ) {
        if let Some(n) = self.notifier_for(service_id).upgrade() {
            n.notify_delivery_state(service_id, message_id, state).await;
        }
    }

    async fn store_for(&self, service_id: &str) -> anyhow::Result<Arc<ConversationStore>> {
        if let Some(s) = self.stores.lock().expect("store map poisoned").get(service_id) {
            return Ok(s.clone());
        }
        let _guard = self.open_lock.lock().await;
        if let Some(s) = self.stores.lock().expect("store map poisoned").get(service_id) {
            return Ok(s.clone());
        }
        let dek = self.storage_provider.load_service_dek(service_id, &self.key_store).await?;
        let dir = self.storage_provider.service_db_dir(service_id)?;
        let queue_config = self.queue_config.clone();
        let conv_config = self.conversation_config.clone();
        let store = tokio::task::spawn_blocking(move || {
            ConversationStore::open_encrypted(&dir, dek.as_deref(), queue_config, conv_config)
        })
        .await??;
        let store = Arc::new(store);
        self.stores
            .lock()
            .expect("store map poisoned")
            .insert(service_id.to_string(), store.clone());
        Ok(store)
    }

    /// Every service the worker should drain this tick: every store already
    /// open, plus every currently-deployed service with a `conversation.db`
    /// already on disk -- so a restart with a message still `pending`
    /// rediscovers it even if no guest call reopens that service's store
    /// first (failure-matrix row 4).
    fn candidate_service_ids(&self) -> Vec<String> {
        let mut ids: HashSet<String> =
            self.stores.lock().expect("store map poisoned").keys().cloned().collect();
        for (service_id, interface, _) in self.registry.get_all_endpoints() {
            if interface != "conversation" || ids.contains(&service_id) {
                continue;
            }
            if self
                .storage_provider
                .service_db_dir(&service_id)
                .is_ok_and(|dir| dir.join("conversation.db").exists())
            {
                ids.insert(service_id);
            }
        }
        ids.into_iter().collect()
    }

    pub(crate) async fn fetch_prekey_bundle(
        &self,
        svc: &str,
        peer_address: &str,
    ) -> Result<crypto::PrekeyBundle, ConversationError> {
        let bundle_json = match self
            .call_peer(
                svc,
                peer_address,
                "prekey-bundle",
                serde_json::json!({}),
                None,
                // Comfortably inside `dispatch_epoch_timeout_secs` (5s, D-B5-20's own
                // figure): an unreachable peer must still leave the caller enough of
                // its guest budget to do something with the result, e.g. `add-member`
                // still has time to persist a membership entry after this returns.
                Some(std::time::Duration::from_secs(2)),
            )
            .await
        {
            Ok(json) => json,
            Err(transport::Disposition::Unreachable) => {
                return Err(ConversationError::Unreachable("peer unreachable".to_string()));
            }
            Err(transport::Disposition::Terminal(e)) => {
                return Err(ConversationError::InvalidArgument(e));
            }
            Err(_) => {
                return Err(ConversationError::Unreachable(
                    "failed to fetch prekey bundle".to_string(),
                ));
            }
        };
        serde_json::from_value(bundle_json).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable prekey bundle: {e}"))
        })
    }

    pub(crate) async fn enqueue_direct(
        &self,
        store: &ConversationStore,
        service_id: &str,
        peer_address: &str,
        content_type: &str,
        body: &[u8],
        system: bool,
    ) -> Result<String, ConversationError> {
        let conv_id = derive_conversation_id(service_id, peer_address);
        let now = store::now_ms();
        store.get_or_create_direct(peer_address, &conv_id, now).map_err(internal)?;
        if system {
            let conn = store.conn().lock().expect("store lock poisoned");
            let _ = conn.execute(
                "UPDATE conversations SET system = 1 WHERE id = ?1 AND (SELECT COUNT(*) FROM \
                 messages WHERE conversation_id = ?1 AND system = 0) = 0",
                rusqlite::params![conv_id],
            );
        }

        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let message_id =
            ids::derive_message_id(service_id, &conv_id, now, content_type, body, &nonce);

        let identity =
            store.local_identity_or_generate(crypto::generate_identity_bytes).map_err(internal)?;
        let sig_bytes: [u8; 32] =
            identity.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sig_bytes);
        let signature = envelope::sign(
            &signing_key,
            &message_id,
            &conv_id,
            service_id,
            now,
            content_type,
            body,
        );

        store
            .insert_outgoing_and_enqueue(
                &conv_id,
                &message_id,
                service_id,
                now,
                content_type,
                body,
                &signature,
                peer_address,
                now,
                system,
            )
            .map_err(|e| {
                if e.downcast_ref::<store::StoreError>().is_some() {
                    ConversationError::QuotaExceeded
                } else {
                    internal(e)
                }
            })?;
        Ok(message_id)
    }
}

/// An always-empty `Weak<dyn ServiceProxy>` -- mirrors
/// `syneroym_sandbox_wasm::empty_service_proxy` exactly, duplicated here
/// (not shared) for the same reason `control_plane`'s own copy is: no
/// dependency between the two crates for one marker type.
fn empty_service_proxy() -> Weak<dyn ServiceProxy> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl ServiceProxy for NeverConstructed {
        async fn invoke(
            &self,
            _request: syneroym_rpc::ProxyRequest,
        ) -> Result<serde_json::Value, syneroym_rpc::ProxyError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
    }
    Weak::<NeverConstructed>::new()
}

/// An always-empty `Weak<dyn ConversationNotifier>` (`.upgrade()` always
/// returns `None`) -- the placeholder before `set_notifier` is first
/// called, mirroring `syneroym_sandbox_wasm::empty_service_proxy`'s
/// `NeverConstructed` pattern: the inherent `Weak::new()` only exists for
/// `T: Sized`, so an unsized `Weak<dyn ConversationNotifier>` has to come
/// from an unsized coercion off a concrete, never-instantiated type.
fn empty_notifier() -> Weak<dyn ConversationNotifier> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl ConversationNotifier for NeverConstructed {
        async fn notify_message(&self, _service_id: &str, _msg: ConversationMessage) {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
        async fn notify_delivery_state(
            &self,
            _service_id: &str,
            _message_id: String,
            _state: ConversationDeliveryState,
        ) {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
    }
    Weak::<NeverConstructed>::new()
}

#[async_trait::async_trait]
impl ConversationHost for ConversationService {
    async fn open_direct(
        &self,
        service_id: &str,
        peer_address: &str,
    ) -> Result<String, ConversationError> {
        if peer_address.is_empty() || peer_address == service_id {
            return Err(ConversationError::InvalidArgument(
                "peer address must be non-empty and not this service's own address".to_string(),
            ));
        }
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv_id = derive_conversation_id(service_id, peer_address);
        store.get_or_create_direct(peer_address, &conv_id, store::now_ms()).map_err(internal)
    }

    async fn conversations(
        &self,
        service_id: &str,
    ) -> Result<Vec<ConversationSummary>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let rows = store.list_conversations().map_err(internal)?;
        let mut summaries = Vec::new();
        for r in rows {
            let participants = if r.kind == ConversationKind::Group {
                store.current_members(&r.id).unwrap_or_default()
            } else {
                let mut p = vec![service_id.to_string()];
                if let Some(peer) = r.peer_address {
                    p.push(peer);
                }
                p.sort();
                p
            };
            summaries.push(ConversationSummary {
                id: r.id,
                kind: r.kind,
                participants,
                created_at: r.created_at_ms,
                last_activity_at: r.last_activity_ms,
            });
        }
        Ok(summaries)
    }

    async fn send(
        &self,
        service_id: &str,
        conversation: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<String, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        if body.len() as u32 > store.config().max_body_bytes {
            return Err(ConversationError::QuotaExceeded);
        }
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind == ConversationKind::Group {
            return self.send_group(service_id, &store, &conv, content_type, &body).await;
        }
        let peer_address = conv.peer_address.ok_or_else(|| {
            ConversationError::Internal(
                "direct conversation is missing its peer address".to_string(),
            )
        })?;
        self.enqueue_direct(&store, service_id, &peer_address, content_type, &body, false).await
    }

    async fn history(
        &self,
        service_id: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<ConversationHistoryPage, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let page = store.history(conversation, limit, cursor.as_deref()).map_err(internal)?;
        Ok(ConversationHistoryPage {
            messages: page.messages.into_iter().map(store::StoredMessage::into_wire).collect(),
            next_cursor: page.next_cursor,
        })
    }

    async fn delivery_status(
        &self,
        service_id: &str,
        message: &str,
    ) -> Result<ConversationDeliveryState, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        store
            .get_message(message)
            .map_err(internal)?
            .map(|m| m.state)
            .ok_or(ConversationError::NotFound)
    }

    async fn outbox(
        &self,
        service_id: &str,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        Ok(store
            .outbox_messages()
            .map_err(internal)?
            .into_iter()
            .map(store::StoredMessage::into_wire)
            .collect())
    }

    async fn retry(&self, service_id: &str, message: &str) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let msg =
            store.get_message(message).map_err(internal)?.ok_or(ConversationError::NotFound)?;
        if msg.state != ConversationDeliveryState::Failed {
            return Err(ConversationError::InvalidArgument(
                "only a failed message can be retried".to_string(),
            ));
        }
        let conv = store
            .get_conversation(&msg.conversation_id)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        let now = store::now_ms();
        store.set_state(message, ConversationDeliveryState::Pending, None).map_err(internal)?;

        if conv.kind == ConversationKind::Group {
            let failed_members = {
                let conn = store
                    .conn()
                    .lock()
                    .map_err(|_| ConversationError::Internal("store lock poisoned".to_string()))?;
                let mut stmt = conn
                    .prepare(
                        "SELECT member_address FROM message_recipients WHERE message_id = ?1 AND \
                         state = 'failed'",
                    )
                    .map_err(internal)?;
                let mut rows = stmt.query(rusqlite::params![message]).map_err(internal)?;
                let mut failed = Vec::new();
                while let Some(r) = rows.next().map_err(internal)? {
                    failed.push(r.get::<_, String>(0).map_err(internal)?);
                }
                failed
            };

            for m in failed_members {
                store
                    .set_recipient_state(message, &m, ConversationDeliveryState::Pending, None)
                    .map_err(internal)?;
                let payload = serde_json::to_vec(&store::OutboxItem {
                    message_id: message.to_string(),
                    peer_address: m.clone(),
                    group: Some(conv.id.clone()),
                })
                .map_err(internal)?;
                store
                    .queue()
                    .enqueue(&conv.id, &format!("{message}:{m}"), &payload, now)
                    .map_err(internal)?;
            }
        } else {
            let peer_address = conv.peer_address.ok_or_else(|| {
                ConversationError::Internal(
                    "direct conversation is missing its peer address".to_string(),
                )
            })?;
            let payload = serde_json::to_vec(&store::OutboxItem {
                message_id: message.to_string(),
                peer_address,
                group: None,
            })
            .map_err(internal)?;
            store
                .queue()
                .enqueue(&msg.conversation_id, message, &payload, now)
                .map_err(internal)?;
        }
        Ok(())
    }

    async fn create_group(&self, service_id: &str) -> Result<String, ConversationError> {
        self.create_group_impl(service_id).await
    }

    async fn add_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError> {
        self.change_membership_impl(service_id, conversation, member_address, "add").await
    }

    async fn remove_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError> {
        self.change_membership_impl(service_id, conversation, member_address, "remove").await
    }

    async fn members(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<String>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        store.current_members(conversation).map_err(internal)
    }

    async fn membership_history(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<ConversationMembershipEvent>, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        store.membership_history(conversation).map_err(internal)
    }

    async fn sync_now(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<(), ConversationError> {
        self.sync_now_impl(service_id, conversation).await
    }

    async fn group_push(
        &self,
        service_id: &str,
        requester_did: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let req: crate::dag::GroupPushRequest = serde_json::from_slice(&payload).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable group-push payload: {e}"))
        })?;
        let ack = self.group_push_impl(service_id, requester_did, req).await?;
        serde_json::to_vec(&ack).map_err(internal)
    }

    async fn group_sync(
        &self,
        service_id: &str,
        requester_did: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let req: crate::dag::GroupSyncRequest = serde_json::from_slice(&payload).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable group-sync payload: {e}"))
        })?;
        let resp = self.group_sync_impl(service_id, requester_did, req).await?;
        serde_json::to_vec(&resp).map_err(internal)
    }

    async fn prekey_bundle(
        &self,
        service_id: &str,
        requester_did: &str,
    ) -> Result<Vec<u8>, ConversationError> {
        if requester_did.is_empty() {
            return Err(ConversationError::PermissionDenied);
        }
        let store = self.store_for(service_id).await.map_err(internal)?;
        if !store.record_prekey_request(requester_did, store::now_ms()).map_err(internal)? {
            return Err(ConversationError::PermissionDenied);
        }
        let bundle = self
            .crypto
            .prekey_bundle(&store)
            .await
            .map_err(|e| ConversationError::Internal(e.to_string()))?;
        serde_json::to_vec(&bundle).map_err(internal)
    }

    async fn peer_deliver(
        &self,
        service_id: &str,
        requester_did: &str,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError> {
        let env: crypto::Envelope = serde_json::from_slice(&envelope).map_err(|e| {
            ConversationError::InvalidArgument(format!("undecodable envelope: {e}"))
        })?;
        let ack = self.peer_deliver_impl(service_id, requester_did, env).await?;
        serde_json::to_vec(&ack).map_err(internal)
    }
}

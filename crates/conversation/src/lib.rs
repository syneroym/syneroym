#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym:conversation`: durable, ordered, end-to-end-encrypted 1:1
//! messaging with outbox `pending`/`delivered`/`failed` state, plus the
//! peer-facing transport underneath it. Not `syneroym-sandbox-wasm`: the
//! store, the ratchet, the outbox, and the delivery worker need no
//! `wasmtime` and are driven from three places (the `HostState` impl, the
//! native shim's delegation, the substrate's own worker loop) — a
//! dependency `syneroym-sandbox-wasm` would drag into all three.

pub mod crypto;
pub mod envelope;
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
    ConversationMessage, ConversationNotifier, ConversationSummary, ServiceProxy,
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
        Ok(rows
            .into_iter()
            .map(|r| {
                let mut participants = vec![service_id.to_string()];
                if let Some(peer) = r.peer_address {
                    participants.push(peer);
                }
                participants.sort();
                ConversationSummary {
                    id: r.id,
                    kind: r.kind,
                    participants,
                    created_at: r.created_at_ms,
                    last_activity_at: r.last_activity_ms,
                }
            })
            .collect())
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
        let peer_address = conv.peer_address.ok_or_else(|| {
            ConversationError::Internal(
                "direct conversation is missing its peer address".to_string(),
            )
        })?;

        let now = store::now_ms();
        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let message_id =
            ids::derive_message_id(service_id, conversation, now, content_type, &body, &nonce);

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
            conversation,
            service_id,
            now,
            content_type,
            &body,
        );

        store
            .insert_outgoing_and_enqueue(
                conversation,
                &message_id,
                service_id,
                now,
                content_type,
                &body,
                &signature,
                &peer_address,
                now,
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
        let peer_address = conv.peer_address.ok_or_else(|| {
            ConversationError::Internal(
                "direct conversation is missing its peer address".to_string(),
            )
        })?;
        store.set_state(message, ConversationDeliveryState::Pending, None).map_err(internal)?;
        let payload = serde_json::to_vec(&store::OutboxItem {
            message_id: message.to_string(),
            peer_address,
        })
        .map_err(internal)?;
        store
            .queue()
            .enqueue(&msg.conversation_id, message, &payload, store::now_ms())
            .map_err(internal)?;
        Ok(())
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

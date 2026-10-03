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
mod host_impl;
pub mod ids;
mod outbox;
pub mod store;
#[cfg(feature = "test-support")]
pub mod test_support;
mod transport;
mod wire;

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
    time::Duration,
};

use crypto::X3dhDoubleRatchetCrypto;
use ed25519_dalek::SigningKey;
use ids::derive_conversation_id;
use rand::RngCore;
use store::{ConversationConfig as StoreConfig, ConversationStore, StoredMessage};
use syneroym_async_queue::QueueConfig;
use syneroym_core::local_registry::EndpointRegistry;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{
    Admission, ConversationDeliveryState, ConversationError, ConversationMessage,
    ConversationNotifier, NotifyOutcome, ServiceProxy,
};
use tokio::{sync::Mutex as TokioMutex, task};
use transport::Disposition;

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
    service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
    registry: EndpointRegistry,
    crypto: Arc<dyn crypto::SessionCrypto>,
    queue_config: QueueConfig,
    conversation_config: StoreConfig,
    max_clock_skew_secs: u64,
    stores: Mutex<HashMap<String, Arc<ConversationStore>>>,
    open_lock: TokioMutex<()>,
    default_notifier: RwLock<Weak<dyn ConversationNotifier>>,
    service_notifiers: Mutex<HashMap<String, Weak<dyn ConversationNotifier>>>,
}

impl fmt::Debug for ConversationService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversationService").finish_non_exhaustive()
    }
}

fn internal(e: impl fmt::Display) -> ConversationError {
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
            service_proxy: OnceLock::new(),
            registry,
            crypto: Arc::new(X3dhDoubleRatchetCrypto::new()),
            queue_config,
            max_clock_skew_secs: config.store.max_clock_skew_secs,
            conversation_config: config.store,
            stores: Mutex::new(HashMap::new()),
            open_lock: TokioMutex::new(()),
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
    /// service with no override registered.
    pub fn set_notifier(&self, notifier: Weak<dyn ConversationNotifier>) {
        #[allow(clippy::expect_used)]
        {
            *self.default_notifier.write().expect("notifier lock poisoned") = notifier;
        }
    }

    /// A natively-linked service's own notification target, registered by
    /// its `NativeHostFactory` -- tried before the default.
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

    async fn notify_message(&self, service_id: &str, msg: ConversationMessage) -> NotifyOutcome {
        if let Some(n) = self.notifier_for(service_id).upgrade() {
            n.notify_message(service_id, msg).await
        } else {
            NotifyOutcome::NoAnswer
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

    pub(crate) async fn notify_and_apply_admission(
        &self,
        store: &ConversationStore,
        service_id: &str,
        msg: &StoredMessage,
        now: i64,
    ) {
        let ask_timeout = Duration::from_millis(store.config().admission_ask_timeout_ms);
        let outcome = match tokio::time::timeout(
            ask_timeout,
            self.notify_message(service_id, msg.clone().into_wire()),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => NotifyOutcome::NoAnswer,
        };
        match outcome {
            NotifyOutcome::Answered(admission) => {
                let _ = store.apply_admission(&msg.id, &admission, now);
            }
            NotifyOutcome::NoHandler => {
                let _ = store.apply_admission(&msg.id, &Admission::Accept, now);
            }
            NotifyOutcome::NoAnswer => {}
        }
    }

    pub async fn store_for(&self, service_id: &str) -> anyhow::Result<Arc<ConversationStore>> {
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
        let store = task::spawn_blocking(move || {
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
    /// first.
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
                // Comfortably inside `dispatch_epoch_timeout_secs` (5s): an
                // unreachable peer must still leave the caller enough of
                // its guest budget to do something with the result, e.g. `add-member`
                // still has time to persist a membership entry after this returns.
                Some(Duration::from_secs(2)),
            )
            .await
        {
            Ok(json) => json,
            Err(Disposition::Unreachable) => {
                return Err(ConversationError::Unreachable("peer unreachable".to_string()));
            }
            Err(Disposition::Terminal(e)) => {
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

    /// Ingests an inbound message into the service's conversation store and
    /// notifies the host/guest listener, applying any admission decision.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn deliver_inbound(
        &self,
        service_id: &str,
        msg: ConversationMessage,
    ) -> Result<(), ConversationError> {
        let store = self
            .store_for(service_id)
            .await
            .map_err(|e| ConversationError::Internal(e.to_string()))?;
        let now = if msg.received_at > 0 { msg.received_at } else { store::now_ms() };
        let is_deletion_req = msg.content_type == dag::DELETION_REQUEST_CONTENT_TYPE;
        let is_refusal_notice = msg.content_type == dag::REFUSAL_NOTICE_CONTENT_TYPE;
        let is_system = is_deletion_req || is_refusal_notice;

        let mut inserted = false;
        store
            .queue()
            .transaction(|tx, _txq| {
                inserted = store.insert_incoming_if_absent(
                    tx,
                    &msg.conversation,
                    &msg.id,
                    &msg.author,
                    msg.sender_timestamp,
                    &msg.content_type,
                    &msg.body,
                    &[0u8; 64],
                    now,
                    store.config().max_messages_per_conversation,
                )?;
                if is_system {
                    tx.execute(
                        "UPDATE messages SET system = 1, admission = 'accepted' WHERE id = ?1",
                        rusqlite::params![msg.id],
                    )?;
                }
                if is_deletion_req {
                    ConversationStore::handle_inbound_deletion_request(
                        tx,
                        &msg.conversation,
                        &msg.author,
                        &msg.body,
                        now,
                    )?;
                }
                if is_refusal_notice {
                    ConversationStore::handle_inbound_refusal_notice(
                        tx,
                        &msg.conversation,
                        &msg.author,
                        &msg.body,
                    )?;
                }
                Ok(())
            })
            .map_err(|e| ConversationError::Internal(e.to_string()))?;

        if is_system {
            return Ok(());
        }

        if inserted {
            let outcome = self.notify_message(service_id, msg.clone()).await;
            match outcome {
                NotifyOutcome::Answered(admission) => {
                    let _ = store.apply_admission(&msg.id, &admission, now);
                }
                NotifyOutcome::NoHandler => {
                    let _ = store.apply_admission(&msg.id, &Admission::Accept, now);
                }
                NotifyOutcome::NoAnswer => {}
            }
        }
        Ok(())
    }

    /// Updates the delivery state of an outbox message and notifies listeners.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn update_delivery_state(
        &self,
        service_id: &str,
        message_id: &str,
        state: ConversationDeliveryState,
    ) -> Result<(), ConversationError> {
        let store = self
            .store_for(service_id)
            .await
            .map_err(|e| ConversationError::Internal(e.to_string()))?;
        let last_error =
            if state == ConversationDeliveryState::Failed { Some("delivery failed") } else { None };
        let _ = store.set_state(message_id, state, last_error);
        self.notify_state(service_id, message_id.to_string(), state).await;
        Ok(())
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
        let now = store::now_ms();
        let honest_conv_id = derive_conversation_id(service_id, peer_address);
        let (author, sender_ts, conv_id) =
            claimed_send_fields(service_id, now, honest_conv_id.clone());
        #[cfg(feature = "test-support")]
        if conv_id != honest_conv_id {
            ensure_forged_conversation(store, &conv_id, now)?;
        }
        store.get_or_create_direct(peer_address, &honest_conv_id, now).map_err(internal)?;
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
            ids::derive_message_id(&author, &conv_id, sender_ts, content_type, body, &nonce);

        let identity =
            store.local_identity_or_generate(crypto::generate_identity_bytes).map_err(internal)?;
        let sig_bytes: [u8; 32] =
            identity.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let signing_key = SigningKey::from_bytes(&sig_bytes);
        let signature = envelope::sign(
            &signing_key,
            &message_id,
            &conv_id,
            &author,
            sender_ts,
            content_type,
            body,
        );

        store
            .insert_outgoing_and_enqueue(
                &conv_id,
                &message_id,
                &author,
                sender_ts,
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

/// The author, sender timestamp and conversation id a new message claims:
/// the sending service, `now` and the honest id, always, except for one
/// test's one-shot override.
#[cfg(feature = "test-support")]
fn claimed_send_fields(service_id: &str, now: i64, conv_id: String) -> (String, i64, String) {
    let o = test_support::take_send_override(service_id).unwrap_or_default();
    (
        o.author.unwrap_or_else(|| service_id.to_string()),
        o.sender_timestamp_ms.unwrap_or(now),
        o.conversation_id.unwrap_or(conv_id),
    )
}

/// A message row must reference an existing conversation. A forged
/// conversation id names none, so a peerless stub stands in for it; the
/// honest direct conversation with the peer is untouched.
#[cfg(feature = "test-support")]
fn ensure_forged_conversation(
    store: &ConversationStore,
    conv_id: &str,
    now: i64,
) -> Result<(), ConversationError> {
    let conn = store
        .conn()
        .lock()
        .map_err(|_| ConversationError::Internal("store lock poisoned".to_string()))?;
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, kind, peer_address, owner_address, \
         current_epoch, system, created_at, last_activity) VALUES (?1, 'direct', NULL, NULL, 0, \
         0, ?2, ?2)",
        rusqlite::params![conv_id, now],
    )
    .map(|_| ())
    .map_err(internal)
}

#[cfg(not(feature = "test-support"))]
fn claimed_send_fields(service_id: &str, now: i64, conv_id: String) -> (String, i64, String) {
    (service_id.to_string(), now, conv_id)
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
        async fn notify_message(
            &self,
            _service_id: &str,
            _msg: ConversationMessage,
        ) -> NotifyOutcome {
            NotifyOutcome::NoAnswer
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

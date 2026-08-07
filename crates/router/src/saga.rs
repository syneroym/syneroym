//! One node's saga logs (ADR-0023 §7, as amended): one [`SagaLog`] per
//! driving service, opened once, in that service's own
//! encrypted database beside its outbox and its fence records. Shape
//! mirrors [`crate::ProxyOutbox`] closely -- same cache, same single-flight
//! open, same "operator verbs never create a file" rule -- because both are
//! the same problem (one durable per-service SQLCipher store) applied to a
//! different table.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use syneroym_app_orchestration::{
    AppInstanceId, LogicalResolver, LogicalServiceName, LogicalServiceRef,
};
use syneroym_async_queue::{SagaConfig, SagaLog};
use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{ProxyError, QueuedTarget};
use tokio::task;

use crate::{call_dedup::ASYNC_DB_NAME, service_async_db::async_db_location};

/// One node's saga logs: one per driving service, opened once, in that
/// service's own encrypted database beside its outbox and its fence
/// records.
pub struct SagaStore {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    resolver: Arc<LogicalResolver>,
    config: SagaConfig,
    logs: Mutex<HashMap<String, SagaLog>>,
    /// Serialises opening a log, for the same reason `ProxyOutbox`'s does:
    /// two handles to one file are two connections, and the single-writer
    /// invariants inside `SagaLog` would then rest on nothing.
    open_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for SagaStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SagaStore").finish_non_exhaustive()
    }
}

impl SagaStore {
    #[must_use]
    pub fn new(
        storage_provider: Arc<dyn StorageProvider>,
        key_store: Arc<KeyStore>,
        resolver: Arc<LogicalResolver>,
        config: SagaConfig,
    ) -> Self {
        Self {
            storage_provider,
            key_store,
            resolver,
            config,
            logs: Mutex::new(HashMap::new()),
            open_lock: tokio::sync::Mutex::new(()),
        }
    }

    #[must_use]
    pub fn config(&self) -> &SagaConfig {
        &self.config
    }

    /// Opens (once) the saga log for `service_id`, on that service's own
    /// database with that service's own key, creating the file if it is
    /// not there yet.
    ///
    /// Deliberately **not** gated on the service having a `state.db` --
    /// mirrors `ProxyOutbox::queue_for`'s own reasoning exactly. The
    /// operator verbs go through [`Self::existing_log_for`] instead.
    pub async fn log_for(&self, service_id: &str) -> Result<SagaLog, ProxyError> {
        if let Some(log) = self.cached_log(service_id)? {
            return Ok(log);
        }
        let _opening = self.open_lock.lock().await;
        if let Some(log) = self.cached_log(service_id)? {
            return Ok(log);
        }
        let (dir, dek) = async_db_location(&self.storage_provider, &self.key_store, service_id)
            .await
            .map_err(|e| ProxyError::Internal(format!("saga log unavailable: {e}")))?;
        let config = self.config.clone();
        let log = task::spawn_blocking(move || -> anyhow::Result<SagaLog> {
            std::fs::create_dir_all(&dir)?;
            SagaLog::open_encrypted(&dir, ASYNC_DB_NAME, dek.as_deref(), config)
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("saga log task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(format!("saga log unavailable: {e}")))?;

        self.logs
            .lock()
            .map_err(|_| ProxyError::Internal("saga log cache poisoned".to_string()))?
            .insert(service_id.to_string(), log.clone());
        Ok(log)
    }

    fn cached_log(&self, service_id: &str) -> Result<Option<SagaLog>, ProxyError> {
        Ok(self
            .logs
            .lock()
            .map_err(|_| ProxyError::Internal("saga log cache poisoned".to_string()))?
            .get(service_id)
            .cloned())
    }

    /// The saga log for `service_id` **only if it already exists**, for the
    /// operator verbs: an id typed by a human must not bring a log file
    /// into being just by being asked about.
    pub async fn existing_log_for(&self, service_id: &str) -> Result<Option<SagaLog>, ProxyError> {
        if !self.log_file_exists(service_id) {
            return Ok(None);
        }
        self.log_for(service_id).await.map(Some)
    }

    /// Which services the sweep should visit this tick: every one whose log
    /// is already open, mirroring `ProxyOutbox::open_services`.
    #[must_use]
    pub fn open_services(&self) -> Vec<String> {
        self.logs.lock().map(|l| l.keys().cloned().collect()).unwrap_or_default()
    }

    /// Whether `service_id` already has an async database file on disk --
    /// what lets a restart find sagas written before it, when nothing is in
    /// the cache yet.
    #[must_use]
    pub fn log_file_exists(&self, service_id: &str) -> bool {
        self.storage_provider
            .service_db_dir(service_id)
            .is_ok_and(|dir| dir.join(ASYNC_DB_NAME).exists())
    }

    /// Resolves a step's stored target into the DID to deliver to, now --
    /// the same reasoning as `ProxyOutbox::resolve_target`, taken apart
    /// into pieces because a saga step is not a `QueuedCall`.
    pub fn resolve_step_target(
        &self,
        app_instance_id: Option<&str>,
        target: &QueuedTarget,
        routing_key: Option<&str>,
    ) -> Result<String, ProxyError> {
        match target {
            QueuedTarget::Service(did) => Ok(did.clone()),
            QueuedTarget::Dependency(name) => {
                let app_instance_id = app_instance_id.ok_or_else(|| {
                    ProxyError::ServiceNotFound(format!(
                        "saga step dependency '{name}' has no app instance to resolve against"
                    ))
                })?;
                let logical_ref = LogicalServiceRef {
                    app_instance_id: AppInstanceId::try_new(app_instance_id).map_err(|e| {
                        ProxyError::Internal(format!("stored app context is unreadable: {e}"))
                    })?,
                    service_name: LogicalServiceName::try_new(name).map_err(|e| {
                        ProxyError::ServiceNotFound(format!("invalid dependency name: {e}"))
                    })?,
                };
                self.resolver
                    .resolve(&logical_ref, routing_key.map(str::as_bytes))
                    .map(|id| id.to_string())
                    .map_err(|e| {
                        ProxyError::ServiceNotFound(format!(
                            "saga step dependency '{name}' is no longer bound: {e}"
                        ))
                    })
            }
        }
    }
}

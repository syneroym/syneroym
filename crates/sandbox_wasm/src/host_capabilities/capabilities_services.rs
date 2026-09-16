use super::*;

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl Host for HostState {
    async fn get_test_context(&mut self, request_ctx: String) -> String {
        let component_ctx = format!("Component: {}", self.component_id);
        if let Some(existing) = &self.request_ctx {
            format!("{component_ctx} | {existing} | {request_ctx}")
        } else {
            format!("{component_ctx} | {request_ctx}")
        }
    }
}

impl vault::Host for HostState {
    async fn reveal(&mut self, key: String) -> Result<Vec<u8>, VaultError> {
        let provider = self.storage_provider.clone();
        let key_store = self.key_store.clone();
        let service_id = self.component_id.clone();

        let store = match provider.open_service_db(&service_id, &key_store).await {
            Ok(s) => s,
            Err(e) => {
                error!(
                    "Vault reveal failed to open service DB for service_id {}: {}",
                    service_id, e
                );
                return Err(VaultError::Internal(e.to_string()));
            }
        };

        match store.reveal_secret(&key).await {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(VaultError::NotFound),
            Err(e) => {
                error!("Vault reveal failed to read secret for service_id {}: {}", service_id, e);
                Err(VaultError::Internal(e.to_string()))
            }
        }
    }
}

impl signing::Host for HostState {
    async fn sign_record(
        &mut self,
        draft: WitRecordDraft,
        as_principal: WitPrincipal,
    ) -> Result<String, WitSigningError> {
        if self.read_only {
            return Err(WitSigningError::PermissionDenied);
        }
        let Some(signer) = self.record_signer.clone() else {
            return Err(WitSigningError::Internal(
                "this node has no record signer configured".to_string(),
            ));
        };
        let draft = convert_draft_in(draft)?;
        let principal = convert_principal_in(as_principal);
        let caller = caller_binding(&self.caller);
        signer
            .sign_record(&self.component_id, draft, &principal, caller)
            .map_err(convert_signing_error_out)
    }

    async fn identity(&mut self) -> Result<WitSigningIdentity, WitSigningError> {
        let Some(signer) = self.record_signer.clone() else {
            return Err(WitSigningError::Internal(
                "this node has no record signer configured".to_string(),
            ));
        };
        let id = signer.identity(&self.component_id).map_err(convert_signing_error_out)?;
        Ok(convert_identity_out(id))
    }
}

impl invocation::Host for HostState {
    async fn caller(&mut self) -> WitCallerOrigin {
        match self.invocation_origin {
            // A local dispatch is trusted for where it came from, whatever
            // identity the dispatching code chose to put in `caller` --
            // the parity driver hands a verified delegated caller to a
            // purely local drive, and a sibling proxy call and an
            // anonymous wire call arrive with the identical `CallerContext`.
            InvocationOrigin::Local => WitCallerOrigin::Internal,
            InvocationOrigin::Wire => match self.caller.auth {
                AuthLevel::Delegated | AuthLevel::Ucan => {
                    WitCallerOrigin::Verified(self.caller.caller_did.clone())
                }
                _ => WitCallerOrigin::Anonymous,
            },
        }
    }
}

fn caller_binding(caller: &CallerContext) -> CallerBinding<'_> {
    match caller.auth {
        AuthLevel::Delegated | AuthLevel::Ucan => CallerBinding::Verified(&caller.caller_did),
        _ => CallerBinding::Internal,
    }
}

fn convert_draft_in(
    draft: WitRecordDraft,
) -> Result<syneroym_signed_record::RecordDraft, WitSigningError> {
    let payload: Value = serde_json::from_str(&draft.payload)
        .map_err(|e| WitSigningError::InvalidRecord(format!("payload is not valid JSON: {e}")))?;
    Ok(syneroym_signed_record::RecordDraft {
        version: draft.version,
        record_type: draft.record_type,
        subject: draft.subject,
        payload,
        expires_at_secs: draft.expires_at_secs,
        supersedes: draft.supersedes,
    })
}

fn convert_principal_in(principal: WitPrincipal) -> SigningPrincipal {
    match principal {
        WitPrincipal::Service => SigningPrincipal::Service,
        WitPrincipal::Delegated(cert_json) => {
            SigningPrincipal::Delegated { delegation_json: cert_json }
        }
    }
}

fn convert_signing_error_out(err: SigningError) -> WitSigningError {
    use syneroym_core::record_signer::SigningError as SE;
    match err {
        SE::NoDelegation(msg) => WitSigningError::NoDelegation(msg),
        SE::InvalidRecord(msg) => WitSigningError::InvalidRecord(msg),
        SE::PermissionDenied => WitSigningError::PermissionDenied,
        SE::Internal(msg) => WitSigningError::Internal(msg),
    }
}

fn convert_identity_out(id: SigningIdentity) -> WitSigningIdentity {
    WitSigningIdentity {
        signing_did: id.signing_did,
        pubkey_hex: id.pubkey_hex,
        owner_did: id.owner_did,
    }
}

impl host_api::Host for HostState {
    async fn publish(&mut self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        let namespaced = namespace_topic_for_publish(&self.component_id, &topic);
        let broker = self.messaging.broker.clone();
        broker.publish(namespaced, payload).await.map_err(map_broker_error)
    }

    async fn subscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        // A subscription registered from a throw-away stage-4 instance
        // would outlive it, and stage-4 is a local, synchronous read-only
        // lookup, not a place to register egress.
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        let namespaced = namespace_topic(&self.component_id, &topic);
        let service_id = self.component_id.clone();
        let storage_provider = self.storage_provider.clone();
        let engine = self.messaging.engine.clone();

        // Checked before the DB write (rather than after) so a teardown
        // race never leaves a persisted subscription row with no live
        // broker registration behind it.
        let Some(engine) = engine.upgrade() else {
            return Err(MessagingError::Internal(
                "sandbox engine unavailable for subscription registration".to_string(),
            ));
        };

        storage_provider
            .save_messaging_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        engine
            .register_internal_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))
    }

    async fn unsubscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        let namespaced = namespace_topic(&self.component_id, &topic);
        let service_id = self.component_id.clone();
        let storage_provider = self.storage_provider.clone();
        let engine = self.messaging.engine.clone();

        storage_provider
            .delete_messaging_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        // Surfaced as an error (rather than silently `Ok`) since the DB
        // row is already gone at this point: a caller told "success" here
        // while the live subscription stays active would have no way to
        // rediscover and clean it up later, via replay or otherwise.
        let Some(engine) = engine.upgrade() else {
            return Err(MessagingError::Internal(
                "sandbox engine unavailable for subscription deregistration".to_string(),
            ));
        };
        engine.subscriptions.remove(&(service_id, namespaced));
        Ok(())
    }

    async fn register_stream_protocol(&mut self, protocol: String) -> Result<(), MessagingError> {
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        let service_id = self.component_id.clone();
        self.streaming
            .registry
            .register(service_id.clone(), protocol, SubstrateEndpoint::WasmChannel { service_id })
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))
    }
}

impl app_config::Host for HostState {
    async fn get(&mut self, key: String) -> Result<Option<String>, ConfigError> {
        if self.config_generation == 0 {
            return Ok(None);
        }

        let config_str = match self
            .storage_provider
            .get_config_generation(&self.component_id, self.config_generation)
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => {
                error!("Failed to read config for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let config_json: Value = match serde_json::from_str(&config_str) {
            Ok(j) => j,
            Err(e) => {
                error!("Invalid config JSON for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let val = config_json.get(&key).and_then(|v| v.as_str()).map(|s| s.to_string());
        Ok(val)
    }

    async fn get_section(&mut self, prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        if self.config_generation == 0 {
            return Ok(vec![]);
        }

        let config_str = match self
            .storage_provider
            .get_config_generation(&self.component_id, self.config_generation)
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(vec![]),
            Err(e) => {
                error!("Failed to read config for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let config_json: Value = match serde_json::from_str(&config_str) {
            Ok(j) => j,
            Err(e) => {
                error!("Invalid config JSON for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let mut results = vec![];
        if let Value::Object(map) = config_json {
            for (k, v) in map {
                #[allow(clippy::collapsible_if)]
                if k == prefix || k.starts_with(&format!("{prefix}.")) {
                    if let Some(s) = v.as_str() {
                        results.push((k, s.to_string()));
                    }
                }
            }
        }

        Ok(results)
    }
}

impl wasmtime::ResourceLimiter for HostState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        match self.memory_limits.memory_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            _ => Err(wasmtime::Error::msg("MemoryFault: Wasm execution exceeded memory limit")),
        }
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        self.memory_limits.table_growing(current, desired, maximum)
    }
}

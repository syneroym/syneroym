//! Native (non-WASM) JSON-RPC dispatch for a deployed `SynSvc`'s
//! data-layer/vault/app-config/blob-store capabilities.
//!
//! One instance is registered per deployed `service_id` in
//! `ControlPlaneService::deploy` (`crates/control_plane/src/service.rs`),
//! mirroring the same host-provided capabilities the WASM `Host` trait
//! impls in `crates/sandbox_wasm/src/engine.rs` expose to guests -- this is
//! the second, independent adapter over the same underlying
//! `StorageProvider`/`ServiceStore`/`BlobProvider` traits, not a
//! reimplementation of their logic. Does **not** depend on
//! `syneroym-sandbox-wasm`: that crate is an optional, feature-gated
//! dependency of `control_plane` (see `crate::dummy_sandbox`), and native
//! data-layer/blob-store access must work even in builds without the WASM
//! sandbox feature enabled.

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Weak},
};

#[cfg(test)]
use serde_json::Value;
use syneroym_core::record_signer::NodeRecordSigner;
use syneroym_data_blob::{
    BlobProvider,
    traits::{DownloadSession, UploadSession},
};
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::Policy;
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_mqtt_broker::MqttBroker;
use syneroym_rpc::{
    ConversationHost, NativeInvocation, NativeResponse, NativeService, RowAuthorizer, RpcError,
    RpcResult, ServiceProxy,
};
use tokio::sync::Mutex;

mod blob;
mod conversation;
mod data;
mod relation;
mod signing;
pub(crate) use signing::signing_error;
mod vault_config;

#[cfg(test)]
mod tests;

pub(crate) use conversation::empty_conversation_host;
#[cfg(test)]
pub(crate) use data::data_layer_error;
#[cfg(test)]
pub(crate) use signing::parse_principal;

pub struct SynSvcNativeService {
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    messaging_broker: Arc<MqttBroker>,
    upload_sessions: Mutex<HashMap<String, Box<dyn UploadSession>>>,
    download_sessions: Mutex<HashMap<String, Box<dyn DownloadSession>>>,
    /// `None` = unfiltered (today's behavior for a service deployed without
    /// a policy). Set once at construction from the `Arc<Policy>` `deploy`
    /// already parsed/validated (ADR-0017) -- no load, no cache, no parse on
    /// this hot path. A re-deploy reconstructs the service, so a policy edit
    /// takes effect with the deploy that carries it.
    fdae_policy: Option<Arc<Policy>>,
    /// This *service's own* signing identity, derived from the
    /// node's identity via
    /// `Identity::derive_service_identity(owner_did, service_id)`
    /// (ADR-0006 "Model A" pattern) -- **not** the shared node identity
    /// directly. `resolve-relation` signs its returned `RelationshipProof`
    /// as this service's asserter DID
    /// (`derive_did_key(&service_identity.public_key())`), per the
    /// service-asserts-under-its-own-identity model of ADR-0017 §6 and §7:
    /// a substrate node routinely hosts multiple, unrelated services
    /// (multi-tenancy is the normal case), so a shared node-wide signing
    /// identity would make every co-hosted service's assertions
    /// cryptographically indistinguishable from one another, and would let
    /// the node operator forge assertions on any hosted service's behalf.
    /// `owner_did` (the deploying/owning DID recorded by
    /// `ControlPlaneService`'s `registry.owner_of`/`set_owner`) is folded
    /// into the derivation alongside `service_id` so that a `service_id`
    /// freed by undeploy and later redeployed under a different owner gets
    /// a distinct identity rather than inheriting the previous owner's key.
    /// Deterministic and redeploy-stable for the *same* owner (same
    /// derivation every time), so no new persisted key material is needed.
    ///
    /// This key still does every signature (Model A is unchanged); the
    /// instance certificate only changes the DID the signature is *asserted
    /// under* -- with `instance_cert` installed, `resolve_relation`'s
    /// `RelationshipProof`
    /// asserts as the member master the certificate names, not this derived
    /// instance key directly, so a member reinstantiated on another node
    /// keeps satisfying every policy naming its master (ADR-0020 §2).
    service_identity: Identity,
    /// The service's installed instance certificate (ADR-0020 §1), when one
    /// exists -- `deploy` verifies and installs it, and passes the same
    /// value here so `resolve_relation` can sign under the member master it
    /// names. **Held by value from construction; `ProxyRouter` reads the
    /// registry live on every call instead** (`proxy.rs`). Those cannot
    /// drift today because the sole production writer of an instance
    /// certificate (`orchestration.rs`) runs inside `deploy`, which rebuilds
    /// this service in the same pass -- any future code that installs a
    /// certificate outside `deploy` (an unattended renewal, say) must also
    /// refresh this service, or `RelationshipProof::verify`'s wall-clock
    /// check starts rejecting every proof it signs with no fallback
    /// (`asserter_did` is already the master, so there is nothing to fall
    /// back to).
    instance_cert: Option<DelegationCertificate>,
    /// The Universal Proxy, needed for the cross-service
    /// relationship-proof fetch: `resolve_query_auth` calls
    /// out through this to a remote service's `resolve-relation`. `Weak`,
    /// like `HostState.service_proxy` (`sandbox_wasm`) -- `ProxyRouter` is
    /// constructed after `ControlPlaneService`/this struct at startup
    /// (`crates/router/src/route_handler.rs`), so it is threaded in via
    /// `ControlPlaneService.service_proxy`'s post-construction `OnceLock`,
    /// the same two-phase wiring `AppSandboxEngine.service_proxy` already
    /// uses for the identical ordering reason.
    service_proxy: Weak<dyn ServiceProxy>,
    /// The stage-4 ABAC after-step invoker (ADR-0017 §7), same reasoning and
    /// same two-phase `Weak` wiring as `service_proxy`: `AppSandboxEngine`
    /// (the sole implementation) is constructed after this service, so it is
    /// threaded in via a post-construction `OnceLock` at the composition
    /// root. `syneroym_rpc::empty_row_authorizer()` in a build without the
    /// `app_sandbox` feature (`crate::dummy_sandbox`), where a stage-4-opted
    /// policy can never be deployed in the first place (`orchestration.rs`'s
    /// deploy-time gate rejects it) -- this field exists only so the same
    /// four native read/delete sites work unconditionally, without a
    /// `#[cfg]`.
    row_authorizer: Weak<dyn RowAuthorizer>,
    /// The Conversation service, reached by
    /// `dispatch_conversation`. `OnceLock`, not a constructor parameter --
    /// `ConversationService` is constructed once, node-wide, alongside
    /// `ControlPlaneService` itself, and adding it to `Self::new`'s
    /// signature would touch every one of this struct's ~26 existing call
    /// sites for a capability most of them never exercise. Unset
    /// (`.upgrade()` always `None`) on a node running no conversation service.
    conversation: std::sync::OnceLock<Weak<dyn ConversationHost>>,
    record_signer: std::sync::OnceLock<Arc<NodeRecordSigner>>,
}

impl fmt::Debug for SynSvcNativeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SynSvcNativeService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

pub(super) fn internal(msg: impl fmt::Display) -> RpcError {
    RpcError::InternalError(msg.to_string())
}

pub(super) fn invalid_params(msg: impl fmt::Display) -> RpcError {
    RpcError::InvalidParams(msg.to_string())
}

pub(super) fn parse_params<T: serde::de::DeserializeOwned>(
    invocation: &NativeInvocation,
) -> RpcResult<T> {
    serde_json::from_value(invocation.params.clone())
        .map_err(|e| invalid_params(format!("invalid params for {}: {e}", invocation.method)))
}

pub(super) fn to_payload<T: serde::Serialize>(value: &T) -> RpcResult<NativeResponse> {
    serde_json::to_value(value)
        .map(|payload| NativeResponse { payload })
        .map_err(|e| internal(format!("failed to serialize response: {e}")))
}

impl SynSvcNativeService {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service_id: String,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
        fdae_policy: Option<Arc<Policy>>,
        node_identity: Arc<Identity>,
        owner_did: &str,
        service_proxy: Weak<dyn ServiceProxy>,
        row_authorizer: Weak<dyn RowAuthorizer>,
        instance_cert: Option<DelegationCertificate>,
    ) -> Self {
        // Derived here, once, rather than at every call site: every
        // existing (and future) construction site already passes the
        // shared node identity and the deploying owner's DID for exactly
        // this purpose, so deriving internally means no caller needs to
        // know this service-scoping happens at all.
        let service_identity = node_identity.derive_service_identity(owner_did, &service_id);
        Self {
            service_id,
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            upload_sessions: Mutex::new(HashMap::new()),
            download_sessions: Mutex::new(HashMap::new()),
            fdae_policy,
            service_identity,
            instance_cert,
            service_proxy,
            row_authorizer,
            conversation: std::sync::OnceLock::new(),
            record_signer: std::sync::OnceLock::new(),
        }
    }

    pub fn set_record_signer(&self, signer: Arc<NodeRecordSigner>) {
        let _ = self.record_signer.set(signer);
    }

    pub fn set_record_signer_from(&self, cp: &crate::ControlPlaneService) {
        if let Some(s) = cp.record_signer.get().cloned() {
            self.set_record_signer(s);
        }
    }

    /// Wires the Conversation service in after construction.
    /// Called at most once per instance, from the node's own composition
    /// root.
    pub fn set_conversation(&self, conversation: Weak<dyn ConversationHost>) {
        let _ = self.conversation.set(conversation);
    }

    fn current_conversation(&self) -> Weak<dyn ConversationHost> {
        self.conversation.get().cloned().unwrap_or_else(empty_conversation_host)
    }
}

#[async_trait::async_trait]
impl NativeService for SynSvcNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.interface.as_str() {
            "data-layer" => self.dispatch_data_layer(invocation).await,
            "vault" => self.dispatch_vault(invocation).await,
            "app-config" => self.dispatch_app_config(invocation).await,
            "blob-store" => self.dispatch_blob_store(invocation).await,
            "messaging" => self.dispatch_messaging(invocation).await,
            "conversation" => self.dispatch_conversation(invocation).await,
            "signing" => self.dispatch_signing(invocation).await,
            other => Err(RpcError::MethodNotFound(format!("unknown interface: {other}"))),
        }
    }
}

#[cfg(test)]
pub(crate) fn empty_service_proxy() -> Weak<dyn ServiceProxy> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl ServiceProxy for NeverConstructed {
        async fn invoke(
            &self,
            _: syneroym_rpc::ProxyRequest,
        ) -> Result<Value, syneroym_rpc::ProxyError> {
            unimplemented!()
        }
    }
    Arc::downgrade(&(Arc::new(NeverConstructed) as Arc<dyn ServiceProxy>))
}

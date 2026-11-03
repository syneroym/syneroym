use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use syneroym_app_host_native::NativeAppHost;
use syneroym_roym_catalog::native::NativeCatalog;
use syneroym_roym_conversation::native::NativeConversation;
use syneroym_roym_directory::native::NativeDirectory;
use syneroym_roym_profile::native::NativeProfile;
use syneroym_roym_transaction::native::NativeTransaction;
use syneroym_roym_web::native::NativeWeb;
use syneroym_rpc::{
    JsonRpcRequest, NativeInvocation, NativeService, ProxyError, ProxyRequest, ServiceProxy,
};
use syneroym_sandbox_wasm::AppSandboxEngine;

use super::{
    anon_wire_caller, did_for_service, foreign_searcher_caller, foreign_wire_route, trust_harness,
};

#[derive(Debug)]
pub(crate) struct TestWasmServiceProxy {
    pub(crate) engine: Arc<AppSandboxEngine>,
    pub(crate) invocations: AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestWasmServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let target = request.target_service.as_str();

        if let Some(canned) = trust_harness::canned_source_response(target, &request.params).await {
            return Ok(canned);
        }

        if let Some((service_id, anon)) = foreign_wire_route(target) {
            let caller = if anon { anon_wire_caller() } else { foreign_searcher_caller() };
            let rpc_req = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: request.method,
                params: request.params,
                id: None,
                idempotency_key: request.idempotency_key,
            };
            return self
                .engine
                .execute_wasm_json_from_wire(
                    &service_id,
                    &request.interface,
                    &rpc_req,
                    Some(caller),
                )
                .await
                .map_err(|e| ProxyError::Internal(e.to_string()));
        }

        let service_id = if target == "did:key:hForeign" {
            did_for_service("directory")
        } else {
            target.to_string()
        };

        if service_id.contains("unbound") {
            return Err(ProxyError::ServiceNotFound("unbound".to_string()));
        }
        if service_id.contains("timeout") {
            return Err(ProxyError::Timeout(Duration::from_secs(30)));
        }

        let rpc_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: request.method,
            params: request.params,
            id: None,
            idempotency_key: request.idempotency_key,
        };

        let res = self
            .engine
            .execute_wasm_json(&service_id, &request.interface, &rpc_req, Some(request.caller))
            .await;

        match res {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!(
                    "TestWasmServiceProxy error invoking {service_id} {}: {e:?}",
                    request.interface
                );
                Err(ProxyError::Internal(e.to_string()))
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct TestNativeServiceProxy {
    pub(crate) web: Arc<NativeWeb<NativeAppHost>>,
    pub(crate) profile: Arc<NativeProfile<NativeAppHost>>,
    pub(crate) conversation: Arc<NativeConversation<NativeAppHost>>,
    pub(crate) catalog: Arc<NativeCatalog<NativeAppHost>>,
    pub(crate) transaction: Arc<NativeTransaction<NativeAppHost>>,
    pub(crate) directory: Arc<NativeDirectory<NativeAppHost>>,
    /// `host_for_wire`-built instances the foreign-wire targets route to,
    /// so a fan-out call is a real wire round trip on the native stack too.
    pub(crate) directory_wire: Arc<dyn NativeService>,
    pub(crate) directory2_wire: Arc<dyn NativeService>,
    pub(crate) invocations: AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestNativeServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let target = request.target_service.as_str();

        if let Some(canned) = trust_harness::canned_source_response(target, &request.params).await {
            return Ok(canned);
        }

        if let Some((_service_id, anon)) = foreign_wire_route(target) {
            let svc = if target == "did:key:hForeignWire2" {
                &self.directory2_wire
            } else {
                &self.directory_wire
            };
            let caller = if anon { anon_wire_caller() } else { foreign_searcher_caller() };
            let inv = NativeInvocation {
                interface: request.interface,
                method: request.method,
                params: request.params,
                caller,
            };
            return svc
                .dispatch(inv)
                .await
                .map(|r| r.payload)
                .map_err(|e| ProxyError::Internal(e.to_string()));
        }

        let svc: Arc<dyn NativeService> = if target == "did:key:hForeign" {
            self.directory.clone()
        } else if target == did_for_service("profile") {
            self.profile.clone()
        } else if target == did_for_service("conversation") {
            self.conversation.clone()
        } else if target == did_for_service("catalog") {
            self.catalog.clone()
        } else if target == did_for_service("transaction") {
            self.transaction.clone()
        } else if target == did_for_service("directory") {
            self.directory.clone()
        } else if target == did_for_service("web") {
            self.web.clone()
        } else {
            return Err(ProxyError::ServiceNotFound(target.to_string()));
        };

        let inv = NativeInvocation {
            interface: request.interface,
            method: request.method,
            params: request.params,
            caller: request.caller,
        };

        let res = svc.dispatch(inv).await;
        match res {
            Ok(resp) => Ok(resp.payload),
            Err(e) => Err(ProxyError::Internal(e.to_string())),
        }
    }
}

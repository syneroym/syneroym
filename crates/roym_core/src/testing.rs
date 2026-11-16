//! Shared test host stub for unit and integration testing of Roym services.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use syneroym_app_host::{
    AppInvocation, AppProxy, AppSigning,
    types::{
        data_layer::{DataLayerError, QueryOptions, QueryResult},
        invocation::CallerOrigin,
        proxy::{CallOptions, CallRequest, CallTarget, ProxyError},
        signing::{Principal, RecordDraft, SigningError, SigningIdentity},
    },
};

pub mod data_layer;
pub mod stubs;

pub use stubs::{DummyReader, DummyWriter};

#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub target: CallTarget,
    pub interface: String,
    pub method: String,
    pub params: String,
    pub options: Option<CallOptions>,
}

type SignerFn =
    Box<dyn Fn(RecordDraft, Principal, u64) -> Result<String, SigningError> + Send + Sync>;
type CallHandler = Arc<dyn Fn(String) -> Result<String, ProxyError> + Send + Sync>;

/// In-memory `AppHost` test implementation with storage and fault injection.
#[derive(Default)]
pub struct TestHost {
    pub(crate) storage: Mutex<HashMap<String, HashMap<String, Vec<u8>>>>,
    pub(crate) signing_id: Mutex<Option<SigningIdentity>>,
    pub(crate) caller_origin: Mutex<Option<CallerOrigin>>,
    pub(crate) query_pages: Mutex<VecDeque<QueryResult>>,
    pub(crate) queries: Mutex<Vec<(String, QueryOptions)>>,
    pub(crate) proxy_response: Mutex<Option<Result<String, ProxyError>>>,
    pub(crate) last_call: Mutex<Option<RecordedCall>>,
    pub(crate) now_secs: Mutex<u64>,
    pub(crate) record_signer: Mutex<Option<SignerFn>>,
    pub(crate) crash_prefix: Mutex<Option<(String, String)>>,
    pub(crate) is_crashed: Mutex<bool>,
    pub(crate) call_handlers: Mutex<HashMap<(String, String), CallHandler>>,
    pub(crate) sent_cards: Mutex<Vec<String>>,
}

impl std::fmt::Debug for TestHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHost")
            .field("storage", &self.storage)
            .field("signing_id", &self.signing_id)
            .field("caller_origin", &self.caller_origin)
            .field("queries", &self.queries)
            .field("last_call", &self.last_call)
            .field("now_secs", &self.now_secs)
            .finish()
    }
}

impl TestHost {
    pub fn set_storage_row<T: serde::Serialize>(&self, collection: &str, id: &str, val: &T) {
        let payload = serde_json::to_vec(val).expect("serialize value");
        self.storage
            .lock()
            .unwrap()
            .entry(collection.to_string())
            .or_default()
            .insert(id.to_string(), payload);
    }
    pub fn set_signing_id(&self, id: SigningIdentity) {
        *self.signing_id.lock().unwrap() = Some(id);
    }

    pub fn set_caller_origin(&self, origin: CallerOrigin) {
        *self.caller_origin.lock().unwrap() = Some(origin);
    }

    pub fn push_query_page(&self, page: QueryResult) {
        self.query_pages.lock().unwrap().push_back(page);
    }

    pub fn queries(&self) -> Vec<(String, QueryOptions)> {
        self.queries.lock().unwrap().clone()
    }

    pub fn last_call(&self) -> Option<RecordedCall> {
        self.last_call.lock().unwrap().clone()
    }

    pub fn set_proxy_response(&self, res: Result<String, ProxyError>) {
        *self.proxy_response.lock().unwrap() = Some(res);
    }

    pub fn on_call<F>(&self, service: &str, method: &str, handler: F)
    where
        F: Fn(String) -> Result<String, ProxyError> + Send + Sync + 'static,
    {
        self.call_handlers
            .lock()
            .unwrap()
            .insert((service.to_string(), method.to_string()), Arc::new(handler));
    }

    pub fn set_record_signer<F>(&self, f: F)
    where
        F: Fn(RecordDraft, Principal, u64) -> Result<String, SigningError> + Send + Sync + 'static,
    {
        *self.record_signer.lock().unwrap() = Some(Box::new(f));
    }

    pub fn set_now(&self, secs: u64) {
        *self.now_secs.lock().unwrap() = secs;
    }

    pub fn now(&self) -> u64 {
        *self.now_secs.lock().unwrap()
    }

    pub fn crash_on_write(&self, collection: impl Into<String>, key_prefix: impl Into<String>) {
        *self.crash_prefix.lock().unwrap() = Some((collection.into(), key_prefix.into()));
    }

    pub fn restart(&self) {
        *self.is_crashed.lock().unwrap() = false;
        *self.crash_prefix.lock().unwrap() = None;
    }

    pub fn sent_cards(&self) -> Vec<String> {
        self.sent_cards.lock().unwrap().clone()
    }

    pub(crate) fn check_write_fault(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<(), DataLayerError> {
        if *self.is_crashed.lock().unwrap() {
            return Err(DataLayerError::Internal("test host crashed".to_string()));
        }
        if let Some((target_col, target_prefix)) = self.crash_prefix.lock().unwrap().as_ref()
            && collection == target_col
            && (target_prefix == "*" || id.starts_with(target_prefix))
        {
            *self.is_crashed.lock().unwrap() = true;
            return Err(DataLayerError::Internal(format!(
                "injected crash on write to {collection}/{id}"
            )));
        }
        Ok(())
    }
}

impl AppInvocation for TestHost {
    async fn caller(&self) -> CallerOrigin {
        self.caller_origin.lock().unwrap().clone().unwrap_or(CallerOrigin::Internal)
    }
}

impl AppSigning for TestHost {
    async fn signing_identity(&self) -> Result<SigningIdentity, SigningError> {
        let id = self.signing_id.lock().unwrap();
        id.clone().ok_or_else(|| SigningError::Internal("no signing identity set".to_string()))
    }

    async fn sign_record(
        &self,
        draft: RecordDraft,
        principal: Principal,
    ) -> Result<String, SigningError> {
        let now = *self.now_secs.lock().unwrap();
        let signer = self.record_signer.lock().unwrap();
        if let Some(ref s) = *signer {
            s(draft, principal, now)
        } else {
            unimplemented!("no record signer set on TestHost")
        }
    }
}

impl AppProxy for TestHost {
    async fn call(
        &self,
        target: CallTarget,
        iface: String,
        method: String,
        params: String,
        opts: Option<CallOptions>,
    ) -> Result<String, ProxyError> {
        *self.last_call.lock().unwrap() = Some(RecordedCall {
            target: target.clone(),
            interface: iface,
            method: method.clone(),
            params: params.clone(),
            options: opts,
        });

        let service = match target {
            CallTarget::Dependency(ref s) | CallTarget::Service(ref s) => s.clone(),
        };

        let (method_name, inner_params) = if method == "invoke" {
            extract_invoke_method_and_params(&params).unwrap_or((method, params))
        } else {
            (method, params)
        };

        let handler = {
            let handlers = self.call_handlers.lock().unwrap();
            handlers.get(&(service.clone(), method_name.clone())).cloned()
        };

        if let Some(h) = handler {
            let res = h(inner_params)?;
            let is_rpc_envelope = serde_json::from_str::<serde_json::Value>(&res)
                .ok()
                .is_some_and(|v| v.get("result").is_some() || v.get("error").is_some());
            if is_rpc_envelope {
                return Ok(res);
            }
            let parsed_val = serde_json::from_str::<serde_json::Value>(&res)
                .unwrap_or(serde_json::Value::String(res));
            return Ok(serde_json::json!({ "result": parsed_val }).to_string());
        }

        if service == "conversation" && method_name == "conversation.send" {
            return self.default_conversation_send(&inner_params);
        }

        if service == "conversation" && method_name == "conversation.changes" {
            return Ok(serde_json::json!({
                "result": {
                    "messages": [],
                    "next_seq": 0,
                    "next_cursor": null
                }
            })
            .to_string());
        }

        if let Some(res) = self.proxy_response.lock().unwrap().clone() {
            return res;
        }

        unimplemented!("no proxy response or handler registered for {service}.{method_name}")
    }

    async fn call_all(
        &self,
        _calls: Vec<CallRequest>,
    ) -> Result<Vec<Result<String, ProxyError>>, ProxyError> {
        unimplemented!()
    }

    async fn enqueue(
        &self,
        _target: CallTarget,
        _iface: String,
        _method: String,
        _params: String,
        _opts: Option<CallOptions>,
    ) -> Result<(), ProxyError> {
        unimplemented!()
    }
}

impl TestHost {
    fn default_conversation_send(&self, inner_params: &str) -> Result<String, ProxyError> {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(inner_params)
            && let Some(body) = val.get("body").and_then(|b| b.as_str())
        {
            self.sent_cards.lock().unwrap().push(body.to_string());
        }
        let count = self.sent_cards.lock().unwrap().len();
        let now_ms = (*self.now_secs.lock().unwrap() as i64) * 1000;
        Ok(serde_json::json!({
            "result": {
                "message_id": format!("msg_{count}"),
                "state": "sent",
                "sender_timestamp_ms": now_ms
            }
        })
        .to_string())
    }
}

fn extract_invoke_method_and_params(params: &str) -> Option<(String, String)> {
    let val = serde_json::from_str::<serde_json::Value>(params).ok()?;
    let arr = val.as_array()?;
    let first = arr.first()?;
    let method = first.get("method").and_then(|v| v.as_str())?.to_string();
    let inner = first.get("params").cloned().unwrap_or(serde_json::Value::Null).to_string();
    Some((method, inner))
}

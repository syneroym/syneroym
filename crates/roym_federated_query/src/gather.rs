//! Request building, proxy call fan-out, and remote response parsing.

use serde_json::json;
use syneroym_app_host::{
    AppProxy,
    types::proxy::{CallOptions, CallRequest, CallTarget, ProxyError},
};
use syneroym_roym_core::{
    directory::{
        FIND_SOURCE_TIMEOUT_MS, MAX_REFUSED_RESULTS, MAX_STORED_PER_SOURCE, SearchHit, SearchQuery,
        SourceError,
    },
    envelope::Response,
    method::directory::SearchOutput,
    services,
};

pub const MAX_SEARCH_REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub did: String,
    pub pinned_issuer: Option<String>,
}

#[derive(Debug)]
pub enum SourceOutcome {
    Success { source: Source, hits: Vec<SearchHit>, truncated: bool },
    Error { source: Source, error: SourceError },
}

pub fn build_requests(sources: &[Source], query: &SearchQuery) -> Vec<CallRequest> {
    let params_str = json!({ "method": "directory.search", "params": query }).to_string();
    let body = json!([params_str]).to_string();
    sources
        .iter()
        .map(|s| CallRequest {
            target: CallTarget::Service(s.did.clone()),
            interface: services::DIRECTORY.interface.to_string(),
            method: "invoke".to_string(),
            params: body.clone(),
            options: Some(CallOptions {
                protocol: None,
                idempotent: true,
                timeout_ms: Some(FIND_SOURCE_TIMEOUT_MS),
                routing_key: None,
                idempotency_key: None,
            }),
        })
        .collect()
}

pub async fn execute_and_parse<P: AppProxy>(
    proxy: &P,
    sources: &[Source],
    requests: Vec<CallRequest>,
) -> Vec<SourceOutcome> {
    let call_results = match proxy.call_all(requests).await {
        Ok(results) => results,
        Err(_) => {
            return sources
                .iter()
                .cloned()
                .map(|source| SourceOutcome::Error { source, error: SourceError::NotStarted })
                .collect();
        }
    };

    sources
        .iter()
        .cloned()
        .zip(call_results)
        .map(|(source, res)| parse_single_response(source, res))
        .collect()
}

fn parse_single_response(source: Source, res: Result<String, ProxyError>) -> SourceOutcome {
    let text = match res {
        Ok(t) => t,
        Err(e) => return SourceOutcome::Error { source, error: map_proxy_error(&e) },
    };

    if text.len() > MAX_SEARCH_REPLY_BYTES {
        return SourceOutcome::Error {
            source,
            error: SourceError::Unreadable { reason: "reply too large".to_string() },
        };
    }

    let resp: Response = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            return SourceOutcome::Error {
                source,
                error: SourceError::Unreadable { reason: e.to_string() },
            };
        }
    };

    let Some(result_val) = resp.result else {
        let code = resp.error.as_ref().map_or(-32603, |e| e.code);
        let message = resp.error.map_or_default(|e| e.message);
        return SourceOutcome::Error { source, error: SourceError::Refused { code, message } };
    };

    let search_output: SearchOutput = match serde_json::from_value(result_val) {
        Ok(out) => out,
        Err(e) => {
            return SourceOutcome::Error {
                source,
                error: SourceError::Unreadable { reason: e.to_string() },
            };
        }
    };

    let mut hits = search_output.hits;
    hits.truncate((MAX_STORED_PER_SOURCE + MAX_REFUSED_RESULTS) as usize);

    SourceOutcome::Success { source, hits, truncated: search_output.truncated }
}

pub fn map_proxy_error(e: &ProxyError) -> SourceError {
    match e {
        ProxyError::ServiceNotFound(_) | ProxyError::DependencyNotBound(_) => SourceError::NotFound,
        ProxyError::TimedOut => SourceError::TimedOut,
        ProxyError::Callee(c) => {
            SourceError::Refused { code: i64::from(c.code), message: c.message.clone() }
        }
        other => SourceError::Refused { code: -32603, message: format!("{other:?}") },
    }
}

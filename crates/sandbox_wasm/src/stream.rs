//! M3B Slice 6B bidirectional streaming (ADR-0014).
//!
//! `stream-cursor`/`stream-sink` are guest-**implemented** WIT resources --
//! the reverse of `blob-writer`/`blob-reader` -- so the host calls methods on
//! a `ResourceAny` the guest returned, via the same dynamic
//! `get_export`/`Func::call_async` pattern `AppSandboxEngine::get_wasm_func`
//! already uses for plain functions, generalized to resource methods
//! (confirmed working against wasmtime 46.0.1 by this slice's day-0 spike;
//! see the ADR). [`GuestStreamCursor`]/[`GuestStreamSink`] each own a
//! dedicated `Store`/`Instance` for one stream's lifetime -- unlike every
//! other invocation path in this crate, which gets a fresh `Store` per call.

use std::{
    fmt::{self, Debug, Formatter},
    sync::Weak,
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use syneroym_chunk_transfer::{ChunkSink, ChunkSource};
use syneroym_core::local_registry::EndpointRegistry;
use tokio::task::AbortHandle;
use wasmtime::{
    Store,
    component::{Instance, ResourceAny, Val},
};

use crate::engine::{AppSandboxEngine, HostState};

/// WIT-package-qualified name of the `stream-types` interface, matching how
/// `AppSandboxEngine::deliver_message` names `guest-api` (the short interface
/// name alone doesn't resolve -- see that function's own comment on the
/// bug this caused in Slice 6A).
pub(crate) const STREAM_TYPES_INTERFACE: &str = "syneroym:messaging/stream-types@0.1.0";

/// Bundles the streaming-specific pieces of `HostState`: the registry
/// `register-stream-protocol` writes into, and a weak handle back to the
/// owning `AppSandboxEngine` -- mirrors [`crate::MessagingContext`] exactly.
#[derive(Debug, Clone)]
pub struct StreamContext {
    pub registry: EndpointRegistry,
    pub engine: Weak<AppSandboxEngine>,
}

/// Per-service tracking of open-stream Tokio tasks, so `stop_wasm`/
/// `undeploy` can abort them explicitly (mirrors today's
/// `AppSandboxEngine::unsubscribe_all`) -- but also so *any other* teardown
/// path (e.g. the whole `AppSandboxEngine` being dropped) aborts them too.
/// A bare `tokio::task::AbortHandle` does nothing on `Drop`, so this wrapper
/// exists specifically to backstop that gap; `SubscriptionHandle` (Slice 6A)
/// doesn't need an equivalent because it actively unsubscribes on drop.
#[derive(Debug, Default)]
pub struct StreamRegistry {
    handles: DashMap<String, Vec<AbortHandle>>,
}

impl StreamRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Checked before opening a new stream instance (not while holding the
    /// eventual `AbortHandle`, which only exists after `tokio::spawn` --
    /// see `AppSandboxEngine::handle_stream_protocol_request`). Prunes
    /// finished handles first so the cap isn't held hostage by streams that
    /// already completed.
    pub fn check_capacity(&self, service_id: &str, max_concurrent: u32) -> Result<()> {
        let mut entry = self.handles.entry(service_id.to_string()).or_default();
        entry.retain(|h| !h.is_finished());
        if entry.len() as u32 >= max_concurrent {
            return Err(anyhow!(
                "service '{service_id}' has reached its concurrent stream limit ({max_concurrent})"
            ));
        }
        Ok(())
    }

    /// Registers `handle` under `service_id`, unconditionally -- call only
    /// after `check_capacity` has already passed for this stream.
    pub fn track(&self, service_id: &str, handle: AbortHandle) {
        self.handles.entry(service_id.to_string()).or_default().push(handle);
    }

    /// Removes `handle` from tracking (called once its stream's task
    /// finishes normally, so completed streams don't count against the cap
    /// or get double-aborted later).
    pub fn untrack(&self, service_id: &str, handle: &AbortHandle) {
        if let Some(mut entry) = self.handles.get_mut(service_id) {
            entry.retain(|h| h.id() != handle.id());
        }
    }

    /// Aborts and drops every tracked handle for `service_id` (called from
    /// `stop_wasm`/`undeploy`, mirroring `unsubscribe_all`).
    pub fn abort_all(&self, service_id: &str) {
        if let Some((_, handles)) = self.handles.remove(service_id) {
            for handle in handles {
                handle.abort();
            }
        }
    }
}

impl Drop for StreamRegistry {
    fn drop(&mut self) {
        for entry in self.handles.iter() {
            for handle in entry.value() {
                handle.abort();
            }
        }
    }
}

/// Converts a `Vec<u8>` into the `Val::List(Vec<Val::U8>)` shape wasmtime's
/// dynamic API uses to represent a WIT `list<u8>`.
fn bytes_to_val_list(data: Vec<u8>) -> Val {
    Val::List(data.into_iter().map(Val::U8).collect())
}

/// The inverse of [`bytes_to_val_list`].
fn val_list_to_bytes(val: &Val) -> Result<Vec<u8>> {
    let Val::List(items) = val else {
        return Err(anyhow!("expected Val::List, got {val:?}"));
    };
    items
        .iter()
        .map(|v| match v {
            Val::U8(b) => Ok(*b),
            other => Err(anyhow!("expected Val::U8 inside list, got {other:?}")),
        })
        .collect()
}

/// Unwraps a `result<T, string>` shaped `Val::Result`, calling
/// `ok_extractor` on the boxed `Ok` payload (or treating a `None` payload as
/// `Ok(_)` for `result<_, string>`'s unit-ok case, handled by callers that
/// pass an extractor tolerating `None`). Every guest export in this slice's
/// WIT returns this shape, so this is the one place that interprets it.
fn extract_result<T>(val: &Val, ok_extractor: impl FnOnce(Option<&Val>) -> Result<T>) -> Result<T> {
    match val {
        Val::Result(Ok(payload)) => ok_extractor(payload.as_deref()),
        Val::Result(Err(Some(boxed))) => match boxed.as_ref() {
            Val::String(msg) => Err(anyhow!("{msg}")),
            other => Err(anyhow!("guest returned a non-string error payload: {other:?}")),
        },
        Val::Result(Err(None)) => Err(anyhow!("guest declined the request")),
        other => Err(anyhow!("expected Val::Result, got {other:?}")),
    }
}

/// Calls `[method]<resource>.<method_name>` on `resource`, re-arming the
/// epoch deadline and fuel budget first (a long-lived stream instance
/// otherwise inherits only its *original* instantiation-time budget --
/// see ADR-0014 "Instance Lifetime and Quota"). `extra_args` are appended
/// after the implicit resource receiver.
async fn call_resource_method(
    store: &mut Store<HostState>,
    instance: &Instance,
    max_instructions: Option<u64>,
    resource: ResourceAny,
    method_name: &str,
    extra_args: &[Val],
) -> Result<Vec<Val>> {
    store.set_epoch_deadline(50);
    if let Some(instructions) = max_instructions {
        store.set_fuel(instructions)?;
    }

    let (func, results_len, _item) = AppSandboxEngine::get_wasm_func(
        store,
        instance,
        Some(STREAM_TYPES_INTERFACE),
        method_name,
    )?;

    let mut args = Vec::with_capacity(1 + extra_args.len());
    args.push(Val::Resource(resource));
    args.extend_from_slice(extra_args);

    let mut results = vec![Val::Bool(false); results_len];
    func.call_async(&mut *store, &args, &mut results).await?;
    Ok(results)
}

/// Drops `resource` via `resource_drop_async`, ignoring the result: a guest
/// whose `Store` already trapped or panicked must not cause a panic in the
/// host's own cleanup path (ADR-0014).
async fn drop_resource_ignore_errors(store: &mut Store<HostState>, resource: ResourceAny) {
    let _ = resource.resource_drop_async(store).await;
}

const GUEST_API_INTERFACE: &str = "syneroym:messaging/guest-api@0.1.0";

/// Calls `guest-api::handle-stream-request(protocol, peer-id, request-data)`
/// dynamically, returning the guest's `stream-cursor` resource if it
/// accepts. Any failure here -- the guest returning `Err`, the export not
/// existing, or the call trapping -- is treated identically by the caller:
/// decline the stream cleanly (ADR-0014).
pub(crate) async fn call_handle_stream_request(
    store: &mut Store<HostState>,
    instance: &Instance,
    protocol: &str,
    peer_id: &str,
    request_data: Vec<u8>,
) -> Result<ResourceAny> {
    let (func, results_len, _item) = AppSandboxEngine::get_wasm_func(
        store,
        instance,
        Some(GUEST_API_INTERFACE),
        "handle-stream-request",
    )?;
    let args = [
        Val::String(protocol.to_string()),
        Val::String(peer_id.to_string()),
        bytes_to_val_list(request_data),
    ];
    let mut results = vec![Val::Bool(false); results_len];
    func.call_async(&mut *store, &args, &mut results).await?;
    extract_result(&results[0], |payload| match payload {
        Some(Val::Resource(resource)) => Ok(*resource),
        other => Err(anyhow!("handle-stream-request: expected Val::Resource, got {other:?}")),
    })
}

/// Calls `guest-api::accept-stream-upload(protocol, peer-id, metadata)`
/// dynamically, returning the guest's `stream-sink` resource if it accepts.
/// `metadata` is UTF-8-lossy-decoded from the framed initial payload bytes
/// the router read, since the WIT signature takes `metadata: string` (not
/// `list<u8>`).
pub(crate) async fn call_accept_stream_upload(
    store: &mut Store<HostState>,
    instance: &Instance,
    protocol: &str,
    peer_id: &str,
    metadata: Vec<u8>,
) -> Result<ResourceAny> {
    let (func, results_len, _item) = AppSandboxEngine::get_wasm_func(
        store,
        instance,
        Some(GUEST_API_INTERFACE),
        "accept-stream-upload",
    )?;
    let metadata_str = String::from_utf8_lossy(&metadata).into_owned();
    let args = [
        Val::String(protocol.to_string()),
        Val::String(peer_id.to_string()),
        Val::String(metadata_str),
    ];
    let mut results = vec![Val::Bool(false); results_len];
    func.call_async(&mut *store, &args, &mut results).await?;
    extract_result(&results[0], |payload| match payload {
        Some(Val::Resource(resource)) => Ok(*resource),
        other => Err(anyhow!("accept-stream-upload: expected Val::Resource, got {other:?}")),
    })
}

/// Owns the `Store`/`Instance`/`ResourceAny` backing one guest-as-source
/// stream (`stream-cursor`) for its whole lifetime. Implements
/// [`ChunkSource`] so the host's pull loop
/// (`syneroym_chunk_transfer::pull_until_eof`) can drive it without knowing
/// anything about Wasmtime.
pub struct GuestStreamCursor {
    store: Store<HostState>,
    instance: Instance,
    resource: ResourceAny,
    max_instructions: Option<u64>,
}

impl Debug for GuestStreamCursor {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestStreamCursor").finish_non_exhaustive()
    }
}

impl GuestStreamCursor {
    #[must_use]
    pub fn new(
        store: Store<HostState>,
        instance: Instance,
        resource: ResourceAny,
        max_instructions: Option<u64>,
    ) -> Self {
        Self { store, instance, resource, max_instructions }
    }
}

#[async_trait]
impl ChunkSource for GuestStreamCursor {
    /// On a terminal outcome (`Ok(None)` EOF, or `Err`), drops the guest
    /// resource before returning -- `resource_drop_async` is async and
    /// there is no sound way to run it from a synchronous `Drop::drop`, so
    /// self-cleanup on the terminal call is this type's only cleanup path
    /// (mirrors `stream-sink`'s `finalize`/`abort`, which are likewise
    /// explicit async calls rather than relying on `Drop`). If the *caller*
    /// (e.g. a failed write on the destination side) abandons the cursor
    /// after a non-terminal `Some(chunk)` without calling `next_chunk`
    /// again, the guest's resource-drop hook is skipped -- the `Store`
    /// itself is still torn down safely by its own synchronous `Drop` impl,
    /// just without that courtesy call into guest code.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let outcome = async {
            let results = call_resource_method(
                &mut self.store,
                &self.instance,
                self.max_instructions,
                self.resource,
                "[method]stream-cursor.next-chunk",
                &[],
            )
            .await?;

            extract_result(&results[0], |payload| match payload {
                None => Err(anyhow!("next-chunk: expected option<list<u8>>, got no payload")),
                Some(Val::Option(Some(inner))) => val_list_to_bytes(inner).map(Some),
                Some(Val::Option(None)) => Ok(None),
                Some(other) => Err(anyhow!("next-chunk: expected Val::Option, got {other:?}")),
            })
        }
        .await;

        // Any outcome other than "got a chunk, keep going" is terminal:
        // clean EOF, a guest `Err`, or a call-level failure (trap, missing
        // export) all end the stream here.
        if !matches!(outcome, Ok(Some(_))) {
            drop_resource_ignore_errors(&mut self.store, self.resource).await;
        }

        outcome
    }
}

/// Owns the `Store`/`Instance`/`ResourceAny` backing one guest-as-sink
/// stream (`stream-sink`) for its whole lifetime. Implements [`ChunkSink`]
/// so the host's push loop (`syneroym_chunk_transfer::push_until_eof`) can
/// drive it without knowing anything about Wasmtime.
pub struct GuestStreamSink {
    store: Store<HostState>,
    instance: Instance,
    resource: ResourceAny,
    max_instructions: Option<u64>,
}

impl Debug for GuestStreamSink {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestStreamSink").finish_non_exhaustive()
    }
}

impl GuestStreamSink {
    #[must_use]
    pub fn new(
        store: Store<HostState>,
        instance: Instance,
        resource: ResourceAny,
        max_instructions: Option<u64>,
    ) -> Self {
        Self { store, instance, resource, max_instructions }
    }
}

#[async_trait]
impl ChunkSink for GuestStreamSink {
    async fn push_chunk(&mut self, data: Vec<u8>) -> Result<()> {
        let results = call_resource_method(
            &mut self.store,
            &self.instance,
            self.max_instructions,
            self.resource,
            "[method]stream-sink.push-chunk",
            &[bytes_to_val_list(data)],
        )
        .await?;
        extract_result(&results[0], |_| Ok(()))
    }

    async fn finalize(self: Box<Self>) -> Result<()> {
        let mut this = *self;
        let results = call_resource_method(
            &mut this.store,
            &this.instance,
            this.max_instructions,
            this.resource,
            "[method]stream-sink.finalize",
            &[],
        )
        .await?;
        let result = extract_result(&results[0], |_| Ok(()));
        drop_resource_ignore_errors(&mut this.store, this.resource).await;
        result
    }

    async fn abort(self: Box<Self>) {
        let mut this = *self;
        drop_resource_ignore_errors(&mut this.store, this.resource).await;
    }
}

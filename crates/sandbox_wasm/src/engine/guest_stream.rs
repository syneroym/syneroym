use super::*;

/// How a `run_stream_protocol_request` call ended. Callers that branch
/// on whether the guest admitted the stream (like the SSE streaming-HTTP
/// bridge in `crates/router/src/route_handler/http.rs`, which maps
/// `Declined` to HTTP 403) can now do so; the raw-QUIC-stream caller
/// (`crates/router/src/route_handler/io.rs`) doesn't need the
/// distinction and ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRequestOutcome {
    /// The guest accepted the request and the stream ran to completion
    /// (or was aborted mid-transfer, in which case this function returns
    /// `Err` instead -- see `run_stream_protocol_request`).
    Completed,
    /// The guest declined the request (`Err` from
    /// `handle-stream-request`/`accept-stream-upload`) or doesn't export
    /// a handler for this protocol at all; the stream was closed cleanly
    /// with no bytes transferred.
    Declined,
}

impl AppSandboxEngine {
    /// Aborts every open stream task for `service_id` (called
    /// from `stop_wasm` and `ControlPlaneService::undeploy`, mirroring
    /// `unsubscribe_all`). `StreamRegistry`'s own `Drop` is the backstop for
    /// every other teardown path (ADR-0014).
    pub fn abort_streams(&self, service_id: &str) {
        self.stream_registry.abort_all(service_id);
    }

    /// Acquires a connection permit for an incoming WebSocket connection,
    /// bounded by the per-service semaphore.
    pub async fn acquire_websocket_permit(
        &self,
        service_id: &str,
        timeout: Duration,
    ) -> Option<OwnedSemaphorePermit> {
        let sem = self
            .guest_websocket_permits
            .entry(service_id.to_string())
            .or_insert_with(|| {
                Arc::new(Semaphore::new(self.max_concurrent_websockets_per_service as usize))
            })
            .clone();
        time::timeout(timeout, sem.acquire_owned()).await.ok().and_then(Result::ok)
    }

    /// Maximum SSE subscribers allowed per service from config.
    #[must_use]
    pub fn max_sse_subscribers_per_service(&self) -> usize {
        self.max_sse_subscribers_per_service as usize
    }

    /// Returns the shared `WebSocketSenders` table, initializing with a default
    /// instance if none was set by the composition root.
    pub fn websocket_senders(&self) -> Arc<syneroym_rpc::WebSocketSenders> {
        self.websocket_senders.get_or_init(WebSocketSenders::new).clone()
    }

    /// Registers a unicast sender channel for an active WebSocket connection,
    /// returning the receiver to drain in the router's connection loop.
    pub fn register_websocket_sender(&self, service_id: &str, conn_id: &str) -> WebSocketReceiver {
        self.websocket_senders().register(service_id, conn_id)
    }

    /// Removes a unicast sender channel for a closed WebSocket connection.
    pub fn deregister_websocket_sender(&self, service_id: &str, conn_id: &str) {
        if let Some(senders) = self.websocket_senders.get() {
            senders.deregister(service_id, conn_id);
        }
    }

    /// Drops all WebSocket senders for a service.
    /// Called from undeploy/stop. Dropping the senders unblocks any active
    /// rx.recv() loops in the router, terminating the WebSockets cleanly.
    pub fn forget_websocket_senders(&self, service_id: &str) {
        if let Some(senders) = self.websocket_senders.get() {
            senders.forget_service(service_id);
        }
        self.guest_websocket_permits.remove(service_id);
    }

    /// Opens a fresh, long-lived `Store`/`Instance` for one
    /// stream's lifetime (ADR-0014 "Instance Lifetime and Quota") --
    /// distinct from `build_store_and_instantiate`'s per-*call* instances,
    /// which don't outlive a single invocation. Also returns the resolved
    /// fuel budget, re-applied before every chunk call by
    /// `GuestStreamCursor`/`GuestStreamSink`.
    async fn open_stream_instance(
        &self,
        service_id: &str,
    ) -> Result<(Store<HostState>, Instance, Option<u64>)> {
        // `service_system`, never `local_elevated` -- same reasoning as
        // `deliver_message`: the component acts as itself, not as an admin.
        // `from_wire`: a raw stream is always peer-initiated router ingress.
        self.build_store_and_instantiate(
            service_id,
            CallerContext::service_system(service_id),
            self.dispatch_epoch_ticks,
            InstanceOptions::from_wire(),
        )
        .await
    }

    /// Entry point for a peer-initiated `raw://<protocol>|<service_id>`
    /// stream (`crates/router/src/route_handler/io.rs`'s
    /// `handle_raw_stream`, per ADR-0014). Spawns one dedicated Tokio task
    /// per stream (owning the long-lived `Store`/`Instance`) *before*
    /// reserving its slot in `StreamRegistry`, since the `AbortHandle` only
    /// exists once the task has been spawned; the reservation itself is a
    /// single atomic check-and-register (see `StreamRegistry::try_reserve`),
    /// so concurrent requests can't all observe spare capacity and all get
    /// admitted. If the reservation is refused, the just-spawned task is
    /// aborted immediately (it can't have made meaningful progress yet) and
    /// the caller sees a clean over-capacity error instead of the stream
    /// briefly starting anyway.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_stream_protocol_request(
        &self,
        service_id: &str,
        protocol: &str,
        peer_id: &str,
        direction: StreamDirection,
        initial_payload: Vec<u8>,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<StreamRequestOutcome> {
        let engine = self
            .self_weak
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| anyhow!("sandbox engine unavailable for stream handling"))?;

        let service_id_owned = service_id.to_string();
        let protocol_owned = protocol.to_string();
        let peer_id_owned = peer_id.to_string();
        let tracked_service_id = service_id.to_string();

        let join_handle = tokio::spawn(async move {
            engine
                .run_stream_protocol_request(
                    &service_id_owned,
                    &protocol_owned,
                    &peer_id_owned,
                    direction,
                    initial_payload,
                    reader,
                    writer,
                )
                .await
        });
        let abort_handle = join_handle.abort_handle();
        if let Err(e) = self.stream_registry.try_reserve(
            &tracked_service_id,
            self.max_concurrent_streams_per_service,
            abort_handle.clone(),
        ) {
            abort_handle.abort();
            return Err(e);
        }

        let result = join_handle.await;
        self.stream_registry.untrack(&tracked_service_id, &abort_handle);

        match result {
            Ok(inner) => inner,
            // Aborted by `stop_wasm`/`undeploy` -- not a real failure from
            // the stream's own perspective, the router already closed (or
            // is closing) the underlying QUIC stream in that case.
            Err(join_err) if join_err.is_cancelled() => Ok(StreamRequestOutcome::Completed),
            Err(join_err) => Err(anyhow!("stream task failed: {join_err}")),
        }
    }

    /// The actual per-stream work, run on its own dedicated Tokio task (see
    /// `handle_stream_protocol_request`): resolves the guest's
    /// `handle-stream-request`/`accept-stream-upload` export for `protocol`
    /// and, if it accepts, drives the pull/push loop until the stream ends.
    /// A guest that declines (`Err`) or doesn't export the relevant
    /// function closes the stream cleanly (`Ok(())`) rather than erroring --
    /// this is also the safety net for the `EndpointRegistry`-reuse caveat
    /// in ADR-0014 (a `raw://` request against a non-stream interface name
    /// simply finds no matching export).
    ///
    /// Acquires a `stream_instance_permits` permit *before* opening the
    /// stream's pooled component instance, and holds it for this function's
    /// whole lifetime (dropped on every exit path, including the early
    /// `return`s below) -- see that field's doc comment for why this
    /// engine-wide budget exists alongside the per-service
    /// `StreamRegistry` cap.
    #[allow(clippy::too_many_arguments)]
    async fn run_stream_protocol_request(
        &self,
        service_id: &str,
        protocol: &str,
        peer_id: &str,
        direction: StreamDirection,
        initial_payload: Vec<u8>,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<StreamRequestOutcome> {
        let _stream_instance_permit = self
            .stream_instance_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow!("stream instance semaphore closed: {e}"))?;

        let (mut store, instance, max_instructions) = self.open_stream_instance(service_id).await?;
        let mut writer = writer;

        let result = match direction {
            StreamDirection::Download => {
                let resource = match stream::call_handle_stream_request(
                    &mut store,
                    &instance,
                    protocol,
                    peer_id,
                    initial_payload,
                )
                .await
                {
                    Ok(resource) => resource,
                    Err(e) => {
                        debug!(
                            service_id,
                            protocol,
                            error = %e,
                            "stream: guest declined handle-stream-request (or does not export it)"
                        );
                        let _ = writer.shutdown().await;
                        return Ok(StreamRequestOutcome::Declined);
                    }
                };
                let cursor = GuestStreamCursor::new(
                    store,
                    instance,
                    resource,
                    max_instructions,
                    self.dispatch_epoch_ticks,
                );
                chunk_transfer::pull_until_eof(cursor, &mut writer).await
            }
            StreamDirection::Upload => {
                let resource = match stream::call_accept_stream_upload(
                    &mut store,
                    &instance,
                    protocol,
                    peer_id,
                    initial_payload,
                )
                .await
                {
                    Ok(resource) => resource,
                    Err(e) => {
                        debug!(
                            service_id,
                            protocol,
                            error = %e,
                            "stream: guest declined accept-stream-upload (or does not export it)"
                        );
                        let _ = writer.shutdown().await;
                        return Ok(StreamRequestOutcome::Declined);
                    }
                };
                let sink: Box<dyn ChunkSink> = Box::new(GuestStreamSink::new(
                    store,
                    instance,
                    resource,
                    max_instructions,
                    self.dispatch_epoch_ticks,
                ));
                chunk_transfer::push_until_eof(reader, sink).await
            }
        };

        // Neither `pull_until_eof` nor `push_until_eof` shuts `writer` down
        // (the latter doesn't touch it at all); without an explicit clean
        // close here, a peer reading this stream's other QUIC direction to
        // EOF has nothing to observe and hangs rather than completing.
        let _ = writer.shutdown().await;
        result.map(|()| StreamRequestOutcome::Completed)
    }

    pub async fn handle_websocket_on_open(
        &self,
        service_id: &str,
        conn_id: &str,
        caller: Option<CallerContext>,
    ) {
        let _active_guard = ActiveInstanceGuard::new();
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));
        let (mut store, instance, _) = match self
            .build_store_and_instantiate(
                service_id,
                caller,
                self.dispatch_epoch_ticks,
                InstanceOptions::from_wire(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-open failed to instantiate component");
                return;
            }
        };

        let (func, _, _) = match Self::get_wasm_func(
            &mut store,
            &instance,
            Some("syneroym:http/websocket-handler@0.1.0"),
            "on-open",
        ) {
            Ok(f) => f,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-open export not found");
                return;
            }
        };
        let conn_val = Val::String(conn_id.to_string());
        if let Err(e) = func.call_async(&mut store, &[conn_val], &mut []).await {
            warn!(service_id, error = %e, "WebSocket on-open invocation error");
        }
    }

    pub async fn handle_websocket_on_message(
        &self,
        service_id: &str,
        conn_id: &str,
        frame: Vec<u8>,
        kind: FrameKind,
        caller: Option<CallerContext>,
    ) {
        let _active_guard = ActiveInstanceGuard::new();
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));
        let (mut store, instance, _) = match self
            .build_store_and_instantiate(
                service_id,
                caller,
                self.dispatch_epoch_ticks,
                InstanceOptions::from_wire(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-message failed to instantiate component");
                return;
            }
        };

        let (func, _, _) = match Self::get_wasm_func(
            &mut store,
            &instance,
            Some("syneroym:http/websocket-handler@0.1.0"),
            "on-message",
        ) {
            Ok(f) => f,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-message export not found");
                return;
            }
        };
        let conn_val = Val::String(conn_id.to_string());
        let frame_val = stream::bytes_to_val_list(frame);
        let kind_val = match kind {
            FrameKind::Text => Val::Enum("text".to_string()),
            FrameKind::Binary => Val::Enum("binary".to_string()),
        };
        if let Err(e) = func.call_async(&mut store, &[conn_val, frame_val, kind_val], &mut []).await
        {
            warn!(service_id, error = %e, "WebSocket on-message invocation error");
        }
    }

    pub async fn handle_websocket_on_close(
        &self,
        service_id: &str,
        conn_id: &str,
        caller: Option<CallerContext>,
    ) {
        let _active_guard = ActiveInstanceGuard::new();
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));
        let (mut store, instance, _) = match self
            .build_store_and_instantiate(
                service_id,
                caller,
                self.dispatch_epoch_ticks,
                InstanceOptions::from_wire(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-close failed to instantiate component");
                return;
            }
        };

        let (func, _, _) = match Self::get_wasm_func(
            &mut store,
            &instance,
            Some("syneroym:http/websocket-handler@0.1.0"),
            "on-close",
        ) {
            Ok(f) => f,
            Err(e) => {
                warn!(service_id, error = %e, "WebSocket on-close export not found");
                return;
            }
        };
        let conn_val = Val::String(conn_id.to_string());
        if let Err(e) = func.call_async(&mut store, &[conn_val], &mut []).await {
            warn!(service_id, error = %e, "WebSocket on-close invocation error");
        }
    }
}

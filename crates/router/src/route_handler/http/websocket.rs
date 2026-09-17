use std::ops::ControlFlow;

use super::*;

pub(super) enum WsTarget {
    Native(Arc<dyn NativeHttpService>),
    Wasm(Arc<AppSandboxEngine>),
}

impl WsTarget {
    pub(super) async fn on_open(
        &self,
        service_id: &str,
        conn: &str,
        caller: Option<CallerContext>,
    ) {
        match self {
            WsTarget::Native(svc) => svc.on_websocket_open(conn.to_string(), caller).await,
            WsTarget::Wasm(engine) => {
                engine.handle_websocket_on_open(service_id, conn, caller).await;
            }
        }
    }

    pub(super) async fn on_message(
        &self,
        service_id: &str,
        conn: &str,
        frame: Vec<u8>,
        kind: FrameKind,
        caller: Option<CallerContext>,
    ) {
        match self {
            WsTarget::Native(svc) => {
                svc.on_websocket_message(conn.to_string(), frame, kind, caller).await;
            }
            WsTarget::Wasm(engine) => {
                engine.handle_websocket_on_message(service_id, conn, frame, kind, caller).await;
            }
        }
    }

    pub(super) async fn on_close(
        &self,
        service_id: &str,
        conn: &str,
        caller: Option<CallerContext>,
    ) {
        match self {
            WsTarget::Native(svc) => svc.on_websocket_close(conn.to_string(), caller).await,
            WsTarget::Wasm(engine) => {
                engine.handle_websocket_on_close(service_id, conn, caller).await;
            }
        }
    }
}

/// The upgraded connection's I/O type once `hyper::upgrade::on` hands it
/// over -- named so the split sink/stream types below don't repeat it.
type WsIo = TokioIo<Upgraded>;
type WsSink = stream::SplitSink<WebSocketStream<WsIo>, Message>;
type WsStream = stream::SplitStream<WebSocketStream<WsIo>>;

/// One connection's identity and cleanup handles, threaded through
/// `run_websocket_session`/`run_websocket_connection`/`run_ws_read_loop` as
/// a single value -- grouped so those functions stay under clippy's
/// argument-count lint instead of taking each field separately.
struct WsConnectionCtx {
    ws_target: WsTarget,
    service_id: String,
    conn_id: String,
    caller: Option<CallerContext>,
    senders_cleanup: Arc<WebSocketSenders>,
    cleanup_service_id: String,
}

impl HttpHandler {
    pub(super) fn validate_websocket_upgrade_headers(
        headers: &HeaderMap,
    ) -> Result<&str, &'static str> {
        let is_upgrade = headers
            .get(hyper::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        let is_conn_upgrade = headers
            .get(hyper::header::CONNECTION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"));
        let has_version_13 = headers
            .get("sec-websocket-version")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "13");
        let req_key =
            headers.get("sec-websocket-key").and_then(|v| v.to_str().ok()).unwrap_or_default();

        if !is_upgrade || !is_conn_upgrade || !has_version_13 || req_key.is_empty() {
            return Err("Invalid WebSocket upgrade request headers");
        }
        Ok(req_key)
    }

    pub(super) async fn handle_websocket_route(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if self.caller.is_none() && !route.public {
            return Ok(http_error(StatusCode::UNAUTHORIZED, "Unauthorized".into()));
        }

        if route.operation != "handle-upgrade" {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported websocket operation: {}", route.operation),
            ));
        }

        let (ws_target, ws_service_id) = match self.resolve_ws_target() {
            ControlFlow::Break(resp) => return Ok(resp),
            ControlFlow::Continue(target) => target,
        };

        let req_key = match Self::validate_websocket_upgrade_headers(req.headers()) {
            Ok(k) => k,
            Err(msg) => {
                return Ok(http_error(StatusCode::BAD_REQUEST, msg.into()));
            }
        };

        let accept_key = derive_accept_key(req_key.as_bytes());
        let response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::CONNECTION, "upgrade")
            .header("Sec-WebSocket-Accept", accept_key)
            .body(full_body(Bytes::new()));

        let response = match response {
            Ok(r) => r,
            Err(_) => {
                return Ok(http_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to build response".into(),
                ));
            }
        };

        let service_id = self.preamble.service_id.clone();
        let (topic_rx, sub_handle) = match self.subscribe_ws_topic(route, &service_id).await {
            ControlFlow::Break(resp) => return Ok(resp),
            ControlFlow::Continue(subscribed) => subscribed,
        };

        let permit = match Self::acquire_ws_permit(&ws_target, &service_id).await {
            ControlFlow::Break(resp) => return Ok(resp),
            ControlFlow::Continue(permit) => permit,
        };

        let conn_id = uuid::Uuid::new_v4().to_string();
        let rx_internal =
            self.route_handler.inner.websocket_senders.register(&ws_service_id, &conn_id);
        let senders_cleanup = self.route_handler.inner.websocket_senders.clone();
        let cleanup_service_id = ws_service_id.clone();
        let caller = self.caller.clone();

        let ctx = WsConnectionCtx {
            ws_target,
            service_id,
            conn_id,
            caller,
            senders_cleanup,
            cleanup_service_id,
        };
        tokio::task::spawn(Self::run_websocket_session(
            req,
            ctx,
            permit,
            sub_handle,
            topic_rx,
            rx_internal,
        ));

        Ok(response)
    }

    /// Picks the WebSocket target the same way `resolve_guest_engine`
    /// (the `guest` route's analogue) does: a natively linked service
    /// shadows a deployed WASM component (only logged, never used), and
    /// otherwise a deployed component must exist. `Break` carries the
    /// response to return immediately -- no sandbox engine at all
    /// (coordinator mode), or no component deployed for this service --
    /// the same as the early `return Ok(...)` this replaces.
    fn resolve_ws_target(&self) -> ControlFlow<Response<HttpBody>, (WsTarget, String)> {
        let native = self
            .route_handler
            .inner
            .native_http
            .get(&self.preamble.service_id)
            .map(|e| e.value().clone());

        if let Some(svc) = native {
            if let Some(engine) = &self.route_handler.inner.app_sandbox_engine
                && engine.is_deployed(&self.preamble.service_id)
            {
                warn!(
                    service_id = %self.preamble.service_id,
                    "native_http service shadows deployed WASM component"
                );
            }
            let ws_id = svc.service_id().unwrap_or(&self.preamble.service_id).to_string();
            return ControlFlow::Continue((WsTarget::Native(svc), ws_id));
        }
        let Some(engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
            return ControlFlow::Break(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "app sandbox engine not available (coordinator mode)".into(),
            ));
        };
        if !engine.is_deployed(&self.preamble.service_id) {
            return ControlFlow::Break(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "service has no deployed WASM component".into(),
            ));
        }
        ControlFlow::Continue((WsTarget::Wasm(engine), self.preamble.service_id.clone()))
    }

    /// Subscribes to `route.topic`'s broker topic when the route declares
    /// one. `Break` carries the response to return immediately when the
    /// subscribe call itself fails, the same as the early `return Ok(...)`
    /// this replaces. `Continue` carries `(None, None)` for a route with no
    /// topic, unchanged from the early `let mut topic_rx = None;` this
    /// replaces.
    async fn subscribe_ws_topic(
        &self,
        route: &HttpRoute,
        service_id: &str,
    ) -> ControlFlow<
        Response<HttpBody>,
        (Option<mpsc::Receiver<(String, Vec<u8>)>>, Option<SubscriptionHandle>),
    > {
        let Some(topic) = &route.topic else {
            return ControlFlow::Continue((None, None));
        };
        let namespaced = namespace_topic(service_id, topic);
        match self.route_handler.inner.messaging_broker.subscribe(namespaced).await {
            Ok((handle, rx_broadcast)) => ControlFlow::Continue((Some(rx_broadcast), Some(handle))),
            Err(e) => ControlFlow::Break(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to subscribe: {e}"),
            )),
        }
    }

    /// Acquires this connection's concurrency-limit permit -- only the
    /// `Wasm` target has a limit to enforce; a natively linked service has
    /// none. `Break` carries the `503` + `Retry-After` response to return
    /// immediately when the limit is reached, the same as the early
    /// `return Ok(resp)` this replaces.
    async fn acquire_ws_permit(
        ws_target: &WsTarget,
        service_id: &str,
    ) -> ControlFlow<Response<HttpBody>, Option<OwnedSemaphorePermit>> {
        match ws_target {
            WsTarget::Wasm(engine) => {
                match engine.acquire_websocket_permit(service_id, Duration::from_secs(2)).await {
                    Some(p) => ControlFlow::Continue(Some(p)),
                    None => {
                        let mut resp = http_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "websocket concurrency limit reached".into(),
                        );
                        resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                        ControlFlow::Break(resp)
                    }
                }
            }
            WsTarget::Native(_) => ControlFlow::Continue(None),
        }
    }

    /// Completes the HTTP-to-WebSocket upgrade and hands off to
    /// `run_websocket_connection`, or logs and cleans up the connection's
    /// registered sender if the upgrade itself never lands. `permit` and
    /// `sub_handle` are held for this whole session's lifetime (an RAII
    /// concurrency slot and a topic subscription respectively) -- they own
    /// nothing this function reads, only something it must keep alive.
    async fn run_websocket_session(
        req: Request<Incoming>,
        ctx: WsConnectionCtx,
        permit: Option<OwnedSemaphorePermit>,
        sub_handle: Option<SubscriptionHandle>,
        topic_rx: Option<mpsc::Receiver<(String, Vec<u8>)>>,
        rx_internal: WebSocketReceiver,
    ) {
        let _keep_alive = sub_handle;
        let _permit = permit;
        match upgrade::on(req).await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                Self::run_websocket_connection(io, ctx, topic_rx, rx_internal).await;
            }
            Err(e) => {
                error!("WebSocket upgrade error: {}", e);
                ctx.senders_cleanup.deregister(&ctx.cleanup_service_id, &ctx.conn_id);
            }
        }
    }

    /// Runs one upgraded WebSocket connection end to end: builds the frame
    /// codec, starts the outbound writer task, dispatches `on_open`, drains
    /// inbound frames until the connection or the writer side ends, then
    /// tears down in the same order the inline version did -- shut the
    /// writer down, join it, deregister the sender, dispatch `on_close`.
    async fn run_websocket_connection(
        io: WsIo,
        ctx: WsConnectionCtx,
        topic_rx: Option<mpsc::Receiver<(String, Vec<u8>)>>,
        rx_internal: WebSocketReceiver,
    ) {
        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(1024 * 1024);
        ws_config.max_frame_size = Some(1024 * 1024);

        let ws_stream = WebSocketStream::from_raw_socket(io, Role::Server, Some(ws_config)).await;
        let (ws_sink, ws_stream) = ws_stream.split();

        let (session_stop_tx, session_stop_rx) = oneshot::channel::<()>();
        let (writer_task, writer_shutdown_tx) =
            Self::spawn_ws_writer_task(ws_sink, topic_rx, rx_internal, session_stop_tx);

        // Sequential dispatch: await on-open before frame loop
        ctx.ws_target.on_open(&ctx.service_id, &ctx.conn_id, ctx.caller.clone()).await;

        Self::run_ws_read_loop(
            ws_stream,
            session_stop_rx,
            &ctx.ws_target,
            &ctx.service_id,
            &ctx.conn_id,
            ctx.caller.clone(),
        )
        .await;

        let _ = writer_shutdown_tx.send(());
        let _ = writer_task.await;
        ctx.senders_cleanup.deregister(&ctx.cleanup_service_id, &ctx.conn_id);
        ctx.ws_target.on_close(&ctx.service_id, &ctx.conn_id, ctx.caller).await;
    }

    /// The outbound half of a WebSocket session: relays both a subscribed
    /// broker topic (if any) and this connection's unicast sender channel
    /// onto the socket, until told to shut down, the connection drops, or
    /// either source dries up. Signals `session_stop_tx` on its own way
    /// out so the inbound read loop stops too. Returns the task's handle
    /// alongside the shutdown sender the caller uses to ask it to stop.
    fn spawn_ws_writer_task(
        mut ws_sink: WsSink,
        mut topic_rx: Option<mpsc::Receiver<(String, Vec<u8>)>>,
        mut rx_internal: WebSocketReceiver,
        session_stop_tx: oneshot::Sender<()>,
    ) -> (JoinHandle<()>, oneshot::Sender<()>) {
        let (writer_shutdown_tx, mut writer_shutdown_rx) = oneshot::channel::<()>();
        let writer_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut writer_shutdown_rx => break,
                    broadcast = async {
                        if let Some(rx) = &mut topic_rx {
                            rx.recv().await
                        } else {
                            futures::future::pending().await
                        }
                    } => {
                        match broadcast {
                            Some((_, payload)) => {
                                let msg = if let Ok(text) = String::from_utf8(payload.clone()) {
                                    Message::Text(text.into())
                                } else {
                                    Message::Binary(payload.into())
                                };
                                if ws_sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    internal_msg = rx_internal.recv() => {
                        match internal_msg {
                            Some((frame, kind)) => {
                                let msg = match kind {
                                    FrameKind::Text => {
                                        let text = String::from_utf8_lossy(&frame).to_string();
                                        Message::Text(text.into())
                                    }
                                    FrameKind::Binary => Message::Binary(frame.into()),
                                };
                                if ws_sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
            let _ = session_stop_tx.send(());
        });
        (writer_task, writer_shutdown_tx)
    }

    /// The inbound half of a WebSocket session: dispatches each text/binary
    /// frame to `ws_target` until the writer side signals `session_stop_rx`,
    /// the client sends `Close`, the stream ends, or a stream error occurs.
    /// `ws_stream` is owned, not borrowed, so it drops -- releasing the
    /// read half -- the moment this returns, at the same point the inline
    /// version's explicit `drop(ws_stream)` did.
    async fn run_ws_read_loop(
        mut ws_stream: WsStream,
        mut session_stop_rx: oneshot::Receiver<()>,
        ws_target: &WsTarget,
        service_id: &str,
        conn_id: &str,
        caller: Option<CallerContext>,
    ) {
        loop {
            tokio::select! {
                _ = &mut session_stop_rx => break,
                msg_opt = ws_stream.next() => {
                    let Some(msg_res) = msg_opt else { break; };
                    match msg_res {
                        Ok(Message::Text(txt)) => {
                            ws_target
                                .on_message(
                                    service_id,
                                    conn_id,
                                    txt.as_bytes().to_vec(),
                                    FrameKind::Text,
                                    caller.clone(),
                                )
                                .await;
                        }
                        Ok(Message::Binary(bin)) => {
                            ws_target
                                .on_message(
                                    service_id,
                                    conn_id,
                                    bin.to_vec(),
                                    FrameKind::Binary,
                                    caller.clone(),
                                )
                                .await;
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(Message::Ping(_)) => {}
                        Ok(Message::Pong(_)) => {}
                        Ok(Message::Frame(_)) => {}
                        Err(e) => {
                            debug!(service_id, conn_id, error = %e, "WebSocket stream error");
                            break;
                        }
                    }
                }
            }
        }
    }
}

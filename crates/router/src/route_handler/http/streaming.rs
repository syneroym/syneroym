use super::*;

pub(super) enum WsTarget {
    Native(Arc<dyn syneroym_rpc::NativeHttpService>),
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

impl HttpHandler {
    /// `req` carries no body worth reading for a `GET`+SSE subscription;
    /// kept as a parameter for symmetry with the other route handlers
    /// (only the `Accept` header is inspected).
    pub(super) async fn handle_messaging_sse(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let accepts_sse = req
            .headers()
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        if !accepts_sse {
            return Ok(http_error(
                StatusCode::NOT_ACCEPTABLE,
                "Accept: text/event-stream is required for SSE subscription routes".into(),
            ));
        }

        let service_id = self.preamble.service_id.clone();
        let sse_service_id = self.preamble.service_id.clone();
        let max_subscribers = self
            .route_handler
            .inner
            .app_sandbox_engine
            .as_ref()
            .map(|e| e.max_sse_subscribers_per_service())
            .unwrap_or(DEFAULT_MAX_SSE_SUBSCRIBERS_PER_SERVICE);
        let permits = {
            let entry = self
                .route_handler
                .inner
                .sse_permits
                .entry(service_id)
                .or_insert_with(|| Arc::new(Semaphore::new(max_subscribers)));
            entry.value().clone()
        };
        let permit = match permits.try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service is at its SSE subscriber concurrency limit".into(),
                );
                resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                return Ok(resp);
            }
        };

        let topic = route.topic.clone().unwrap_or_default();
        let namespaced = namespace_topic(&self.preamble.service_id, &topic);
        let (handle, receiver) = self
            .route_handler
            .inner
            .messaging_broker
            .subscribe(namespaced)
            .await
            .map_err(|e| anyhow!("SSE subscribe failed: {e}"))?;

        // Pull-based: each poll awaits the next broker message and formats
        // it as one SSE frame. `handle` (the `SubscriptionHandle`) and
        // `permit` are carried inside the stream's own state, so they -- and
        // the broker subscription they own -- are dropped the moment hyper
        // stops driving this response body, which is exactly what happens
        // when the client disconnects.
        let stream = stream::unfold(
            (receiver, handle, permit, sse_service_id),
            |(mut receiver, handle, permit, sid)| async move {
                let (topic, payload) = receiver.recv().await?;
                let name = service_relative_topic(&sid, &topic);
                let frame = Frame::data(Bytes::from(format_sse_frame(name, &payload)));
                Some((Ok::<_, Infallible>(frame), (receiver, handle, permit, sid)))
            },
        );

        let body = StreamBody::new(stream).boxed_unsync();
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| anyhow!("failed to build SSE response: {e}"))
    }

    // -- stream / chunked upload and download ----------------------------

    pub(super) async fn handle_stream_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let Some(app_sandbox_engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "app sandbox engine not available (coordinator mode)".into(),
            ));
        };
        let Some(protocol) = route.protocol.clone() else {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "http_routes entry missing `protocol` for a stream route".into(),
            ));
        };
        // Mirrors `io.rs::handle_stream_protocol_request`'s
        // `UNKNOWN_PEER_ID` fallback for the raw-QUIC path -- an HTTP
        // caller carries the same optional `delegation` on its preamble.
        let peer_id = self
            .preamble
            .delegation
            .as_ref()
            .map(|d| d.master_did.clone())
            .unwrap_or_else(|| "unknown-peer".to_string());

        // `initial_payload` doubles as the guest's `metadata` parameter
        // (`accept-stream-upload(protocol, peer-id, metadata)`) or the
        // download request parameter (`handle-stream-request(protocol, peer-id,
        // request-data)`).
        let initial_payload = path_param
            .as_ref()
            .map(|p| percent_encoding::percent_decode_str(p).collect::<Vec<u8>>())
            .unwrap_or_else(|| {
                req.uri()
                    .query()
                    .and_then(|q| parse_query(q).remove("metadata"))
                    .map(String::into_bytes)
                    .unwrap_or_default()
            });

        match route.operation.as_str() {
            "accept-upload" => {
                let body_stream = req.into_body().into_data_stream().map_err(io::Error::other);
                let reader: Box<dyn AsyncRead + Unpin + Send> =
                    Box::new(StreamReader::new(body_stream));
                let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(tokio_io::sink());

                match app_sandbox_engine
                    .handle_stream_protocol_request(
                        &self.preamble.service_id,
                        &protocol,
                        &peer_id,
                        StreamDirection::Upload,
                        initial_payload,
                        reader,
                        writer,
                    )
                    .await
                {
                    Ok(StreamRequestOutcome::Completed) => Ok(json_response(
                        StatusCode::OK,
                        &serde_json::json!({"status": "uploaded"}),
                    )),
                    Ok(StreamRequestOutcome::Declined) => {
                        Ok(http_error(StatusCode::FORBIDDEN, "upload declined by guest".into()))
                    }
                    Err(e) => {
                        error!(
                            service_id = %self.preamble.service_id,
                            protocol = %protocol,
                            error = %e,
                            "accept-upload failed"
                        );
                        Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
                    }
                }
            }
            "accept-download" => {
                let (duplex_writer, mut duplex_reader) = tokio_io::duplex(64 * 1024);
                let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(tokio_io::empty());
                let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(duplex_writer);

                let service_id = self.preamble.service_id.clone();
                let engine = app_sandbox_engine.clone();
                let proto = protocol.clone();
                let peer = peer_id.clone();

                let join_handle = tokio::spawn(async move {
                    engine
                        .handle_stream_protocol_request(
                            &service_id,
                            &proto,
                            &peer,
                            StreamDirection::Download,
                            initial_payload,
                            reader,
                            writer,
                        )
                        .await
                });

                let mut first_buf = vec![0u8; 64 * 1024];
                let first_chunk = match duplex_reader.read(&mut first_buf).await {
                    Ok(0) => match join_handle.await {
                        Ok(Ok(StreamRequestOutcome::Declined)) => {
                            return Ok(http_error(
                                StatusCode::NOT_FOUND,
                                "stream download declined or file not found".into(),
                            ));
                        }
                        Ok(Err(e)) => {
                            return Ok(http_error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                e.to_string(),
                            ));
                        }
                        _ => {
                            return Ok(http_error(
                                StatusCode::NOT_FOUND,
                                "stream download produced no data".into(),
                            ));
                        }
                    },
                    Ok(n) => {
                        first_buf.truncate(n);
                        Some(Bytes::from(first_buf))
                    }
                    Err(e) => {
                        return Ok(http_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to read stream download: {e}"),
                        ));
                    }
                };

                let stream = stream::unfold(
                    (duplex_reader, first_chunk, Some(join_handle)),
                    |(mut reader, first, mut handle)| async move {
                        if let Some(chunk) = first {
                            return Some((
                                Ok::<_, Infallible>(Frame::data(chunk)),
                                (reader, None, handle),
                            ));
                        }
                        let mut buf = vec![0u8; 64 * 1024];
                        match reader.read(&mut buf).await {
                            Ok(0) => {
                                if let Some(h) = handle.take() {
                                    match h.await {
                                        Ok(Ok(StreamRequestOutcome::Completed)) => {}
                                        Ok(Ok(StreamRequestOutcome::Declined)) => {
                                            warn!(
                                                "stream download declined by guest after partial \
                                                 transfer"
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            error!(
                                                "stream download task failed after partial \
                                                 transfer: {e}"
                                            );
                                        }
                                        Err(e) => {
                                            error!("stream download task panicked: {e}");
                                        }
                                    }
                                }
                                None
                            }
                            Ok(n) => {
                                buf.truncate(n);
                                Some((
                                    Ok::<_, Infallible>(Frame::data(Bytes::from(buf))),
                                    (reader, None, handle),
                                ))
                            }
                            Err(e) => {
                                error!("stream download read error: {e}");
                                None
                            }
                        }
                    },
                );

                let body = StreamBody::new(stream).boxed_unsync();
                let mut resp_builder = Response::builder().status(StatusCode::OK);
                if let Some(filename) = path_param.as_deref() {
                    let mime = mime_guess::from_path(filename).first_or_octet_stream();
                    resp_builder = resp_builder.header(CONTENT_TYPE, mime.as_ref());
                } else {
                    resp_builder = resp_builder.header(CONTENT_TYPE, "application/octet-stream");
                }
                resp_builder
                    .body(body)
                    .map_err(|e| anyhow!("failed to build download response: {e}"))
            }
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported stream operation: {other}"),
            )),
        }
    }

    // -- guest HTTP route target -------------------------------------------

    /// The fourth `dispatch_route` target: hands the request to the
    /// deployed component's `syneroym:http/incoming-handler#handle-request`
    /// export and turns its answer into an HTTP response. Reaches the guest
    /// directly through `app_sandbox_engine`, mirroring
    /// `handle_stream_route` -- an `http-native` connection resolves to a
    /// `NativeService` pipeline, so `dispatch_json_rpc_once` can never
    /// reach a guest, unlike `data-layer`/`messaging` above.
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

        let native = self
            .route_handler
            .inner
            .native_http
            .get(&self.preamble.service_id)
            .map(|e| e.value().clone());

        let (ws_target, ws_service_id) = if let Some(svc) = native {
            if let Some(engine) = &self.route_handler.inner.app_sandbox_engine
                && engine.is_deployed(&self.preamble.service_id)
            {
                warn!(
                    service_id = %self.preamble.service_id,
                    "native_http service shadows deployed WASM component"
                );
            }
            let ws_id = svc.service_id().unwrap_or(&self.preamble.service_id).to_string();
            (WsTarget::Native(svc), ws_id)
        } else {
            let Some(engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
                return Ok(http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "app sandbox engine not available (coordinator mode)".into(),
                ));
            };
            if !engine.is_deployed(&self.preamble.service_id) {
                return Ok(http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service has no deployed WASM component".into(),
                ));
            }
            (WsTarget::Wasm(engine), self.preamble.service_id.clone())
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
        let mut topic_rx = None;
        let mut _sub_handle = None;
        if let Some(topic) = &route.topic {
            let namespaced = syneroym_mqtt_broker::namespace_topic(&service_id, topic);
            let (handle, rx_broadcast) =
                match self.route_handler.inner.messaging_broker.subscribe(namespaced).await {
                    Ok(res) => res,
                    Err(e) => {
                        return Ok(http_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to subscribe: {e}"),
                        ));
                    }
                };
            topic_rx = Some(rx_broadcast);
            _sub_handle = Some(handle);
        }

        let permit = match &ws_target {
            WsTarget::Wasm(engine) => {
                match engine.acquire_websocket_permit(&service_id, Duration::from_secs(2)).await {
                    Some(p) => Some(p),
                    None => {
                        let mut resp = http_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "websocket concurrency limit reached".into(),
                        );
                        resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                        return Ok(resp);
                    }
                }
            }
            WsTarget::Native(_) => None,
        };

        let conn_id = uuid::Uuid::new_v4().to_string();
        let mut rx_internal =
            self.route_handler.inner.websocket_senders.register(&ws_service_id, &conn_id);
        let senders_cleanup = self.route_handler.inner.websocket_senders.clone();
        let cleanup_service_id = ws_service_id.clone();
        let caller = self.caller.clone();

        tokio::task::spawn(async move {
            let _keep_alive = _sub_handle;
            let _permit = permit;
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let io = hyper_util::rt::TokioIo::new(upgraded);
                    let mut ws_config = WebSocketConfig::default();
                    ws_config.max_message_size = Some(1024 * 1024);
                    ws_config.max_frame_size = Some(1024 * 1024);

                    let ws_stream =
                        WebSocketStream::from_raw_socket(io, Role::Server, Some(ws_config)).await;

                    use futures::{SinkExt, StreamExt};
                    let (mut ws_sink, mut ws_stream) = ws_stream.split();

                    let (writer_shutdown_tx, mut writer_shutdown_rx) = oneshot::channel::<()>();
                    let (session_stop_tx, mut session_stop_rx) = oneshot::channel::<()>();
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

                    // Sequential dispatch: await on-open before frame loop
                    ws_target.on_open(&service_id, &conn_id, caller.clone()).await;

                    loop {
                        tokio::select! {
                            _ = &mut session_stop_rx => break,
                            msg_opt = ws_stream.next() => {
                                let Some(msg_res) = msg_opt else { break; };
                                match msg_res {
                                    Ok(Message::Text(txt)) => {
                                        ws_target
                                            .on_message(
                                                &service_id,
                                                &conn_id,
                                                txt.as_bytes().to_vec(),
                                                FrameKind::Text,
                                                caller.clone(),
                                            )
                                            .await;
                                    }
                                    Ok(Message::Binary(bin)) => {
                                        ws_target
                                            .on_message(
                                                &service_id,
                                                &conn_id,
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

                    drop(ws_stream);
                    let _ = writer_shutdown_tx.send(());
                    let _ = writer_task.await;
                    senders_cleanup.deregister(&cleanup_service_id, &conn_id);
                    ws_target.on_close(&service_id, &conn_id, caller).await;
                }
                Err(e) => {
                    error!("WebSocket upgrade error: {}", e);
                    senders_cleanup.deregister(&cleanup_service_id, &conn_id);
                }
            }
        });

        Ok(response)
    }

    // -- signed-URL blob GET ---------------------------------------------
}

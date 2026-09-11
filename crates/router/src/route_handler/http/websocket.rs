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
}

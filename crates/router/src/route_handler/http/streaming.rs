use super::*;

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

    // -- signed-URL blob GET ---------------------------------------------
}

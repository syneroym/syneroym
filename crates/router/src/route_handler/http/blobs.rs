use super::*;

impl HttpHandler {
    pub(super) fn resolve_asset(&self, path: &str) -> Option<AssetEntry> {
        let service_assets = self.route_handler.inner.assets.get(&self.preamble.service_id)?;
        if !service_assets.public {
            return None;
        }
        let decoded = percent_encoding::percent_decode_str(path).decode_utf8().ok()?;
        let lookup_path = match decoded.strip_suffix('/') {
            Some(prefix) => format!("{prefix}/index.html"),
            None => decoded.into_owned(),
        };
        service_assets.manifest.entries.get(&lookup_path).cloned()
    }

    /// Serves one static asset. `Ok(None)` means "not an asset"
    /// and the caller falls through to route resolution unchanged.
    pub(super) async fn try_handle_asset(
        &self,
        method: &Method,
        path: &str,
        req: &Request<Incoming>,
    ) -> Result<Option<Response<HttpBody>>> {
        if *method != Method::GET && *method != Method::HEAD {
            return Ok(None);
        }
        let Some(entry) = self.resolve_asset(path) else {
            return Ok(None);
        };

        let etag = format!("\"{}\"", entry.hash);
        let cache_control = cache_control_for(&entry.content_type);
        if if_none_match_hits(req.headers().get(IF_NONE_MATCH), &etag) {
            let resp = Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(ETAG, etag)
                .header(CACHE_CONTROL, cache_control)
                .body(full_body(Bytes::new()))
                .map_err(|e| anyhow!("failed to build 304 response: {e}"))?;
            return Ok(Some(resp));
        }

        // `mime_guess` (`crates/control_plane/src/assets.rs`) falls back to
        // `application/octet-stream` for an unrecognised extension --
        // `nosniff` stops a browser from content-sniffing that into
        // something it will execute or render unexpectedly. `text/html`
        // additionally needs an explicit charset: with none, encoding falls
        // back to the browser's default, which mangles non-ASCII pages.
        let content_type = if entry.content_type == "text/html" {
            "text/html; charset=utf-8".to_string()
        } else {
            entry.content_type.clone()
        };
        // `entry.len` is promised here, before the body has streamed a
        // single byte -- a mid-stream `read-chunk` error in
        // `blob_download_step` below ends the body short of this declared
        // length instead of surfacing as a clean error, so the client sees
        // an aborted connection rather than a graceful failure. Pre-existing
        // on the signed-URL blob-download path too, just not previously
        // paired with a `Content-Length` for the mismatch to be assertable
        // against.
        let builder = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_LENGTH, entry.len.to_string())
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control)
            .header(X_CONTENT_TYPE_OPTIONS, "nosniff");

        if *method == Method::HEAD {
            let resp = builder
                .body(full_body(Bytes::new()))
                .map_err(|e| anyhow!("failed to build HEAD response: {e}"))?;
            return Ok(Some(resp));
        }

        // Never instantiates the component -- the same
        // `blob-store/open-download`+`read-chunk` native-dispatch streaming
        // `handle_blob_get` uses, reached through the identical
        // `NativeService` arm. Deliberately bypasses `self.dispatch()`
        // (bound to `self.caller`, which may be `None`): a public asset's
        // authorization is its declared `visibility`, already checked in
        // `resolve_asset`, not the connection's own delegation.
        let system_caller = CallerContext::service_system(&self.preamble.service_id);
        let open_params = serde_json::json!({"hash": entry.hash, "offset": 0});
        let download_id = match dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            Some(&system_caller),
            "blob-store",
            "open-download",
            open_params,
        )
        .await?
        {
            DispatchOutcome::Success(value) => {
                let resp: OpenDownloadResponse = serde_json::from_value(value)
                    .map_err(|e| anyhow!("malformed open-download response: {e}"))?;
                resp.download_id
            }
            DispatchOutcome::Error { code, message } => {
                return Ok(Some(structured_rpc_error(
                    status_for_rpc_error_code(code),
                    code,
                    message,
                )));
            }
        };

        let state = BlobDownloadState {
            route_handler: self.route_handler.clone(),
            pipeline: self.pipeline.clone(),
            preamble: RoutePreamble {
                interface: "blob-store".to_string(),
                ..self.preamble.clone()
            },
            caller: system_caller,
            download_id,
            closed: false,
        };
        let stream = stream::unfold(state, blob_download_step);
        let body = StreamBody::new(stream).boxed_unsync();
        let resp =
            builder.body(body).map_err(|e| anyhow!("failed to build asset response: {e}"))?;
        Ok(Some(resp))
    }

    pub(super) async fn handle_blob_get(
        &self,
        hash: &str,
        query: &str,
    ) -> Result<Response<HttpBody>> {
        let params = parse_query(query);
        let Some(svc) = params.get("svc") else {
            return Ok(http_error(StatusCode::BAD_REQUEST, "missing svc query parameter".into()));
        };
        // Decision 6: `svc` must equal the connection's own
        // `preamble.service_id` -- self-authorizing via the HMAC alone
        // doesn't extend to letting one connection serve another
        // service's blobs.
        if svc != &self.preamble.service_id {
            return Ok(http_error(
                StatusCode::FORBIDDEN,
                "svc query parameter must match the connected service".into(),
            ));
        }
        let Some(exp) = params.get("exp").and_then(|v| v.parse::<u64>().ok()) else {
            return Ok(http_error(
                StatusCode::BAD_REQUEST,
                "missing or invalid exp query parameter".into(),
            ));
        };
        let Some(sig) = params.get("sig") else {
            return Ok(http_error(StatusCode::BAD_REQUEST, "missing sig query parameter".into()));
        };

        let (Some(key_store), Some(storage_provider)) =
            (&self.route_handler.inner.key_store, &self.route_handler.inner.storage_provider)
        else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "blob serving is not available in this mode".into(),
            ));
        };
        let dek = storage_provider
            .load_service_dek(&self.preamble.service_id, key_store)
            .await
            .map_err(|e| anyhow!("failed to resolve service DEK: {e}"))?
            .unwrap_or_default();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        if let Err(e) =
            crypto::verify_signed_url(&dek, &self.preamble.service_id, hash, exp, sig, now)
        {
            return Ok(http_error(
                StatusCode::FORBIDDEN,
                format!("invalid or expired signed URL: {e}"),
            ));
        }

        // TODO(FDAE): the signed-URL HMAC is the interim authorization for
        // blob GET. Final policy (who may fetch which blob) is enforced by
        // a full FDAE policy against the resolved caller; `service_system`
        // is an interim system identity.
        //
        // This bypasses `self.dispatch()` (bound to `self.caller`, which may
        // be `None` for an anonymous signed-URL request) deliberately -- the
        // HMAC verified above is this route's authorization, not the
        // connection's delegation.
        let system_caller = CallerContext::service_system(&self.preamble.service_id);
        let open_params = serde_json::json!({"hash": hash, "offset": 0});
        let download_id = match dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            Some(&system_caller),
            "blob-store",
            "open-download",
            open_params,
        )
        .await?
        {
            DispatchOutcome::Success(value) => {
                let resp: OpenDownloadResponse = serde_json::from_value(value)
                    .map_err(|e| anyhow!("malformed open-download response: {e}"))?;
                resp.download_id
            }
            DispatchOutcome::Error { code, message } => {
                return Ok(structured_rpc_error(status_for_rpc_error_code(code), code, message));
            }
        };

        let state = BlobDownloadState {
            route_handler: self.route_handler.clone(),
            pipeline: self.pipeline.clone(),
            preamble: RoutePreamble {
                interface: "blob-store".to_string(),
                ..self.preamble.clone()
            },
            caller: system_caller,
            download_id,
            closed: false,
        };
        let stream = stream::unfold(state, blob_download_step);
        let body = StreamBody::new(stream).boxed_unsync();
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .map_err(|e| anyhow!("failed to build blob response: {e}"))
    }
}

/// State carried across `blob_download_step`'s `stream::unfold` iterations:
/// everything needed to make another `blob-store/read-chunk` native-dispatch
/// call. No `blob_provider`/DEK access here -- streaming reuses the
/// existing `open-download`/`read-chunk` methods (which resolve the DEK
/// internally per call, same as every other native-dispatch blob-store
/// method).
pub(super) struct BlobDownloadState {
    route_handler: RouteHandler,
    pipeline: RoutePipeline,
    preamble: RoutePreamble,
    /// The `service_system` caller established in `handle_blob_get` -- reused
    /// here rather than `None`/`self.caller` so the per-chunk and cleanup
    /// dispatches stay self-authorizing regardless of the original
    /// connection's delegation.
    caller: CallerContext,
    download_id: String,
    /// Set once the server side is known to have already released
    /// `download_id` on its own (the EOF path in `dispatch_blob_store`'s
    /// `read-chunk` arm doesn't reinsert the session) -- `Drop` only issues
    /// a `close-download` cleanup call when this is still `false`, so a
    /// normally-completed download doesn't pay for a redundant round trip.
    closed: bool,
}

impl Drop for BlobDownloadState {
    /// An HTTP client that disconnects before the body reaches EOF (a
    /// routine tab close, a client timeout, or simply not reading the full
    /// response) makes hyper drop this state without polling
    /// `blob_download_step` again -- with no other cancellation signal,
    /// the server-side `download_sessions` entry would otherwise leak
    /// until process restart. Fires a best-effort, fire-and-forget
    /// `close-download` in that case (mirrors `abort-upload`'s cleanup for
    /// the symmetric upload-side case).
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let route_handler = self.route_handler.clone();
        let pipeline = self.pipeline.clone();
        let preamble = self.preamble.clone();
        let caller = self.caller.clone();
        let download_id = self.download_id.clone();
        tokio::spawn(async move {
            let _ = dispatch_native(
                &route_handler,
                &pipeline,
                &preamble,
                Some(&caller),
                "blob-store",
                "close-download",
                serde_json::json!({"download_id": download_id}),
            )
            .await;
        });
    }
}

/// Pull-based blob `GET` body: `stream::unfold` naturally drives "read next
/// chunk" lazily as the HTTP body is polled. A read-chunk error mid-stream
/// has no HTTP-status channel left to use (headers are already sent, and
/// chunked transfer-encoding has no structured mid-body error frame) --
/// ending the stream cleanly here is the same "peer observes a clean
/// failure, not a hang" outcome the raw-QUIC stream paths use.
pub(super) async fn blob_download_step(
    mut state: BlobDownloadState,
) -> Option<(result::Result<Frame<Bytes>, Infallible>, BlobDownloadState)> {
    let params =
        serde_json::json!({"download_id": state.download_id, "max_bytes": BLOB_CHUNK_BYTES});
    let outcome = dispatch_native(
        &state.route_handler,
        &state.pipeline,
        &state.preamble,
        Some(&state.caller),
        "blob-store",
        "read-chunk",
        params,
    )
    .await
    .ok()?;
    let DispatchOutcome::Success(value) = outcome else {
        return None;
    };
    let resp: ReadChunkResponse = serde_json::from_value(value).ok()?;
    if resp.eof {
        // The server already dropped this download_id from its own
        // session map on the EOF path -- no cleanup call needed.
        state.closed = true;
        return None;
    }
    let frame = Frame::data(Bytes::from(resp.chunk));
    Some((Ok(frame), state))
}

use super::*;

/// The outcome of bridging one JSON-RPC round trip through
/// `RouteHandler::dispatch_json_rpc_once` -- `dispatch_json_rpc_once` itself
/// never surfaces a native-service error as `Err`; it always returns
/// `Ok(bytes)` containing either a JSON-RPC `result` or `error` envelope, so
/// callers that want a real HTTP status code have to inspect the envelope.
pub(super) enum DispatchOutcome {
    Success(Value),
    Error { code: i32, message: String },
}

/// Builds and dispatches one native JSON-RPC request through the existing,
/// unchanged `dispatch_json_rpc_once` path, with `preamble.interface`
/// overridden to whichever real native interface (`data-layer`/`blob-store`/
/// `messaging`) the resolved HTTP route implies: a client connects once
/// with `http://http-native|<service_id>`, and
/// `pipeline.service` (resolved once per connection from the `"http-native"`
/// native-capability interface) already points at the right `service_id`
/// regardless of which real interface a given request targets.
pub(super) async fn dispatch_native(
    route_handler: &RouteHandler,
    pipeline: &RoutePipeline,
    preamble: &RoutePreamble,
    caller: Option<&CallerContext>,
    interface: &str,
    method: &str,
    params: Value,
) -> Result<DispatchOutcome> {
    // Every bridged data-layer/messaging route reaches native dispatch
    // through this shared fn, so one guard here covers them all and maps to
    // a clean 401 (ADR-0016 §3, ADR-0016 §4.4) -- rather than the 500 a raw
    // `dispatch_json_rpc_once` rejection would surface. Callers that are
    // already self-authorizing by another mechanism (the signed-URL blob
    // GET, see `handle_blob_get`) pass an explicit `service_system` caller,
    // never `None`, so they never hit this guard.
    if caller.is_none() {
        return Ok(DispatchOutcome::Error {
            code: UNAUTHENTICATED_RPC_CODE,
            message: format!("unauthenticated caller for native interface '{interface}'"),
        });
    }
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(Value::Number(1.into())),
        idempotency_key: None,
    };
    let body = serde_json::to_vec(&request)?;
    let synthetic = RoutePreamble { interface: interface.to_string(), ..preamble.clone() };
    let response_bytes =
        route_handler.dispatch_json_rpc_once(pipeline, &synthetic, caller, &body).await?;
    let response: Value = serde_json::from_slice(&response_bytes)
        .map_err(|e| anyhow!("malformed native-dispatch response: {e}"))?;
    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32;
        let message =
            error.get("message").and_then(Value::as_str).unwrap_or("internal error").to_string();
        Ok(DispatchOutcome::Error { code, message })
    } else {
        Ok(DispatchOutcome::Success(response.get("result").cloned().unwrap_or(Value::Null)))
    }
}

/// Reserved JSON-RPC error code for "no verifiable caller identity" on a
/// bridged native-capability request -- never emitted by a
/// native service itself, only by the `dispatch_native` guard above, and
/// mapped to HTTP 401 below rather than the default 500.
impl HttpHandler {
    /// The original `POST`+`application/json` JSON-RPC bridge, wrapped in the
    /// unified `HttpBody` type. An anonymous caller targeting a native
    /// service is rejected with 401 before dispatch; a WASM-component
    /// target is unaffected.
    pub(super) async fn handle_json_rpc_bridge(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if req.method() != Method::POST {
            // Also where a static-asset miss and a non-`public` bundle land
            // (`try_handle_asset` returning `Ok(None)` for a `GET`/
            // `HEAD` falls all the way through to here): 405, not 404,
            // since this bridge rejects every non-`POST` method uniformly,
            // asset request or not, and special-casing `GET`/`HEAD` here
            // would change behaviour for the ordinary JSON-RPC-bridge case
            // too, not just assets. The property that matters -- absence
            // and refusal look identical from outside -- holds regardless
            // of which 4xx it is; 405 is the deliberate answer.
            return Ok(http_error(StatusCode::METHOD_NOT_ALLOWED, "Only POST is supported".into()));
        }

        let content_type =
            req.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !content_type.starts_with("application/json") {
            return Ok(http_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json".into(),
            ));
        }

        let body_bytes =
            req.collect().await.map_err(|e| anyhow!("Failed to read HTTP body: {e}"))?.to_bytes();

        if body_bytes.is_empty() {
            return Ok(http_error(StatusCode::BAD_REQUEST, "Empty request body".into()));
        }

        // Mirrors `dispatch_native`'s guard: only the native-service
        // arm of `dispatch_json_rpc_once` requires a caller, so only gate
        // here when the resolved pipeline targets one -- an anonymous
        // WASM-component call over this same fallthrough is unaffected.
        if matches!(self.pipeline.service, ServiceStage::NativeService { .. })
            && self.caller.is_none()
        {
            return Ok(structured_rpc_error(
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED_RPC_CODE,
                format!(
                    "unauthenticated caller for native interface '{}'",
                    self.preamble.interface
                ),
            ));
        }

        match self
            .route_handler
            .dispatch_json_rpc_once(
                &self.pipeline,
                &self.preamble,
                self.caller.as_ref(),
                &body_bytes,
            )
            .await
        {
            Ok(payload) => {
                let res = Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(full_body(Bytes::from(payload)));
                Ok(res.unwrap_or_else(|_| Response::default()))
            }
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }

    pub(super) async fn dispatch(
        &self,
        interface: &str,
        method: &str,
        params: Value,
    ) -> Result<DispatchOutcome> {
        dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            self.caller.as_ref(),
            interface,
            method,
            params,
        )
        .await
    }

    /// Percent-decodes `path`, then does an exact-path lookup plus
    /// one rewrite: a path ending in `/` resolves to
    /// `<path>index.html`. This function owns both the decoding and the
    /// rewrite -- callers pass the raw request path and do no
    /// normalisation of their own, so there is exactly one place either
    /// rule lives. Manifest keys come from raw archive entry names (never
    /// encoded), but every browser percent-encodes a request path (a file
    /// named `my file.js` is requested as `/my%20file.js`), so decoding
    /// here -- not at `resolve_route`, which keeps its existing
    /// non-decoding style for API routes -- is what makes such a file
    /// reachable at all. No history-fallback, no prefix rules:
    /// `/api/comments` has no trailing slash, so it is never rewritten and
    /// always falls through to route resolution.
    ///
    /// `None` when the service has no bundle, its declared visibility is
    /// not `public`, `path` isn't valid percent-encoded UTF-8, or no entry
    /// matches the (possibly rewritten) path -- deliberately
    /// indistinguishable, so a miss and a non-public bundle both read as
    /// "not found" to the caller.
    pub(super) fn resolve_route(
        &self,
        method: &Method,
        path: &str,
    ) -> Option<(HttpRoute, Option<String>)> {
        let routes = self.route_handler.inner.http_routes.get(&self.preamble.service_id)?;
        routes.iter().find_map(|route| {
            if !route.method.eq_ignore_ascii_case(method.as_str()) {
                return None;
            }
            match_path(&route.path, path).map(|param| (route.clone(), param))
        })
    }

    pub(super) async fn read_small_body(&self, req: Request<Incoming>) -> Result<BodyRead> {
        let limited = Limited::new(req.into_body(), MAX_SMALL_BODY_BYTES);
        match limited.collect().await {
            Ok(collected) => Ok(BodyRead::Ok(collected.to_bytes())),
            Err(e) => {
                if e.downcast_ref::<LengthLimitError>().is_some() {
                    Ok(BodyRead::Rejected(http_error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!("request body exceeds {MAX_SMALL_BODY_BYTES} byte limit"),
                    )))
                } else {
                    Ok(BodyRead::Rejected(http_error(
                        StatusCode::BAD_REQUEST,
                        format!("failed to read request body: {e}"),
                    )))
                }
            }
        }
    }

    /// `read_small_body` plus the JSON-validity check every small-body
    /// route (`put`/`patch`/`publish`) needs -- collapses each call site's
    /// repeated "read, then reject on non-JSON" block to one match.
    pub(super) async fn read_small_json_body(
        &self,
        req: Request<Incoming>,
    ) -> Result<result::Result<Bytes, Response<HttpBody>>> {
        let body = match self.read_small_body(req).await? {
            BodyRead::Ok(bytes) => bytes,
            BodyRead::Rejected(resp) => return Ok(Err(resp)),
        };
        if serde_json::from_slice::<Value>(&body).is_err() {
            return Ok(Err(http_error(
                StatusCode::BAD_REQUEST,
                "request body must be valid JSON".into(),
            )));
        }
        Ok(Ok(body))
    }

    /// Dispatches one native request and maps its `DispatchOutcome` to an
    /// HTTP response, without special-casing a `null` success value --
    /// shared by every route whose success case is "return the result
    /// as-is" (`query`/`patch`/`publish`). `get` and `put`'s follow-up
    /// fetch-back need different `null` handling per call site (a 404 vs.
    /// an internal error) and use `dispatch_get_response` instead.
    pub(super) async fn dispatch_response(
        &self,
        interface: &str,
        method: &str,
        params: Value,
        ok_status: StatusCode,
    ) -> Result<Response<HttpBody>> {
        Ok(match self.dispatch(interface, method, params).await? {
            DispatchOutcome::Success(value) => json_response(ok_status, &value),
            DispatchOutcome::Error { code, message } => {
                structured_rpc_error(status_for_rpc_error_code(code), code, message)
            }
        })
    }

    /// Dispatches a `data-layer::get` and maps a `null` result (no record
    /// with this id) to `not_found_status`/`not_found_message` -- shared by
    /// the plain `get` route (a genuine 404) and `put`'s follow-up
    /// fetch-back (the record we just wrote being gone is a 500, not a
    /// 404).
    pub(super) async fn dispatch_get_response(
        &self,
        collection: &str,
        id: &str,
        ok_status: StatusCode,
        not_found_status: StatusCode,
        not_found_message: &str,
    ) -> Result<Response<HttpBody>> {
        Ok(
            match self
                .dispatch(
                    "data-layer",
                    "get",
                    serde_json::json!({"collection": collection, "id": id}),
                )
                .await?
            {
                DispatchOutcome::Success(value) if value.is_null() => {
                    http_error(not_found_status, not_found_message.into())
                }
                DispatchOutcome::Success(value) => json_response(ok_status, &value),
                DispatchOutcome::Error { code, message } => {
                    structured_rpc_error(status_for_rpc_error_code(code), code, message)
                }
            },
        )
    }

    pub(super) async fn dispatch_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        match route.target.as_str() {
            "data-layer" => self.handle_data_layer_route(route, path_param, req).await,
            "messaging" => self.handle_messaging_route(route, req).await,
            "stream" => self.handle_stream_route(route, path_param, req).await,
            "guest" => self.handle_guest_route(route, path_param, req).await,
            "websocket" => self.handle_websocket_route(route, req).await,
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("http_routes entry has unknown target: {other}"),
            )),
        }
    }

    // -- data-layer ---------------------------------------------------

    pub(super) async fn handle_data_layer_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let collection = route.collection.clone().unwrap_or_default();
        match route.operation.as_str() {
            "get" => {
                let Some(id) = path_param else {
                    return Ok(http_error(
                        StatusCode::BAD_REQUEST,
                        "route requires a path parameter".into(),
                    ));
                };
                self.dispatch_get_response(
                    &collection,
                    &id,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                    "record not found",
                )
                .await
            }
            "query" => {
                let opts = match query_opts_from_query_string(req.uri().query().unwrap_or("")) {
                    Ok(opts) => opts,
                    Err(message) => return Ok(http_error(StatusCode::BAD_REQUEST, message)),
                };
                self.dispatch_response(
                    "data-layer",
                    "query",
                    serde_json::json!({"collection": collection, "opts": opts}),
                    StatusCode::OK,
                )
                .await
            }
            "put" => {
                let body = match self.read_small_json_body(req).await? {
                    Ok(bytes) => bytes,
                    Err(resp) => return Ok(resp),
                };
                // No `{id}` path segment (a plain `POST /collection`
                // create route) means the record id is server-generated --
                // `data-layer::put`'s WIT signature has no separate
                // create-vs-update distinction (it's an upsert), and this
                // shape carries no id in the path.
                let id = path_param.unwrap_or_else(|| Uuid::new_v4().to_string());
                let value = serde_json::json!({"id": id, "payload": body.to_vec()});
                match self
                    .dispatch(
                        "data-layer",
                        "put",
                        serde_json::json!({"collection": collection, "value": value}),
                    )
                    .await?
                {
                    DispatchOutcome::Error { code, message } => {
                        Ok(structured_rpc_error(status_for_rpc_error_code(code), code, message))
                    }
                    DispatchOutcome::Success(_) => {
                        // `put` itself returns `()` -- fetch the record back
                        // so the HTTP response can return it (a `POST
                        // /orders` returns the resulting record).
                        // A `null` here means the record we just wrote is
                        // already gone (e.g. a concurrent delete raced this
                        // request) -- that's a 500, not the plain-`get`
                        // route's 404, since the write itself succeeded.
                        self.dispatch_get_response(
                            &collection,
                            &id,
                            StatusCode::CREATED,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "record vanished immediately after being written",
                        )
                        .await
                    }
                }
            }
            "patch" => {
                let Some(id) = path_param else {
                    return Ok(http_error(
                        StatusCode::BAD_REQUEST,
                        "route requires a path parameter".into(),
                    ));
                };
                let body = match self.read_small_json_body(req).await? {
                    Ok(bytes) => bytes,
                    Err(resp) => return Ok(resp),
                };
                self.dispatch_response(
                    "data-layer",
                    "patch",
                    serde_json::json!({"collection": collection, "id": id, "patch_json": body.to_vec()}),
                    StatusCode::OK,
                )
                .await
            }
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported data-layer operation: {other}"),
            )),
        }
    }

    // -- messaging ------------------------------------------------------

    pub(super) async fn handle_messaging_route(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        match route.operation.as_str() {
            "publish" => self.handle_messaging_publish(route, req).await,
            "subscribe-sse" => self.handle_messaging_sse(route, req).await,
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported messaging operation: {other}"),
            )),
        }
    }

    pub(super) async fn handle_messaging_publish(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let topic = route.topic.clone().unwrap_or_default();
        let body = match self.read_small_json_body(req).await? {
            Ok(bytes) => bytes,
            Err(resp) => return Ok(resp),
        };
        self.dispatch_response(
            "messaging",
            "publish",
            serde_json::json!({"topic": topic, "payload": body.to_vec()}),
            StatusCode::OK,
        )
        .await
    }
}

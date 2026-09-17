use std::ops::ControlFlow;

use super::*;

impl HttpHandler {
    /// The fourth `dispatch_route` target: hands the request to the
    /// deployed component's `syneroym:http/incoming-handler#handle-request`
    /// export and turns its answer into an HTTP response. Reaches the guest
    /// directly through `app_sandbox_engine`, mirroring
    /// `handle_stream_route` -- an `http-native` connection resolves to a
    /// `NativeService` pipeline, so `dispatch_json_rpc_once` can never
    /// reach a guest, unlike `data-layer`/`messaging` above.
    pub(super) async fn handle_guest_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if route.operation != "handle-request" {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported guest operation: {}", route.operation),
            ));
        }

        let (parts, body) = req.into_parts();
        let headers = match guest_request_headers(&parts.headers) {
            Ok(headers) => headers,
            Err((status, message)) => return Ok(http_error(status, message)),
        };

        let caller_identity = match guest_caller_identity(self.caller.as_ref(), &self.preamble) {
            Ok(id) => id,
            Err(e) => return Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e)),
        };

        // BEFORE any engine work: an unauthenticated caller on a
        // non-public route never instantiates anything. Same code and
        // status shape `dispatch_native` uses, so one 401 taxonomy covers
        // the whole bridge.
        if self.caller.is_none() && !route.public {
            return Ok(structured_rpc_error(
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED_RPC_CODE,
                format!("unauthenticated caller for guest route {} {}", route.method, route.path),
            ));
        }

        let native = self
            .route_handler
            .inner
            .native_http
            .get(&self.preamble.service_id)
            .map(|e| e.value().clone());

        let app_sandbox_engine = match self.resolve_guest_engine(native.is_some()) {
            ControlFlow::Break(resp) => return Ok(resp),
            ControlFlow::Continue(engine) => engine,
        };

        // Every rejection above happens before any engine call, so each
        // costs zero instantiations.
        let body_bytes = match Self::read_guest_body(body).await {
            ControlFlow::Break(resp) => return Ok(resp),
            ControlFlow::Continue(bytes) => bytes,
        };

        let path_params = match (param_name(&route.path), path_param) {
            (Some(name), Some(value)) => vec![(name.to_string(), value)],
            _ => vec![],
        };
        let request = HttpRequest {
            method: parts.method.as_str().to_string(),
            path: parts.uri.path().to_string(),
            query: parts.uri.query().unwrap_or("").to_string(),
            route: route.path.clone(),
            path_params,
            headers,
            body: body_bytes.to_vec(),
            caller: caller_identity,
        };

        if let Some(svc) = native {
            Ok(Self::dispatch_native_guest_request(
                &svc,
                request,
                self.caller.clone(),
                &self.preamble.service_id,
                &route.path,
            )
            .await)
        } else if let Some(app_sandbox_engine) = app_sandbox_engine {
            Ok(Self::dispatch_sandbox_guest_request(
                &app_sandbox_engine,
                &self.preamble.service_id,
                &request,
                self.caller.clone(),
                &route.path,
            )
            .await)
        } else {
            Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, "no HTTP handler available".into()))
        }
    }

    /// Picks the guest HTTP target. `native_present` true means a natively
    /// linked service already claims the route -- a deployed WASM
    /// component underneath it, if any, is only logged as shadowed, never
    /// used, so this returns `Continue(None)`. Otherwise a deployed
    /// component must serve it: `Break` carries the response to return
    /// immediately when there is no sandbox engine at all (coordinator
    /// mode) or no component deployed for this service, the same as the
    /// early `return Ok(...)` this replaces.
    fn resolve_guest_engine(
        &self,
        native_present: bool,
    ) -> ControlFlow<Response<HttpBody>, Option<Arc<AppSandboxEngine>>> {
        if native_present {
            if let Some(engine) = &self.route_handler.inner.app_sandbox_engine
                && engine.is_deployed(&self.preamble.service_id)
            {
                warn!(
                    service_id = %self.preamble.service_id,
                    "native_http service shadows deployed WASM component"
                );
            }
            return ControlFlow::Continue(None);
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
        ControlFlow::Continue(Some(engine))
    }

    /// Reads and size-limits the guest request body. `Break` carries the
    /// error response to return immediately -- over the byte limit, or
    /// unreadable for any other reason -- the same as the early `return
    /// Ok(...)` this replaces.
    async fn read_guest_body(body: Incoming) -> ControlFlow<Response<HttpBody>, Bytes> {
        let limited = Limited::new(body, MAX_GUEST_REQUEST_BODY_BYTES);
        match limited.collect().await {
            Ok(collected) => ControlFlow::Continue(collected.to_bytes()),
            Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
                ControlFlow::Break(http_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("request body exceeds {MAX_GUEST_REQUEST_BODY_BYTES} byte limit"),
                ))
            }
            Err(e) => ControlFlow::Break(http_error(
                StatusCode::BAD_REQUEST,
                format!("failed to read request body: {e}"),
            )),
        }
    }

    /// Dispatches to a natively linked service's HTTP handler and turns its
    /// answer into a response. Always produces a response rather than an
    /// `Err`: a handler failure becomes an HTTP 500, so the caller can wrap
    /// this in `Ok(..)` unconditionally.
    async fn dispatch_native_guest_request(
        svc: &Arc<dyn NativeHttpService>,
        request: HttpRequest,
        caller: Option<CallerContext>,
        service_id: &str,
        route_path: &str,
    ) -> Response<HttpBody> {
        match svc.handle_request(request, caller).await {
            Ok(response) => build_guest_response(response),
            Err(detail) => {
                warn!(
                    service_id = %service_id,
                    route = %route_path,
                    error = %detail,
                    "native HTTP handler failed"
                );
                http_error(StatusCode::INTERNAL_SERVER_ERROR, detail)
            }
        }
    }

    /// Dispatches to a deployed WASM component's HTTP export and turns its
    /// answer into a response. Always produces a response rather than an
    /// `Err`, the same as `dispatch_native_guest_request`.
    async fn dispatch_sandbox_guest_request(
        app_sandbox_engine: &Arc<AppSandboxEngine>,
        service_id: &str,
        request: &HttpRequest,
        caller: Option<CallerContext>,
        route_path: &str,
    ) -> Response<HttpBody> {
        match app_sandbox_engine.handle_guest_http_request(service_id, request, caller).await {
            Ok(GuestHttpOutcome::Response(response)) => build_guest_response(response),
            Ok(GuestHttpOutcome::Failed(GuestHttpFailure::Unavailable(detail))) => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("service is at its guest HTTP concurrency limit: {detail}"),
                );
                resp.headers_mut().insert("retry-after", HeaderValue::from_static("1"));
                resp
            }
            Ok(GuestHttpOutcome::Failed(failure)) => {
                // `Declined` is the guest's own `Err` return -- an ordinary
                // application-level outcome, and on a `public: true` route
                // one any anonymous caller can trigger at will. Logging it
                // at `error!` would make the node's error log a rate the
                // caller controls; every other variant is a genuine host or
                // component-shape problem and stays at `error!`.
                if matches!(failure, GuestHttpFailure::Declined(_)) {
                    warn!(
                        service_id = %service_id,
                        route = %route_path,
                        ?failure,
                        "guest HTTP handler declined the request"
                    );
                } else {
                    error!(
                        service_id = %service_id,
                        route = %route_path,
                        ?failure,
                        "guest HTTP handler failed"
                    );
                }
                http_error(StatusCode::INTERNAL_SERVER_ERROR, describe_guest_http_failure(&failure))
            }
            Err(e) => http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}

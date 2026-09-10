use super::*;

impl HttpHandler {
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

        let app_sandbox_engine = if let Some(_svc) = &native {
            if let Some(engine) = &self.route_handler.inner.app_sandbox_engine
                && engine.is_deployed(&self.preamble.service_id)
            {
                warn!(
                    service_id = %self.preamble.service_id,
                    "native_http service shadows deployed WASM component"
                );
            }
            None
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
            Some(engine)
        };

        // Every rejection above happens before any engine call, so each
        // costs zero instantiations.
        let limited = Limited::new(body, MAX_GUEST_REQUEST_BODY_BYTES);
        let body_bytes = match limited.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
                return Ok(http_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("request body exceeds {MAX_GUEST_REQUEST_BODY_BYTES} byte limit"),
                ));
            }
            Err(e) => {
                return Ok(http_error(
                    StatusCode::BAD_REQUEST,
                    format!("failed to read request body: {e}"),
                ));
            }
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
            match svc.handle_request(request, self.caller.clone()).await {
                Ok(response) => Ok(build_guest_response(response)),
                Err(detail) => {
                    warn!(
                        service_id = %self.preamble.service_id,
                        route = %route.path,
                        error = %detail,
                        "native HTTP handler failed"
                    );
                    Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, detail))
                }
            }
        } else if let Some(app_sandbox_engine) = app_sandbox_engine {
            match app_sandbox_engine
                .handle_guest_http_request(&self.preamble.service_id, &request, self.caller.clone())
                .await
            {
                Ok(GuestHttpOutcome::Response(response)) => Ok(build_guest_response(response)),
                Ok(GuestHttpOutcome::Failed(GuestHttpFailure::Unavailable(detail))) => {
                    let mut resp = http_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("service is at its guest HTTP concurrency limit: {detail}"),
                    );
                    resp.headers_mut().insert("retry-after", HeaderValue::from_static("1"));
                    Ok(resp)
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
                            service_id = %self.preamble.service_id,
                            route = %route.path,
                            ?failure,
                            "guest HTTP handler declined the request"
                        );
                    } else {
                        error!(
                            service_id = %self.preamble.service_id,
                            route = %route.path,
                            ?failure,
                            "guest HTTP handler failed"
                        );
                    }
                    Ok(http_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        describe_guest_http_failure(&failure),
                    ))
                }
                Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        } else {
            Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, "no HTTP handler available".into()))
        }
    }
}

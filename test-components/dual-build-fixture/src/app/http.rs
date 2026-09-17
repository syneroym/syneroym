use super::*;

/// Echoes every field of the request back as JSON, or handles sub-paths.
pub async fn handle_http<H: AppHost>(host: &H, req: HttpRequest) -> Result<HttpResponse, String> {
    let path = req.path.as_str();
    if path == "/echo" {
        let body = serde_json::to_vec(&json!({
            "method": req.method,
            "path": req.path,
            "query": req.query,
            "route": req.route,
            "path-params": req.path_params,
            "headers": req.headers,
            "body": String::from_utf8_lossy(&req.body),
            "caller": req.caller.as_ref().map(|c| json!({
                "did": c.did,
                "auth": match c.auth {
                    CallerAuth::Delegated => "delegated",
                    CallerAuth::Ucan => "ucan",
                    CallerAuth::SelfAsserted => "self-asserted",
                },
                "app-instance": c.app_instance,
            })),
        }))
        .map_err(|e| e.to_string())?;
        Ok(HttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
        })
    } else if path.starts_with("/store") {
        ensure_collection(host, HTTP_STORE).await.map_err(fmt_err)?;
        let id = if req.query.is_empty() { "default".to_string() } else { req.query.clone() };
        host.put(HTTP_STORE.into(), RecordWriteValue { id, payload: req.body.clone() })
            .await
            .map_err(fmt_err)?;
        Ok(HttpResponse { status: 200, headers: vec![], body: b"stored".to_vec() })
    } else if path == "/reject" {
        Ok(HttpResponse { status: 403, headers: vec![], body: b"forbidden".to_vec() })
    } else if path == "/fail" {
        Err("handler exploded intentionally".to_string())
    } else if path == "/whoami" {
        let caller_did = req.caller.as_ref().map(|c| c.did.as_str()).unwrap_or("anonymous");
        Ok(HttpResponse { status: 200, headers: vec![], body: caller_did.as_bytes().to_vec() })
    } else if path == "/origin" {
        // Reports `invocation.caller()`'s arm: a guest HTTP request is
        // router ingress, so this must never answer `internal`.
        let arm = match AppInvocation::caller(host).await {
            CallerOrigin::Internal => "internal",
            CallerOrigin::Verified(_) => "verified",
            CallerOrigin::Anonymous => "anonymous",
        };
        Ok(HttpResponse { status: 200, headers: vec![], body: arm.as_bytes().to_vec() })
    } else {
        Ok(HttpResponse { status: 200, headers: vec![], body: b"ok".to_vec() })
    }
}

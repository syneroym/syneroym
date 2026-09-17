use super::*;

pub(super) async fn handle_bootstrap(
    State(state): State<Arc<BootstrapState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let host =
        req.headers().get(HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost").to_string();
    let path = req.uri().path();
    if path == "/favicon.ico" {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (mut target_peer_id, mut target_service_id, target_interface, already_resolved) =
        match parse_target_host(&host) {
            None => (host.clone(), host.clone(), String::new(), false),
            Some(TargetHost::Service { lookup_alias, interface }) => {
                (lookup_alias.clone(), lookup_alias, interface, false)
            }
            Some(TargetHost::App {
                app_lookup_alias,
                app_did_hash,
                service_name_hash,
                interface,
            }) => {
                // Resolve through Tier 1 -> Tier 2 -> member
                // selection, exactly as the client gateway's own
                // app-scoped path does (both binding checks are
                // applied inside `resolve_app_host`, shared rather than
                // reimplemented). A failed resolve returns an error,
                // never the raw-host fallback below -- that fallback
                // would dial whatever the hostname happened to spell.
                let member_did = match state
                    .app_host_resolver
                    .resolve_app_host(&app_lookup_alias, &app_did_hash, &service_name_hash, None)
                    .await
                {
                    Ok(m) => m,
                    Err(e) => {
                        error!("coordinator failed to resolve app-scoped host '{host}': {e:#}");
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };
                // Tier 3, exactly as the physical path already does it:
                // the member DID's own endpoint record names the
                // substrate hosting it. Already resolved -- the alias
                // lookup below is for the physical path's own alias,
                // which `member_did` already is not.
                match state.registry_client.lookup(&member_did, true).await {
                    Ok(rec) => (rec.info.substrate_id, member_did, interface, true),
                    Err(e) => {
                        error!(
                            "coordinator failed to resolve Tier 3 for member '{member_did}': {e:#}"
                        );
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                }
            }
        };

    if !already_resolved && state.registry_url.is_some() {
        debug!("Attempting to resolve alias: {}", target_peer_id);
        if let Ok(info) = state.registry_client.lookup(&target_peer_id, true).await {
            info!(
                "Resolved service alias '{}' to substrate DID '{}' and service DID '{}'",
                target_peer_id, info.info.substrate_id, info.info.service_id
            );
            target_peer_id = info.info.substrate_id;
            target_service_id = info.info.service_id;
        }
    }

    let signaling_server_url =
        construct_signaling_url("ws", &host, &state.external_host, state.signaling_port);

    let target_pubkey_hex = match resolve_did_key(&target_peer_id) {
        Ok(pubkey) => hex::encode(pubkey.as_bytes()),
        Err(e) => {
            error!("Failed to resolve target_peer_id '{}' DID: {}", target_peer_id, e);
            String::new()
        }
    };

    let tpl = PeerProxyTemplate {
        target_peer_id,
        target_service_id,
        signaling_server_url,
        http_version: "HTTP/1.1".to_string(),
        target_pubkey_hex,
        target_interface,
    };

    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Failed to render template: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub(super) fn construct_signaling_url(
    scheme: &str,
    host: &str,
    external_host: &Option<String>,
    signaling_port: u16,
) -> String {
    let signaling_host = if let Some(h) = external_host {
        h.clone()
    } else {
        // Strip port from Host header if present
        host.split(':').next().unwrap_or("localhost").to_string()
    };

    format!("{scheme}://{signaling_host}:{signaling_port}/ws")
}

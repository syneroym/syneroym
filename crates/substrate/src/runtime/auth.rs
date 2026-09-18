//! Authentication service initialization.

#[cfg(feature = "auth")]
use std::sync::Arc;

#[cfg(feature = "auth")]
use syneroym_auth::AuthService;
#[cfg(feature = "auth")]
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::RegistryClient,
    http_routes::HttpRoute,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    protocol_utils::AUTH_SERVICE_ALIAS,
};
#[cfg(feature = "auth")]
use syneroym_identity::Identity;
#[cfg(feature = "auth")]
use syneroym_rpc::NativeHttpService;

#[cfg(feature = "auth")]
use super::handles::SharedNodeHandles;

#[cfg(feature = "auth")]
pub(super) async fn init_auth_service(
    config: &SubstrateConfig,
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    node_service_id: &str,
) -> anyhow::Result<Option<Arc<AuthService>>> {
    let Some(auth_cfg) = &config.roles.auth else {
        return Ok(None);
    };

    let auth_identity = if let Some(key_path) = &auth_cfg.key_path {
        Identity::load_from_path(key_path)?
    } else {
        Identity::generate()?
    };

    let anchor_resolver = Arc::new(RegistryClient::new(
        config.substrate.enable_bep0044_dht,
        config.substrate.registry_url.clone(),
    ));

    let auth_service = Arc::new(
        AuthService::new(
            auth_identity,
            node_service_id.to_string(),
            auth_cfg.session_ttl_secs,
            auth_cfg.nonce_ttl_secs,
            auth_cfg.person_identities_dir.clone(),
            anchor_resolver,
        )
        .with_allowed_origins(auth_cfg.allowed_origins.clone())
        .with_secure_cookies(auth_cfg.secure_cookies),
    );

    let auth_did = auth_service.auth_did().to_string();

    shared
        .native_http()
        .insert(AUTH_SERVICE_ALIAS.to_string(), auth_service.clone() as Arc<dyn NativeHttpService>);
    shared
        .native_http()
        .insert(auth_did.clone(), auth_service.clone() as Arc<dyn NativeHttpService>);

    let auth_routes = auth_http_routes();
    register_auth_endpoints(shared, &auth_did, &auth_routes, endpoint_registry).await?;

    Ok(Some(auth_service))
}

/// The static HTTP route table the auth service exposes. All 21 routes share
/// the same `guest`/`handle-request` target and are public; only the method
/// and path differ, so each is built through `auth_route` rather than
/// repeating the other six fields 21 times. Extracted so the route list is
/// testable independently and `init_auth_service` reads as orchestration
/// rather than a large literal block.
#[cfg(feature = "auth")]
fn auth_http_routes() -> Vec<HttpRoute> {
    // Paired short paths (`/challenge`, `/login`, …) and their canonical
    // `/_syneroym/session/*` equivalents (some tooling targets only the
    // canonical form; both forms are kept for backward compatibility).
    vec![
        auth_route("POST", "/challenge"),
        auth_route("POST", "/login"),
        auth_route("GET", "/methods"),
        auth_route("GET", "/whoami"),
        auth_route("POST", "/logout"),
        auth_route("POST", "/refresh"),
        auth_route("POST", "/_syneroym/session/challenge"),
        auth_route("POST", "/_syneroym/session/login"),
        auth_route("GET", "/_syneroym/session/methods"),
        auth_route("GET", "/_syneroym/session/whoami"),
        auth_route("POST", "/_syneroym/session/logout"),
        auth_route("POST", "/_syneroym/session/refresh"),
        auth_route("POST", "/_syneroym/session/{endpoint}"),
        auth_route("GET", "/_syneroym/session/{endpoint}"),
        auth_route("OPTIONS", "/challenge"),
        auth_route("OPTIONS", "/login"),
        auth_route("OPTIONS", "/methods"),
        auth_route("OPTIONS", "/whoami"),
        auth_route("OPTIONS", "/logout"),
        auth_route("OPTIONS", "/refresh"),
        auth_route("OPTIONS", "/_syneroym/session/{endpoint}"),
    ]
}

/// Builds one auth-service route entry. Every entry targets the guest's
/// `handle-request` export, is public (the auth handshake itself is how an
/// otherwise-anonymous caller gets a session), and uses none of the
/// `collection`/`topic`/`protocol` fields, so only `method` and `path` need
/// to vary per call.
#[cfg(feature = "auth")]
fn auth_route(method: &str, path: &str) -> HttpRoute {
    HttpRoute {
        method: method.into(),
        path: path.into(),
        target: "guest".into(),
        operation: "handle-request".into(),
        collection: None,
        topic: None,
        protocol: None,
        public: true,
    }
}

/// Inserts the auth service's route table and registers its two endpoint
/// entries (one under `AUTH_SERVICE_ALIAS`, one under the auth DID). Split
/// from `init_auth_service` so that function reads as orchestration.
#[cfg(feature = "auth")]
async fn register_auth_endpoints(
    shared: &SharedNodeHandles,
    auth_did: &str,
    routes: &[HttpRoute],
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<()> {
    shared.http_routes().insert(AUTH_SERVICE_ALIAS.to_string(), routes.to_vec());
    shared.http_routes().insert(auth_did.to_string(), routes.to_vec());

    endpoint_registry
        .register(
            AUTH_SERVICE_ALIAS.to_string(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: AUTH_SERVICE_ALIAS.to_string() },
        )
        .await?;
    endpoint_registry
        .register(
            auth_did.to_string(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: auth_did.to_string() },
        )
        .await?;

    Ok(())
}

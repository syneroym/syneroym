use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use dashmap::DashMap;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};
use syneroym_core::{
    dht_registry::MasterAnchorResolver,
    protocol_utils::{SESSION_COOKIE_NAME, SessionRevocationCheck, gateway_session_assertion},
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SESSION_AUTH, substrate,
};
use syneroym_rpc::{CallerContext, NativeHttpService};
use tracing::warn;

use crate::token::{
    AUTH_METHOD_DELEGATED_KEY, AUTH_METHOD_LOCAL, CLAIM_DELEGATION_EXPIRES_AT_SECS, SessionToken,
};

pub const MAX_PENDING_CHALLENGES: usize = 4096;
pub const MAX_LOGGED_OUT_TOKENS: usize = 10000;
pub const DEFAULT_NONCE_TTL_SECS: u64 = 60;
pub const DEFAULT_SESSION_TTL_SECS: u64 = 8 * 3600;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub nonce: String,
    pub node_did: String,
    pub expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedKeyLoginParams {
    pub temp_did: String,
    pub delegation: DelegationCertificate,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalLoginParams {
    pub identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub temp_did: Option<String>,
    #[serde(default)]
    pub delegation: Option<DelegationCertificate>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub person_did: String,
    pub expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoamiResponse {
    pub person_did: String,
    pub auth: String,
    pub expires_at_secs: u64,
    #[serde(default)]
    pub facts: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodsResponse {
    pub methods: Vec<String>,
}

/// The Node Auth Service.
#[derive(Debug)]
pub struct AuthService {
    auth_identity: Identity,
    auth_did: String,
    node_did: String,
    session_ttl_secs: u64,
    nonce_ttl_secs: u64,
    challenges: DashMap<String, u64>,
    logged_out_tokens: DashMap<String, u64>,
    person_identities_dir: Option<PathBuf>,
    anchor_resolver: Arc<dyn MasterAnchorResolver>,
}

impl AuthService {
    pub fn new(
        auth_identity: Identity,
        node_did: String,
        session_ttl_secs: u64,
        nonce_ttl_secs: u64,
        person_identities_dir: Option<PathBuf>,
        anchor_resolver: Arc<dyn MasterAnchorResolver>,
    ) -> Self {
        let auth_did = substrate::derive_did_key(&auth_identity.public_key());
        Self {
            auth_identity,
            auth_did,
            node_did,
            session_ttl_secs,
            nonce_ttl_secs,
            challenges: DashMap::new(),
            logged_out_tokens: DashMap::new(),
            person_identities_dir,
            anchor_resolver,
        }
    }

    #[must_use]
    pub fn auth_did(&self) -> &str {
        &self.auth_did
    }

    #[must_use]
    pub fn node_did(&self) -> &str {
        &self.node_did
    }

    fn sweep_challenges(&self, now: u64) {
        self.challenges.retain(|_, expires_at| *expires_at > now);
    }

    fn sweep_logged_out_tokens(&self, now: u64) {
        self.logged_out_tokens.retain(|_, expires_at| *expires_at > now);
    }

    pub fn record_logout(&self, token_str: &str) {
        let now = now_secs();
        self.sweep_logged_out_tokens(now);

        let expires_at = if let Ok(claims) = SessionToken::verify_any_issuer(token_str) {
            claims.expires_at_secs
        } else {
            now + self.session_ttl_secs
        };

        if self.logged_out_tokens.len() >= MAX_LOGGED_OUT_TOKENS {
            let oldest = self
                .logged_out_tokens
                .iter()
                .min_by_key(|entry| *entry.value())
                .map(|entry| entry.key().clone());
            if let Some(key) = oldest {
                self.logged_out_tokens.remove(&key);
            }
        }

        self.logged_out_tokens.insert(token_str.to_string(), expires_at);
    }

    pub fn is_logged_out(&self, token_str: &str) -> bool {
        let now = now_secs();
        if let Some(exp) = self.logged_out_tokens.get(token_str) {
            if *exp > now {
                return true;
            }
            drop(exp);
            self.logged_out_tokens.remove(token_str);
        }
        false
    }

    pub fn issue_challenge(&self) -> ChallengeResponse {
        let now = now_secs();
        self.sweep_challenges(now);

        if self.challenges.len() >= MAX_PENDING_CHALLENGES {
            let oldest = self
                .challenges
                .iter()
                .min_by_key(|entry| *entry.value())
                .map(|entry| entry.key().clone());
            if let Some(key) = oldest {
                self.challenges.remove(&key);
            }
        }

        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let nonce = hex::encode(bytes);
        let expires_at_secs = now + self.nonce_ttl_secs;
        self.challenges.insert(nonce.clone(), expires_at_secs);

        ChallengeResponse { nonce, node_did: self.node_did.clone(), expires_at_secs }
    }

    pub async fn login_delegated_key(
        &self,
        params: &DelegatedKeyLoginParams,
    ) -> Result<LoginResponse, (u16, &'static str)> {
        let now = now_secs();
        self.sweep_challenges(now);

        // Check nonce
        let (_, expires_at) =
            self.challenges.remove(&params.nonce).ok_or((401, "unknown or already used nonce"))?;
        if now >= expires_at {
            return Err((401, "expired nonce"));
        }

        // Verify delegation
        if params.temp_did != params.delegation.temporary_did {
            return Err((401, "temporary DID does not match delegation"));
        }

        let accepted_scopes = [SCOPE_SESSION_AUTH];
        if let Err(e) = params.delegation.verify(&params.delegation.master_did, &accepted_scopes) {
            warn!(error = %e, "delegation certificate verification failed");
            return Err((401, "invalid delegation certificate"));
        }

        if now >= params.delegation.expires_at_secs {
            return Err((401, "delegation certificate has expired"));
        }

        // Resolve master anchor to check revocation
        let master_anchor = match self
            .anchor_resolver
            .resolve_master_anchor(&params.delegation.master_did)
            .await
        {
            Ok(payload) => payload,
            Err(e) => {
                warn!(master_did = %params.delegation.master_did, error = %e, "master anchor unresolvable");
                return Err((409, "person master anchor is not resolvable"));
            }
        };

        let temp_pubkey_res = substrate::resolve_did_key(&params.temp_did);
        let temp_pubkey_hex =
            temp_pubkey_res.as_ref().map(|pk| hex::encode(pk.as_bytes())).unwrap_or_default();
        if master_anchor.revoked_keys.contains(&params.temp_did)
            || master_anchor.revoked_keys.contains(&temp_pubkey_hex)
        {
            return Err((401, "delegated key is in master revoked_keys list"));
        }

        // Verify signature over assertion by temp_did or master_did
        let assertion =
            gateway_session_assertion(&self.node_did, &params.nonce, &params.delegation.master_did);
        let is_temp_sig =
            substrate::verify_json_signature(&params.temp_did, &assertion, &params.signature)
                .is_ok();
        let is_master_sig = substrate::verify_json_signature(
            &params.delegation.master_did,
            &assertion,
            &params.signature,
        )
        .is_ok();

        if !is_temp_sig && !is_master_sig {
            warn!("challenge signature verification failed");
            return Err((401, "invalid signature"));
        }

        let remaining_delegation_ttl = params.delegation.expires_at_secs.saturating_sub(now);
        let token_ttl = remaining_delegation_ttl.min(self.session_ttl_secs);
        if token_ttl == 0 {
            return Err((401, "delegation certificate has expired"));
        }

        let mut additional_facts = Map::new();
        additional_facts.insert(
            CLAIM_DELEGATION_EXPIRES_AT_SECS.to_string(),
            Value::from(params.delegation.expires_at_secs),
        );

        let session_token = SessionToken::mint(
            &self.auth_identity,
            &params.delegation.master_did,
            AUTH_METHOD_DELEGATED_KEY,
            Some(additional_facts),
            token_ttl,
        )
        .map_err(|_| (500, "failed to mint session token"))?;

        let token_str =
            session_token.to_token_string().map_err(|_| (500, "failed to encode session token"))?;

        Ok(LoginResponse {
            token: token_str,
            person_did: params.delegation.master_did.clone(),
            expires_at_secs: now + token_ttl,
        })
    }

    pub fn login_local(&self, identity_name: &str) -> Result<LoginResponse, (u16, &'static str)> {
        let identities_dir = self
            .person_identities_dir
            .as_ref()
            .ok_or((400, "local login method is not enabled"))?;

        let trimmed = identity_name.trim();
        if trimmed.is_empty()
            || trimmed.contains('/')
            || trimmed.contains('\\')
            || trimmed.contains("..")
        {
            return Err((400, "invalid identity name"));
        }

        let mut key_path = identities_dir.join(format!("{trimmed}.key"));
        if !key_path.exists() {
            key_path = identities_dir.join(trimmed);
        }
        if !key_path.exists() {
            return Err((401, "unknown local identity"));
        }

        let secret_bytes = match fs::read(&key_path) {
            Ok(bytes) => {
                if bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    arr
                } else if let Ok(s) = std::str::from_utf8(&bytes) {
                    if let Ok(hex_bytes) = hex::decode(s.trim()) {
                        if hex_bytes.len() == 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&hex_bytes);
                            arr
                        } else {
                            return Err((500, "invalid key file length"));
                        }
                    } else {
                        return Err((500, "invalid key file format"));
                    }
                } else {
                    return Err((500, "invalid key file content"));
                }
            }
            Err(_) => return Err((500, "failed to read identity key file")),
        };

        let person_identity = Identity::from_bytes(&secret_bytes);
        let person_did = substrate::derive_did_key(&person_identity.public_key());

        let now = now_secs();
        let session_token = SessionToken::mint(
            &self.auth_identity,
            &person_did,
            AUTH_METHOD_LOCAL,
            Some({
                let mut map = Map::new();
                map.insert("local_name".to_string(), Value::String(trimmed.to_string()));
                map
            }),
            self.session_ttl_secs,
        )
        .map_err(|_| (500, "failed to mint session token"))?;

        let token_str =
            session_token.to_token_string().map_err(|_| (500, "failed to encode session token"))?;

        Ok(LoginResponse {
            token: token_str,
            person_did,
            expires_at_secs: now + self.session_ttl_secs,
        })
    }

    pub fn methods(&self) -> MethodsResponse {
        let mut methods = vec![AUTH_METHOD_DELEGATED_KEY.to_string()];
        if self.person_identities_dir.is_some() {
            methods.push(AUTH_METHOD_LOCAL.to_string());
        }
        MethodsResponse { methods }
    }

    pub fn whoami(&self, token_str: &str) -> Result<WhoamiResponse, (u16, &'static str)> {
        if self.is_logged_out(token_str) {
            return Err((401, "session token has been logged out"));
        }
        let claims = SessionToken::verify(token_str, &self.auth_did)
            .map_err(|_| (401, "invalid or expired session token"))?;

        let auth_method = claims.auth_method().unwrap_or(AUTH_METHOD_DELEGATED_KEY).to_string();

        Ok(WhoamiResponse {
            person_did: claims.person_did,
            auth: auth_method,
            expires_at_secs: claims.expires_at_secs,
            facts: claims.facts,
        })
    }

    pub fn refresh(&self, token_str: &str) -> Result<LoginResponse, (u16, &'static str)> {
        if self.is_logged_out(token_str) {
            return Err((401, "session token has been logged out"));
        }
        let claims = SessionToken::verify(token_str, &self.auth_did)
            .map_err(|_| (401, "invalid or expired session token"))?;

        let now = now_secs();
        let auth_method = claims.auth_method().unwrap_or(AUTH_METHOD_DELEGATED_KEY).to_string();

        // Enforce delegation expiry cap if present
        let token_ttl = if let Some(del_exp) = claims.delegation_expires_at_secs() {
            if now >= del_exp {
                return Err((401, "delegation certificate has expired"));
            }
            let remaining = del_exp.saturating_sub(now);
            remaining.min(self.session_ttl_secs)
        } else {
            self.session_ttl_secs
        };

        if token_ttl == 0 {
            return Err((401, "delegation certificate has expired"));
        }

        let new_token = SessionToken::mint(
            &self.auth_identity,
            &claims.person_did,
            &auth_method,
            Some(claims.facts),
            token_ttl,
        )
        .map_err(|_| (500, "failed to refresh session token"))?;

        let token_str =
            new_token.to_token_string().map_err(|_| (500, "failed to encode session token"))?;

        Ok(LoginResponse {
            token: token_str,
            person_did: claims.person_did,
            expires_at_secs: now + token_ttl,
        })
    }

    fn extract_token(&self, req: &HttpRequest) -> Option<String> {
        // Look in Cookie header
        for (name, val) in &req.headers {
            if name.eq_ignore_ascii_case("cookie") {
                for pair in val.split(';') {
                    let mut parts = pair.splitn(2, '=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next())
                        && k.trim() == SESSION_COOKIE_NAME
                    {
                        return Some(v.trim().to_string());
                    }
                }
            }
        }
        // Look in Authorization header
        for (name, val) in &req.headers {
            if name.eq_ignore_ascii_case("authorization") {
                let trimmed = val.trim();
                if let Some(tok) = trimmed.strip_prefix("Bearer ") {
                    return Some(tok.trim().to_string());
                }
                if let Some(tok) = trimmed.strip_prefix("bearer ") {
                    return Some(tok.trim().to_string());
                }
            }
        }
        None
    }

    fn cors_headers(origin: Option<&str>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("access-control-allow-methods".to_string(), "GET, POST, OPTIONS".to_string()),
            (
                "access-control-allow-headers".to_string(),
                "content-type, authorization, x-syneroym-routing-key".to_string(),
            ),
        ];
        if let Some(orig) = origin {
            headers.push(("access-control-allow-origin".to_string(), orig.to_string()));
            headers.push(("access-control-allow-credentials".to_string(), "true".to_string()));
            headers.push(("vary".to_string(), "Origin".to_string()));
        } else {
            headers.push(("access-control-allow-origin".to_string(), "*".to_string()));
        }
        headers
    }
}

impl SessionRevocationCheck for AuthService {
    fn is_revoked(&self, token: &str) -> bool {
        self.is_logged_out(token)
    }
}

#[async_trait::async_trait]
impl NativeHttpService for AuthService {
    async fn handle_request(
        &self,
        request: HttpRequest,
        _caller: Option<CallerContext>,
    ) -> std::result::Result<HttpResponse, String> {
        let method = request.method.to_uppercase();
        let path = request.path.trim();

        let origin = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
            .map(|(_, v)| v.as_str());

        let mut headers = Self::cors_headers(origin);
        headers.push(("content-type".to_string(), "application/json".to_string()));

        if method == "OPTIONS" {
            return Ok(HttpResponse { status: 204, headers, body: vec![] });
        }

        let is_challenge = path == "/challenge" || path == "/_syneroym/session/challenge";
        let is_login = path == "/login" || path == "/_syneroym/session/login";
        let is_methods = path == "/methods" || path == "/_syneroym/session/methods";
        let is_whoami = path == "/whoami" || path == "/_syneroym/session/whoami";
        let is_logout = path == "/logout" || path == "/_syneroym/session/logout";
        let is_refresh = path == "/refresh" || path == "/_syneroym/session/refresh";

        if method == "POST" && is_challenge {
            let ch = self.issue_challenge();
            let body = serde_json::to_vec(&ch).map_err(|e| e.to_string())?;
            return Ok(HttpResponse { status: 200, headers, body });
        }

        if method == "GET" && is_methods {
            let m = self.methods();
            let body = serde_json::to_vec(&m).map_err(|e| e.to_string())?;
            return Ok(HttpResponse { status: 200, headers, body });
        }

        if method == "POST" && is_login {
            let req: LoginRequest = match serde_json::from_slice(&request.body) {
                Ok(r) => r,
                Err(_) => {
                    let body = serde_json::to_vec(
                        &serde_json::json!({"error": "malformed login request"}),
                    )
                    .map_err(|e| e.to_string())?;
                    return Ok(HttpResponse { status: 400, headers, body });
                }
            };

            let Some(req_method) = req.method else {
                let body = serde_json::to_vec(
                    &serde_json::json!({"error": "missing required method parameter"}),
                )
                .map_err(|e| e.to_string())?;
                return Ok(HttpResponse { status: 400, headers, body });
            };

            let res = match req_method.as_str() {
                AUTH_METHOD_DELEGATED_KEY => {
                    let temp_did = req
                        .temp_did
                        .or_else(|| req.delegation.as_ref().map(|d| d.temporary_did.clone()));
                    let (Some(temp_did), Some(delegation), Some(nonce), Some(signature)) =
                        (temp_did, req.delegation, req.nonce, req.signature)
                    else {
                        let body = serde_json::to_vec(&serde_json::json!({
                            "error": "missing parameters for delegated-key login (expected delegation, nonce, signature)"
                        }))
                        .map_err(|e| e.to_string())?;
                        return Ok(HttpResponse { status: 400, headers, body });
                    };
                    self.login_delegated_key(&DelegatedKeyLoginParams {
                        temp_did,
                        delegation,
                        nonce,
                        signature,
                    })
                    .await
                }
                AUTH_METHOD_LOCAL => {
                    let Some(identity) = req.identity else {
                        let body = serde_json::to_vec(
                            &serde_json::json!({"error": "missing identity parameter for local login"}),
                        )
                        .map_err(|e| e.to_string())?;
                        return Ok(HttpResponse { status: 400, headers, body });
                    };
                    self.login_local(&identity)
                }
                _ => {
                    let body = serde_json::to_vec(
                        &serde_json::json!({"error": format!("unknown login method: {}", req_method)}),
                    )
                    .map_err(|e| e.to_string())?;
                    return Ok(HttpResponse { status: 400, headers, body });
                }
            };

            return match res {
                Ok(grant) => {
                    let now = now_secs();
                    let remaining_ttl = grant.expires_at_secs.saturating_sub(now);
                    let cookie = format!(
                        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
                        SESSION_COOKIE_NAME, grant.token, remaining_ttl
                    );
                    headers.push(("set-cookie".to_string(), cookie));
                    let body = serde_json::to_vec(&grant).map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status: 200, headers, body })
                }
                Err((status, msg)) => {
                    let body = serde_json::to_vec(&serde_json::json!({"error": msg}))
                        .map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status, headers, body })
                }
            };
        }

        if method == "GET" && is_whoami {
            let Some(token) = self.extract_token(&request) else {
                let body =
                    serde_json::to_vec(&serde_json::json!({"error": "no session token provided"}))
                        .map_err(|e| e.to_string())?;
                return Ok(HttpResponse { status: 401, headers, body });
            };

            return match self.whoami(&token) {
                Ok(resp) => {
                    let body = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status: 200, headers, body })
                }
                Err((status, msg)) => {
                    let body = serde_json::to_vec(&serde_json::json!({"error": msg}))
                        .map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status, headers, body })
                }
            };
        }

        if method == "POST" && is_logout {
            if let Some(token) = self.extract_token(&request) {
                self.record_logout(&token);
            }
            let cookie =
                format!("{}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax", SESSION_COOKIE_NAME);
            headers.push(("set-cookie".to_string(), cookie));
            let body = serde_json::to_vec(&serde_json::json!({"status": "ended"}))
                .map_err(|e| e.to_string())?;
            return Ok(HttpResponse { status: 200, headers, body });
        }

        if method == "POST" && is_refresh {
            let Some(token) = self.extract_token(&request) else {
                let body =
                    serde_json::to_vec(&serde_json::json!({"error": "no session token provided"}))
                        .map_err(|e| e.to_string())?;
                return Ok(HttpResponse { status: 401, headers, body });
            };

            return match self.refresh(&token) {
                Ok(grant) => {
                    let now = now_secs();
                    let remaining_ttl = grant.expires_at_secs.saturating_sub(now);
                    let cookie = format!(
                        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
                        SESSION_COOKIE_NAME, grant.token, remaining_ttl
                    );
                    headers.push(("set-cookie".to_string(), cookie));
                    let body = serde_json::to_vec(&grant).map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status: 200, headers, body })
                }
                Err((status, msg)) => {
                    let body = serde_json::to_vec(&serde_json::json!({"error": msg}))
                        .map_err(|e| e.to_string())?;
                    Ok(HttpResponse { status, headers, body })
                }
            };
        }

        let body = serde_json::to_vec(&serde_json::json!({"error": "unknown auth endpoint"}))
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse { status: 404, headers, body })
    }

    async fn on_websocket_open(&self, _conn: String, _caller: Option<CallerContext>) {}
    async fn on_websocket_message(
        &self,
        _conn: String,
        _frame: Vec<u8>,
        _kind: FrameKind,
        _caller: Option<CallerContext>,
    ) {
    }
    async fn on_websocket_close(&self, _conn: String, _caller: Option<CallerContext>) {}

    fn service_id(&self) -> Option<&str> {
        Some(&self.auth_did)
    }
}

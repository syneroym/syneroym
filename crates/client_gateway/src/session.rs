use std::{
    cmp,
    error::Error,
    fmt, str,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use httparse::Header;
use rand::RngCore;
use serde::{Deserialize, Serialize};
pub use syneroym_core::protocol_utils::gateway_session_assertion as assertion_value;
use syneroym_core::{
    dht_registry::MasterAnchorResolver,
    protocol_utils::{GATEWAY_RESERVED_PATH_PREFIX, SESSION_COOKIE_NAME},
};
use syneroym_identity::{DelegationCertificate, delegation::SCOPE_ROUTING, substrate};
use tokio::time;

pub const NONCE_TTL_SECS: u64 = 60;
pub const MAX_PENDING_CHALLENGES: usize = 64;
pub const MAX_ACTIVE_SESSIONS: usize = 64;
pub const MAX_SESSION_BODY_BYTES: usize = 4096;
pub const TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub nonce: String,
    pub node_did: String,
    pub expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub person_did: String,
    pub nonce: String,
    pub signature: String,
    pub delegation: DelegationCertificate,
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
    pub auth: &'static str,
    pub expires_at_secs: u64,
}

#[derive(Debug, Clone)]
pub struct PersonSession {
    pub person_did: String,
    pub delegation: DelegationCertificate,
    pub expires_at_secs: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    UnknownOrUsedNonce,
    ExpiredNonce,
    BadSignature,
    BadDelegation(String),
    WrongDelegate,
    DelegationExpired,
    AnchorUnresolvable,
    TooManySessions,
}

impl SessionError {
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::UnknownOrUsedNonce
            | Self::ExpiredNonce
            | Self::BadSignature
            | Self::BadDelegation(_)
            | Self::WrongDelegate
            | Self::DelegationExpired => 401,
            Self::AnchorUnresolvable => 409,
            Self::TooManySessions => 503,
        }
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::UnknownOrUsedNonce => "unknown or already used nonce",
            Self::ExpiredNonce => "expired nonce",
            Self::BadSignature => "invalid signature",
            Self::BadDelegation(_) => "invalid delegation certificate",
            Self::WrongDelegate => "delegation certificate was not issued to this node",
            Self::DelegationExpired => "delegation certificate has expired",
            Self::AnchorUnresolvable => {
                "person master anchor is not resolvable; publish the anchor first"
            }
            Self::TooManySessions => "too many active sessions",
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl Error for SessionError {}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug)]
pub struct SessionStore {
    node_did: String,
    ttl_secs: u64,
    challenges: DashMap<String, u64>,
    sessions: DashMap<String, PersonSession>,
}

impl SessionStore {
    #[must_use]
    pub fn new(node_did: String, ttl_secs: u64) -> Self {
        Self { node_did, ttl_secs, challenges: DashMap::new(), sessions: DashMap::new() }
    }

    fn sweep(&self, now: u64) {
        self.challenges.retain(|_, expires_at| *expires_at > now);
        self.sessions.retain(|_, session| session.expires_at_secs > now);
    }

    pub fn issue_challenge(&self) -> ChallengeResponse {
        let now = now_secs();
        self.sweep(now);

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

        let mut nonce_bytes = [0u8; TOKEN_BYTES];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = hex::encode(nonce_bytes);
        let expires = now.saturating_add(NONCE_TTL_SECS);
        self.challenges.insert(nonce.clone(), expires);

        ChallengeResponse { nonce, node_did: self.node_did.clone(), expires_at_secs: expires }
    }

    pub async fn login(
        &self,
        req: &LoginRequest,
        anchor_lookup: &dyn MasterAnchorResolver,
    ) -> Result<LoginResponse, SessionError> {
        let now = now_secs();
        self.sessions.retain(|_, session| session.expires_at_secs > now);

        // 1. Single-use nonce. Consumed before verification.
        let expires = self
            .challenges
            .remove(&req.nonce)
            .map(|(_, exp)| exp)
            .ok_or(SessionError::UnknownOrUsedNonce)?;
        if now >= expires {
            return Err(SessionError::ExpiredNonce);
        }

        // 2. Proof of possession of the person's master key.
        let value = assertion_value(&self.node_did, &req.nonce, &req.person_did);
        substrate::verify_json_signature(&req.person_did, &value, &req.signature)
            .map_err(|_| SessionError::BadSignature)?;

        // 3. The authorization: this person delegated THIS node, for routing.
        if req.delegation.expires_at_secs <= now {
            return Err(SessionError::DelegationExpired);
        }
        req.delegation.verify(&req.person_did, &[SCOPE_ROUTING]).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("expired") {
                SessionError::DelegationExpired
            } else {
                SessionError::BadDelegation(msg)
            }
        })?;
        if req.delegation.temporary_did != self.node_did {
            return Err(SessionError::WrongDelegate);
        }

        // 4. The anchor must be resolvable when the session is opened.
        let anchor_res = time::timeout(
            Duration::from_secs(5),
            anchor_lookup.resolve_master_anchor(&req.person_did),
        )
        .await;

        match anchor_res {
            Ok(Ok(_payload)) => {}
            _ => return Err(SessionError::AnchorUnresolvable),
        }

        // 5. Mint.
        if self.sessions.len() >= MAX_ACTIVE_SESSIONS {
            let oldest = self
                .sessions
                .iter()
                .min_by_key(|entry| entry.value().expires_at_secs)
                .map(|entry| entry.key().clone());
            if let Some(key) = oldest {
                self.sessions.remove(&key);
            }
            if self.sessions.len() >= MAX_ACTIVE_SESSIONS {
                return Err(SessionError::TooManySessions);
            }
        }

        let expires_at_secs =
            cmp::min(now.saturating_add(self.ttl_secs), req.delegation.expires_at_secs);
        let mut token_bytes = [0u8; TOKEN_BYTES];
        rand::rng().fill_bytes(&mut token_bytes);
        let token = hex::encode(token_bytes);

        self.sessions.insert(
            token.clone(),
            PersonSession {
                person_did: req.person_did.clone(),
                delegation: req.delegation.clone(),
                expires_at_secs,
            },
        );

        Ok(LoginResponse { token, person_did: req.person_did.clone(), expires_at_secs })
    }

    #[must_use]
    pub fn lookup(&self, token: &str) -> Option<PersonSession> {
        let now = now_secs();
        let entry = self.sessions.get(token)?;
        if entry.value().expires_at_secs <= now {
            drop(entry);
            self.sessions.remove(token);
            return None;
        }
        Some(entry.clone())
    }

    pub fn logout(&self, token: &str) -> bool {
        self.sessions.remove(token).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Cookie,
    Bearer,
}

#[must_use]
pub fn extract_credential(headers: &[Header<'_>]) -> Option<(String, CredentialSource)> {
    // Cookie takes precedence over Authorization: Bearer
    for h in headers {
        if h.name.eq_ignore_ascii_case("cookie") {
            for pair in h.value.split(|&b| b == b';') {
                let trimmed = trim_ascii_whitespace(pair);
                if let Some(pos) = trimmed.iter().position(|&b| b == b'=') {
                    let k = trim_ascii_whitespace(&trimmed[..pos]);
                    if k == SESSION_COOKIE_NAME.as_bytes() {
                        let v = trim_ascii_whitespace(&trimmed[pos + 1..]);
                        if let Ok(token) = str::from_utf8(v)
                            && !token.is_empty()
                        {
                            return Some((token.to_string(), CredentialSource::Cookie));
                        }
                    }
                }
            }
        }
    }

    for h in headers {
        if h.name.eq_ignore_ascii_case("authorization") {
            let val = trim_ascii_whitespace(h.value);
            if val.len() >= 7 && val[..7].eq_ignore_ascii_case(b"bearer ") {
                let bearer_tok = trim_ascii_whitespace(&val[7..]);
                if let Ok(token) = str::from_utf8(bearer_tok)
                    && !token.is_empty()
                {
                    return Some((token.to_string(), CredentialSource::Bearer));
                }
            }
        }
    }

    None
}

#[must_use]
pub fn strip_credential(
    raw: &[u8],
    header_len: usize,
    token: &str,
    source: CredentialSource,
) -> Option<Vec<u8>> {
    let head_bytes = &raw[..header_len];
    let tail_bytes = &raw[header_len..];

    let mut out_head = Vec::with_capacity(head_bytes.len());
    let mut changed = false;

    let mut remaining = head_bytes;
    while !remaining.is_empty() {
        let (line, rest) = match remaining.windows(2).position(|w| w == b"\r\n") {
            Some(pos) => (&remaining[..pos], &remaining[pos + 2..]),
            None => (remaining, &[][..]),
        };
        remaining = rest;

        if line.is_empty() {
            break;
        }

        let is_cookie = line.len() >= 7 && line[..7].eq_ignore_ascii_case(b"cookie:");
        let is_auth = line.len() >= 14 && line[..14].eq_ignore_ascii_case(b"authorization:");

        if is_cookie {
            let value = &line[7..];
            let mut remaining_pairs = Vec::new();
            for pair in value.split(|&b| b == b';') {
                let trimmed = trim_ascii_whitespace(pair);
                if let Some(pos) = trimmed.iter().position(|&b| b == b'=') {
                    let key = trim_ascii_whitespace(&trimmed[..pos]);
                    if key == SESSION_COOKIE_NAME.as_bytes() {
                        changed = true;
                        continue;
                    }
                }
                if !trimmed.is_empty() {
                    remaining_pairs.push(trimmed);
                }
            }

            if remaining_pairs.is_empty() {
                changed = true;
            } else {
                out_head.extend_from_slice(b"Cookie: ");
                for (i, p) in remaining_pairs.iter().enumerate() {
                    if i > 0 {
                        out_head.extend_from_slice(b"; ");
                    }
                    out_head.extend_from_slice(p);
                }
                out_head.extend_from_slice(b"\r\n");
            }
        } else if is_auth {
            let value = trim_ascii_whitespace(&line[14..]);
            if value.len() >= 7 && value[..7].eq_ignore_ascii_case(b"bearer ") {
                let bearer_tok = trim_ascii_whitespace(&value[7..]);
                if source == CredentialSource::Bearer && bearer_tok == token.as_bytes() {
                    changed = true;
                    continue;
                }
            }
            out_head.extend_from_slice(line);
            out_head.extend_from_slice(b"\r\n");
        } else {
            out_head.extend_from_slice(line);
            out_head.extend_from_slice(b"\r\n");
        }
    }

    if !changed {
        return None;
    }

    out_head.extend_from_slice(b"\r\n");
    let mut result = Vec::with_capacity(out_head.len() + tail_bytes.len());
    result.extend_from_slice(&out_head);
    result.extend_from_slice(tail_bytes);
    Some(result)
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    &bytes[start..end]
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestKind {
    Session(SessionRoute),
    Proxy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionRoute {
    Challenge,
    Login,
    Logout,
    Whoami,
    Unknown,
}

#[must_use]
pub fn classify(method: &str, path: &str) -> RequestKind {
    let mut clean_path = path;
    if let Some(idx) = clean_path.find("://") {
        if let Some(slash_idx) = clean_path[idx + 3..].find('/') {
            clean_path = &clean_path[idx + 3 + slash_idx..];
        } else {
            clean_path = "/";
        }
    }
    let path_no_query = clean_path.split('?').next().unwrap_or(clean_path);
    if !path_no_query.starts_with(GATEWAY_RESERVED_PATH_PREFIX) {
        return RequestKind::Proxy;
    }
    match (method, path_no_query) {
        ("POST", "/_syneroym/session/challenge") => RequestKind::Session(SessionRoute::Challenge),
        ("POST", "/_syneroym/session/login") => RequestKind::Session(SessionRoute::Login),
        ("POST", "/_syneroym/session/logout") => RequestKind::Session(SessionRoute::Logout),
        ("GET", "/_syneroym/session/whoami") => RequestKind::Session(SessionRoute::Whoami),
        _ => RequestKind::Session(SessionRoute::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use syneroym_core::dht_registry::MasterAnchorPayload;
    use syneroym_identity::Identity;

    use super::*;

    #[derive(Debug)]
    struct OkAnchorResolver;

    #[async_trait::async_trait]
    impl MasterAnchorResolver for OkAnchorResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> anyhow::Result<MasterAnchorPayload> {
            Ok(MasterAnchorPayload::default())
        }
    }

    #[derive(Debug)]
    struct ErrAnchorResolver;

    #[async_trait::async_trait]
    impl MasterAnchorResolver for ErrAnchorResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> anyhow::Result<MasterAnchorPayload> {
            Err(anyhow::anyhow!("Master anchor not found"))
        }
    }

    fn setup_test_actors() -> (Identity, Identity, String, String) {
        let person = Identity::generate().unwrap();
        let node = Identity::generate().unwrap();
        let person_did = substrate::derive_did_key(&person.public_key());
        let node_did = substrate::derive_did_key(&node.public_key());
        (person, node, person_did, node_did)
    }

    // 1. challenge -> sign -> login -> lookup returns the person's DID and cert.
    #[tokio::test]
    async fn test_session_login_lookup_success() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        assert_eq!(ch.node_did, node_did);

        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest {
            person_did: person_did.clone(),
            nonce: ch.nonce,
            signature: sig,
            delegation: cert.clone(),
        };

        let resolver = OkAnchorResolver;
        let login_res = store.login(&req, &resolver).await.unwrap();
        assert_eq!(login_res.person_did, person_did);

        let session = store.lookup(&login_res.token).expect("session found");
        assert_eq!(session.person_did, person_did);
        assert_eq!(session.delegation.master_did, cert.master_did);
    }

    // 2. a signature made by a different key, claiming Alice's DID -> BadSignature.
    #[tokio::test]
    async fn test_session_bad_signature() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let attacker = Identity::generate().unwrap();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = attacker.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::BadSignature);
    }

    // 3. the same nonce used twice -> second attempt UnknownOrUsedNonce.
    #[tokio::test]
    async fn test_session_nonce_single_use() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        store.login(&req, &resolver).await.unwrap();

        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::UnknownOrUsedNonce);
    }

    // 4. a nonce older than NONCE_TTL_SECS -> ExpiredNonce.
    #[tokio::test]
    async fn test_session_expired_nonce() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        store.challenges.insert(ch.nonce.clone(), now_secs() - 1);

        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::ExpiredNonce);
    }

    // 5. a certificate delegating to some other node's DID -> WrongDelegate.
    #[tokio::test]
    async fn test_session_wrong_delegate() {
        let (person, _node, person_did, node_did) = setup_test_actors();
        let other_node = Identity::generate().unwrap();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            other_node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::WrongDelegate);
    }

    // 6. a certificate whose master_did is not the claimed person_did ->
    //    BadDelegation.
    #[tokio::test]
    async fn test_session_bad_delegation_mismatched_master() {
        let (_person, node, _person_did, node_did) = setup_test_actors();
        let bob = Identity::generate().unwrap();
        let alice = Identity::generate().unwrap();
        let alice_did = substrate::derive_did_key(&alice.public_key());
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert =
            DelegationCertificate::issue(&bob, node.public_key(), 3600, SCOPE_ROUTING.to_string())
                .unwrap();
        let sig = alice.sign_json(&assertion_value(&node_did, &ch.nonce, &alice_did)).unwrap();

        let req = LoginRequest {
            person_did: alice_did,
            nonce: ch.nonce,
            signature: sig,
            delegation: cert,
        };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert!(matches!(err, SessionError::BadDelegation(_)));
    }

    // 7. a certificate with scope = "service-instance" -> BadDelegation.
    #[tokio::test]
    async fn test_session_bad_delegation_wrong_scope() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            "service-instance".to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert!(matches!(err, SessionError::BadDelegation(_)));
    }

    // 8. an already-expired certificate -> DelegationExpired.
    #[tokio::test]
    async fn test_session_delegation_expired() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        // Issue a certificate with 1s expiry and wait until it is expired,
        // so the signature remains valid and the expiry handling is truly verified.
        let cert =
            DelegationCertificate::issue(&person, node.public_key(), 1, SCOPE_ROUTING.to_string())
                .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = OkAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::DelegationExpired);
    }

    // 8a. an unresolvable master anchor -> AnchorUnresolvable (409) and no session
    // created.
    #[tokio::test]
    async fn test_session_anchor_unresolvable() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let req = LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert };

        let resolver = ErrAnchorResolver;
        let err = store.login(&req, &resolver).await.unwrap_err();
        assert_eq!(err, SessionError::AnchorUnresolvable);
        assert_eq!(err.http_status(), 409);
        assert_eq!(store.sessions.len(), 0);
    }

    // 9. expires_at is min(ttl, cert expiry).
    #[tokio::test]
    async fn test_session_expiry_clamped_to_min() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let resolver = OkAnchorResolver;

        let store_a = SessionStore::new(node_did.clone(), 3600);
        let ch_a = store_a.issue_challenge();
        let cert_a = DelegationCertificate::issue(
            &person,
            node.public_key(),
            600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig_a =
            person.sign_json(&assertion_value(&node_did, &ch_a.nonce, &person_did)).unwrap();
        let res_a = store_a
            .login(
                &LoginRequest {
                    person_did: person_did.clone(),
                    nonce: ch_a.nonce,
                    signature: sig_a,
                    delegation: cert_a.clone(),
                },
                &resolver,
            )
            .await
            .unwrap();
        assert_eq!(res_a.expires_at_secs, cert_a.expires_at_secs);

        let store_b = SessionStore::new(node_did.clone(), 300);
        let ch_b = store_b.issue_challenge();
        let cert_b = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig_b =
            person.sign_json(&assertion_value(&node_did, &ch_b.nonce, &person_did)).unwrap();
        let now = now_secs();
        let res_b = store_b
            .login(
                &LoginRequest {
                    person_did: person_did.clone(),
                    nonce: ch_b.nonce,
                    signature: sig_b,
                    delegation: cert_b,
                },
                &resolver,
            )
            .await
            .unwrap();
        assert!(res_b.expires_at_secs <= now + 300 + 1);
        assert!(res_b.expires_at_secs >= now + 300 - 1);
    }

    // 10. lookup of a token past its expiry -> None and entry removed.
    #[tokio::test]
    async fn test_session_lookup_expired() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();

        let resolver = OkAnchorResolver;
        let res = store
            .login(
                &LoginRequest { person_did, nonce: ch.nonce, signature: sig, delegation: cert },
                &resolver,
            )
            .await
            .unwrap();

        if let Some(mut session) = store.sessions.get_mut(&res.token) {
            session.expires_at_secs = now_secs() - 1;
        }

        assert!(store.lookup(&res.token).is_none());
        assert!(!store.sessions.contains_key(&res.token));
    }

    // 11. MAX_PENDING_CHALLENGES + 10 challenges -> map never exceeds cap.
    #[test]
    fn test_max_pending_challenges_bound() {
        let store = SessionStore::new("did:key:zNode".to_string(), 3600);
        for _ in 0..(MAX_PENDING_CHALLENGES + 10) {
            store.issue_challenge();
        }
        assert!(store.challenges.len() <= MAX_PENDING_CHALLENGES);
    }

    // 12. MAX_ACTIVE_SESSIONS reached -> oldest evicted, newest usable.
    #[tokio::test]
    async fn test_max_active_sessions_eviction() {
        let (person, node, person_did, node_did) = setup_test_actors();
        let store = SessionStore::new(node_did.clone(), 3600);
        let resolver = OkAnchorResolver;

        let mut first_token = String::new();
        for i in 0..MAX_ACTIVE_SESSIONS {
            let ch = store.issue_challenge();
            let cert = DelegationCertificate::issue(
                &person,
                node.public_key(),
                3600,
                SCOPE_ROUTING.to_string(),
            )
            .unwrap();
            let sig =
                person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();
            let res = store
                .login(
                    &LoginRequest {
                        person_did: person_did.clone(),
                        nonce: ch.nonce,
                        signature: sig,
                        delegation: cert,
                    },
                    &resolver,
                )
                .await
                .unwrap();
            if i == 0 {
                first_token = res.token.clone();
                if let Some(mut sess) = store.sessions.get_mut(&first_token) {
                    sess.expires_at_secs = now_secs() + 10;
                }
            }
        }

        assert_eq!(store.sessions.len(), MAX_ACTIVE_SESSIONS);

        let ch = store.issue_challenge();
        let cert = DelegationCertificate::issue(
            &person,
            node.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();
        let sig = person.sign_json(&assertion_value(&node_did, &ch.nonce, &person_did)).unwrap();
        let newest_res = store
            .login(
                &LoginRequest {
                    person_did: person_did.clone(),
                    nonce: ch.nonce,
                    signature: sig,
                    delegation: cert,
                },
                &resolver,
            )
            .await
            .unwrap();

        assert_eq!(store.sessions.len(), MAX_ACTIVE_SESSIONS);
        assert!(store.lookup(&first_token).is_none());
        assert!(store.lookup(&newest_res.token).is_some());
    }

    // 13. extract_credential tests.
    #[test]
    fn test_extract_credential() {
        let headers_cookie = [httparse::Header {
            name: "Cookie",
            value: b"foo=bar; syneroym_session=tok123; baz=qux",
        }];
        assert_eq!(
            extract_credential(&headers_cookie),
            Some(("tok123".to_string(), CredentialSource::Cookie))
        );

        let headers_bearer = [httparse::Header { name: "Authorization", value: b"Bearer tok456" }];
        assert_eq!(
            extract_credential(&headers_bearer),
            Some(("tok456".to_string(), CredentialSource::Bearer))
        );

        // Cookie takes priority over Bearer when both are present
        let headers_both = [
            httparse::Header { name: "Authorization", value: b"Bearer tok456" },
            httparse::Header { name: "Cookie", value: b"syneroym_session=tok123" },
        ];
        assert_eq!(
            extract_credential(&headers_both),
            Some(("tok123".to_string(), CredentialSource::Cookie))
        );

        let headers_basic =
            [httparse::Header { name: "Authorization", value: b"Basic dXNlcjpwYXNz" }];
        assert_eq!(extract_credential(&headers_basic), None);

        // Multi-byte UTF-8 in Authorization header does not panic
        let headers_utf8 =
            [httparse::Header { name: "Authorization", value: "€€€".as_bytes() }];
        assert_eq!(extract_credential(&headers_utf8), None);

        // Invalid non-UTF8 byte sequence does not panic
        let headers_invalid_utf8 =
            [httparse::Header { name: "Authorization", value: b"\xff\xfe\xfd" }];
        assert_eq!(extract_credential(&headers_invalid_utf8), None);

        // Empty cookie value does not shadow a valid Authorization header
        let headers_empty_cookie = [
            httparse::Header { name: "Cookie", value: b"syneroym_session=" },
            httparse::Header { name: "Authorization", value: b"Bearer tok456" },
        ];
        assert_eq!(
            extract_credential(&headers_empty_cookie),
            Some(("tok456".to_string(), CredentialSource::Bearer))
        );

        // Empty bearer token returns None
        let headers_empty_bearer = [httparse::Header { name: "Authorization", value: b"Bearer " }];
        assert_eq!(extract_credential(&headers_empty_bearer), None);

        let headers_empty: [httparse::Header; 0] = [];
        assert_eq!(extract_credential(&headers_empty), None);
    }

    // 14. strip_credential tests.
    #[test]
    fn test_strip_credential() {
        let raw = b"GET / HTTP/1.1\r\nHost: localhost\r\nCookie: a=1; syneroym_session=tok; b=2\r\n\r\nbody";
        let header_len = raw.len() - 4;
        let stripped = strip_credential(raw, header_len, "tok", CredentialSource::Cookie).unwrap();
        assert_eq!(
            str::from_utf8(&stripped).unwrap(),
            "GET / HTTP/1.1\r\nHost: localhost\r\nCookie: a=1; b=2\r\n\r\nbody"
        );

        let raw2 = b"GET / HTTP/1.1\r\nHost: localhost\r\nCookie: syneroym_session=tok\r\n\r\nbody";
        let header_len2 = raw2.len() - 4;
        let stripped2 =
            strip_credential(raw2, header_len2, "tok", CredentialSource::Cookie).unwrap();
        assert_eq!(
            str::from_utf8(&stripped2).unwrap(),
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\nbody"
        );

        let raw3 = b"GET / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer tok\r\n\r\nbody";
        let header_len3 = raw3.len() - 4;
        let stripped3 =
            strip_credential(raw3, header_len3, "tok", CredentialSource::Bearer).unwrap();
        assert_eq!(
            str::from_utf8(&stripped3).unwrap(),
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\nbody"
        );

        // When source is Cookie, unrelated Authorization header is preserved
        let raw_both = b"GET / HTTP/1.1\r\nHost: localhost\r\nCookie: syneroym_session=tok\r\nAuthorization: Bearer app_token\r\n\r\nbody";
        let header_len_both = raw_both.len() - 4;
        let stripped_both =
            strip_credential(raw_both, header_len_both, "tok", CredentialSource::Cookie).unwrap();
        assert_eq!(
            str::from_utf8(&stripped_both).unwrap(),
            "GET / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer app_token\r\n\r\nbody"
        );

        // When source is Bearer, cookie is also stripped unconditionally
        let raw_both_bearer = b"GET / HTTP/1.1\r\nHost: localhost\r\nCookie: syneroym_session=tok1\r\nAuthorization: Bearer tok2\r\n\r\nbody";
        let header_len_both_bearer = raw_both_bearer.len() - 4;
        let stripped_both_bearer = strip_credential(
            raw_both_bearer,
            header_len_both_bearer,
            "tok2",
            CredentialSource::Bearer,
        )
        .unwrap();
        assert_eq!(
            str::from_utf8(&stripped_both_bearer).unwrap(),
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\nbody"
        );

        let raw4 = b"GET / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer other\r\n\r\nbody";
        let header_len4 = raw4.len() - 4;
        assert_eq!(strip_credential(raw4, header_len4, "tok", CredentialSource::Bearer), None);

        // Multi-byte UTF-8 in headers does not panic during stripping
        let raw_utf8 =
            "GET /€ HTTP/1.1\r\nHost: localhost\r\nAuthorization: €€€\r\n\r\nbody".as_bytes();
        let header_len_utf8 = raw_utf8.len() - 4;
        assert_eq!(
            strip_credential(raw_utf8, header_len_utf8, "tok", CredentialSource::Bearer),
            None
        );

        // Request head with invalid non-UTF-8 bytes strips session cookie and preserves
        // raw bytes
        let raw_invalid_utf8 = b"GET / HTTP/1.1\r\nHost: localhost\r\nCookie: syneroym_session=tok\r\nX-Custom: \xff\xfe\r\n\r\nbody";
        let header_len_invalid = raw_invalid_utf8.len() - 4;
        let stripped_invalid =
            strip_credential(raw_invalid_utf8, header_len_invalid, "tok", CredentialSource::Cookie)
                .expect("must strip cookie even with non-UTF8 header bytes present");
        assert_eq!(
            stripped_invalid,
            b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Custom: \xff\xfe\r\n\r\nbody"
        );
    }

    // 15. classify tests.
    #[test]
    fn test_classify() {
        assert_eq!(
            classify("POST", "/_syneroym/session/challenge"),
            RequestKind::Session(SessionRoute::Challenge)
        );
        assert_eq!(
            classify("POST", "/_syneroym/session/login"),
            RequestKind::Session(SessionRoute::Login)
        );
        assert_eq!(
            classify("POST", "/_syneroym/session/logout"),
            RequestKind::Session(SessionRoute::Logout)
        );
        assert_eq!(
            classify("GET", "/_syneroym/session/whoami"),
            RequestKind::Session(SessionRoute::Whoami)
        );
        assert_eq!(
            classify("GET", "/_syneroym/session/whoami?query=1"),
            RequestKind::Session(SessionRoute::Whoami)
        );
        assert_eq!(
            classify("GET", "/_syneroym/other"),
            RequestKind::Session(SessionRoute::Unknown)
        );
        assert_eq!(classify("GET", "/index.html"), RequestKind::Proxy);
    }
}

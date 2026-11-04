use std::{
    fs,
    path::{Path, PathBuf},
    str,
};

use serde_json::{Map, Value};
use syneroym_app_host::types::http::HttpRequest;
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::CallerContext;

use super::{AuthService, LoginResponse, now_secs};
use crate::token::{AUTH_METHOD_LOCAL, SessionToken};

/// Request headers that a forwarding proxy adds. Their presence means the
/// request may come from another machine.
const PROXY_HEADERS: [&str; 4] = ["forwarded", "x-forwarded-for", "x-real-ip", "via"];

type Refusal = (u16, &'static str);

/// Whether the `local` login method answers.
#[derive(Debug)]
pub(super) enum LocalLogin {
    /// No `person_identities_dir` is set.
    NotConfigured,
    /// A directory is set, but remote access is on.
    OffForRemoteAccess,
    /// Keys are read from this directory.
    On(PathBuf),
}

impl LocalLogin {
    pub(super) fn from_dir(dir: Option<PathBuf>) -> Self {
        dir.map_or(Self::NotConfigured, Self::On)
    }
}

impl AuthService {
    /// Turns `local` login off when remote access is on. The node reads the
    /// key itself, so `local` is only safe for a program on this machine.
    #[must_use]
    pub fn with_remote_access(mut self, on: bool) -> Self {
        if on && matches!(self.local_login, LocalLogin::On(_)) {
            self.local_login = LocalLogin::OffForRemoteAccess;
        }
        self
    }

    /// Decides whether this request may use `local` login. The node reads the
    /// key itself, so the request must come from a program on this machine:
    /// through this node's own gateway, which binds loopback when remote
    /// access is off, and with no sign of a proxy in between. A header never
    /// allows a login; it can only refuse one.
    pub(super) fn local_login_gate(
        &self,
        request: &HttpRequest,
        caller: Option<&CallerContext>,
    ) -> Result<&Path, Refusal> {
        let dir = match &self.local_login {
            LocalLogin::NotConfigured => {
                return Err((400, "local login method is not enabled; use delegated-key"));
            }
            LocalLogin::OffForRemoteAccess => {
                return Err((
                    403,
                    "local login is off because remote access is on; use delegated-key",
                ));
            }
            LocalLogin::On(dir) => dir,
        };
        if has_header(request, "origin") {
            return Err((
                403,
                "local login method is not allowed from a browser origin; use delegated-key",
            ));
        }
        if PROXY_HEADERS.iter().any(|name| has_header(request, name)) {
            return Err((403, "local login is not allowed through a proxy; use delegated-key"));
        }
        // A stream from another node, or one that carries a person (a `fixed`
        // gateway, or a valid session), has a caller other than the node.
        if !caller.is_some_and(|c| c.caller_did == self.node_did) {
            return Err((
                403,
                "local login works only through this node's own gateway in open or login mode; \
                 use delegated-key",
            ));
        }
        Ok(dir)
    }

    /// Mints a session for a key file in `identities_dir`. Only
    /// `handle_request` calls it, after `local_login_gate`.
    pub(crate) fn login_local(
        &self,
        identities_dir: &Path,
        identity_name: &str,
    ) -> Result<LoginResponse, Refusal> {
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
                } else if let Ok(s) = str::from_utf8(&bytes) {
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
}

fn has_header(request: &HttpRequest, name: &str) -> bool {
    request.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests;

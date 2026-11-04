use std::fs;

use serde_json::{Map, Value};
use syneroym_identity::{Identity, substrate};

use super::{AuthService, LoginResponse, now_secs};
use crate::token::{AUTH_METHOD_LOCAL, SessionToken};

impl AuthService {
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
}

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_identity::{Identity, substrate};
use syneroym_ucan::CapabilityToken;

/// Standard claim name for the authentication method.
pub const CLAIM_AUTH_METHOD: &str = "auth_method";
pub const AUTH_METHOD_DELEGATED_KEY: &str = "delegated-key";
pub const AUTH_METHOD_LOCAL: &str = "local";
pub const AUTH_METHOD_FIXED: &str = "fixed";

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Parsed and verified session token claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTokenClaims {
    pub person_did: String,
    pub issuer_did: String,
    pub facts: Map<String, Value>,
    pub not_before_secs: u64,
    pub expires_at_secs: u64,
}

impl SessionTokenClaims {
    #[must_use]
    pub fn auth_method(&self) -> Option<&str> {
        self.facts.get(CLAIM_AUTH_METHOD).and_then(|v| v.as_str())
    }
}

/// A person session token issued and signed by the node auth service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToken {
    inner: CapabilityToken,
}

impl SessionToken {
    /// Mint a new session token bound to a person's master DID.
    pub fn mint(
        auth_identity: &Identity,
        person_did: &str,
        auth_method: &str,
        mut additional_facts: Option<Map<String, Value>>,
        expires_in_secs: u64,
    ) -> Result<Self> {
        let mut facts = Map::new();
        facts.insert(CLAIM_AUTH_METHOD.to_string(), Value::String(auth_method.to_string()));
        if let Some(extra) = additional_facts.take() {
            for (k, v) in extra {
                facts.insert(k, v);
            }
        }

        // Session tokens have empty capabilities and empty proofs (they attest to who,
        // not what).
        let inner = CapabilityToken::issue(
            auth_identity,
            person_did,
            vec![],
            facts,
            expires_in_secs,
            vec![],
        )?;
        Ok(Self { inner })
    }

    /// Returns the underlying `CapabilityToken`.
    #[must_use]
    pub fn into_inner(self) -> CapabilityToken {
        self.inner
    }

    /// Returns a reference to the underlying `CapabilityToken`.
    #[must_use]
    pub fn as_capability_token(&self) -> &CapabilityToken {
        &self.inner
    }

    /// Serialize the session token to a compact, URL-safe string.
    pub fn to_token_string(&self) -> Result<String> {
        let json_bytes = serde_json::to_vec(&self.inner)?;
        Ok(z32::encode(&json_bytes))
    }

    /// Parse and verify a session token string against the expected auth
    /// service DID.
    pub fn verify(token_str: &str, expected_auth_did: &str) -> Result<SessionTokenClaims> {
        let token = Self::decode_token(token_str)?;

        if token.issuer_did != expected_auth_did {
            return Err(anyhow!(
                "session token issuer '{}' does not match expected auth service '{}'",
                token.issuer_did,
                expected_auth_did
            ));
        }

        if !token.capabilities.is_empty() {
            return Err(anyhow!("session token must have empty capabilities"));
        }

        if !token.proofs.is_empty() {
            return Err(anyhow!("session token must have empty proofs"));
        }

        let now = now_secs();
        if token.not_before_secs > now + 300 {
            return Err(anyhow!("session token not_before is in the future"));
        }
        if now >= token.expires_at_secs {
            return Err(anyhow!("session token has expired"));
        }

        substrate::verify_json_signature(
            &token.issuer_did,
            &token_signing_value(&token),
            &token.signature,
        )
        .context("session token signature verification failed")?;

        Ok(SessionTokenClaims {
            person_did: token.audience_did,
            issuer_did: token.issuer_did,
            facts: token.facts,
            not_before_secs: token.not_before_secs,
            expires_at_secs: token.expires_at_secs,
        })
    }

    /// Parse and structurally verify a session token string against its issuer
    /// signature.
    pub fn verify_any_issuer(token_str: &str) -> Result<SessionTokenClaims> {
        let token = Self::decode_token(token_str)?;

        if !token.capabilities.is_empty() {
            return Err(anyhow!("session token must have empty capabilities"));
        }

        if !token.proofs.is_empty() {
            return Err(anyhow!("session token must have empty proofs"));
        }

        let now = now_secs();
        if token.not_before_secs > now + 300 {
            return Err(anyhow!("session token not_before is in the future"));
        }
        if now >= token.expires_at_secs {
            return Err(anyhow!("session token has expired"));
        }

        substrate::verify_json_signature(
            &token.issuer_did,
            &token_signing_value(&token),
            &token.signature,
        )
        .context("session token signature verification failed")?;

        Ok(SessionTokenClaims {
            person_did: token.audience_did,
            issuer_did: token.issuer_did,
            facts: token.facts,
            not_before_secs: token.not_before_secs,
            expires_at_secs: token.expires_at_secs,
        })
    }

    fn decode_token(raw: &str) -> Result<CapabilityToken> {
        let trimmed = raw.trim();
        // Try z32 decode first
        if let Ok(bytes) = z32::decode(trimmed.as_bytes())
            && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&bytes)
        {
            return Ok(token);
        }
        // Try direct JSON deserialize
        if let Ok(token) = serde_json::from_str::<CapabilityToken>(trimmed) {
            return Ok(token);
        }
        // Try hex decode
        if let Ok(bytes) = hex::decode(trimmed)
            && let Ok(token) = serde_json::from_slice::<CapabilityToken>(&bytes)
        {
            return Ok(token);
        }
        Err(anyhow!("malformed session token string"))
    }
}

fn token_signing_value(token: &CapabilityToken) -> Value {
    serde_json::json!({
        "issuer_did": token.issuer_did,
        "audience_did": token.audience_did,
        "anchor_did": token.anchor_did,
        "capabilities": token.capabilities,
        "facts": token.facts,
        "not_before_secs": token.not_before_secs,
        "expires_at_secs": token.expires_at_secs,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_verify_session_token_succeeds() {
        let auth_id = Identity::generate().unwrap();
        let auth_did = substrate::derive_did_key(&auth_id.public_key());
        let person_id = Identity::generate().unwrap();
        let person_did = substrate::derive_did_key(&person_id.public_key());

        let token =
            SessionToken::mint(&auth_id, &person_did, AUTH_METHOD_DELEGATED_KEY, None, 3600)
                .unwrap();
        let encoded = token.to_token_string().unwrap();

        let claims = SessionToken::verify(&encoded, &auth_did).unwrap();
        assert_eq!(claims.person_did, person_did);
        assert_eq!(claims.issuer_did, auth_did);
        assert_eq!(claims.auth_method(), Some(AUTH_METHOD_DELEGATED_KEY));
    }

    #[test]
    fn verify_refuses_wrong_issuer() {
        let auth_id = Identity::generate().unwrap();
        let other_id = Identity::generate().unwrap();
        let other_did = substrate::derive_did_key(&other_id.public_key());
        let person_id = Identity::generate().unwrap();
        let person_did = substrate::derive_did_key(&person_id.public_key());

        let token =
            SessionToken::mint(&auth_id, &person_did, AUTH_METHOD_DELEGATED_KEY, None, 3600)
                .unwrap();
        let encoded = token.to_token_string().unwrap();

        let err = SessionToken::verify(&encoded, &other_did).unwrap_err();
        assert!(err.to_string().contains("does not match expected auth service"));
    }

    #[test]
    fn verify_refuses_expired_token() {
        let auth_id = Identity::generate().unwrap();
        let auth_did = substrate::derive_did_key(&auth_id.public_key());
        let person_id = Identity::generate().unwrap();
        let person_did = substrate::derive_did_key(&person_id.public_key());

        let token = SessionToken::mint(&auth_id, &person_did, AUTH_METHOD_LOCAL, None, 0).unwrap();
        let encoded = token.to_token_string().unwrap();

        let err = SessionToken::verify(&encoded, &auth_did).unwrap_err();
        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn verify_refuses_token_with_capabilities() {
        use syneroym_ucan::{Ability, Capability, ResourceUri};

        let auth_id = Identity::generate().unwrap();
        let auth_did = substrate::derive_did_key(&auth_id.public_key());
        let person_id = Identity::generate().unwrap();
        let person_did = substrate::derive_did_key(&person_id.public_key());

        let cap = Capability {
            with: ResourceUri::substrate("test"),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        };
        let bad_token =
            CapabilityToken::issue(&auth_id, &person_did, vec![cap], Map::new(), 3600, vec![])
                .unwrap();
        let json_bytes = serde_json::to_vec(&bad_token).unwrap();
        let encoded = z32::encode(&json_bytes);

        let err = SessionToken::verify(&encoded, &auth_did).unwrap_err();
        assert!(err.to_string().contains("empty capabilities"));
    }
}

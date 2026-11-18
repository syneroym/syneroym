use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

pub const SALT_BYTES: usize = 16;
pub const SALT_CHARS: usize = 22;
pub const DIGEST_CHARS: usize = 43;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Salt([u8; SALT_BYTES]);

impl Salt {
    pub fn from_bytes(bytes: [u8; SALT_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, DisclosureError> {
        if bytes.len() != SALT_BYTES {
            return Err(DisclosureError::SaltLength(bytes.len()));
        }
        let mut arr = [0u8; SALT_BYTES];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    pub fn as_bytes(&self) -> &[u8; SALT_BYTES] {
        &self.0
    }

    pub fn encoded(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisclosureError {
    #[error("disclosure is not valid unpadded base64url")]
    NotBase64Url,
    #[error("disclosure payload is not valid JSON")]
    NotJson,
    #[error("disclosure must be a JSON array of 3 elements, found {0}")]
    NotThreeElements(usize),
    #[error("disclosure salt is not a string")]
    SaltNotString,
    #[error("disclosure name is not a string")]
    NameNotString,
    #[error("disclosure value contains a non-integer number")]
    NonIntegerNumber,
    #[error("salt length must be 16 bytes, found {0}")]
    SaltLength(usize),
}

/// `base64url(compact JSON [salt, name, value])`.
pub fn encode_disclosure(
    salt: &Salt,
    name: &str,
    value: &Value,
) -> Result<String, DisclosureError> {
    crate::envelope::walk_integers(value, 1, "").map_err(|_| DisclosureError::NonIntegerNumber)?;
    let tuple = (salt.encoded(), name, value);
    let json = serde_json::to_string(&tuple).map_err(|_| DisclosureError::NotJson)?;
    Ok(URL_SAFE_NO_PAD.encode(json.as_bytes()))
}

/// `base64url(SHA-256(ASCII bytes of `disclosure`))`, 43 chars.
pub fn disclosure_digest(disclosure: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(disclosure.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

/// Test salt `n` for `seed`: the first 16 bytes of
/// SHA-256(seed as 8 LE bytes ‖ n as 8 LE bytes). The one copy of this
/// rule; the host's seeded signer and the Roym test host both call it.
pub fn seeded_salt(seed: u64, n: u64) -> Salt {
    let mut hasher = Sha256::new();
    hasher.update(seed.to_le_bytes());
    hasher.update(n.to_le_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0u8; SALT_BYTES];
    bytes.copy_from_slice(&hash[..SALT_BYTES]);
    Salt::from_bytes(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedDisclosure {
    pub salt: String,
    pub name: String,
    pub value: Value,
    pub digest: String,
}

/// Accepts any JSON spacing (the RFC's own example has spaces). Refuses:
/// not base64url without padding, not a JSON array of exactly 3, a salt or
/// name that is not a string, any number in `value` that is not an integer.
pub fn decode_disclosure(disclosure: &str) -> Result<DecodedDisclosure, DisclosureError> {
    let raw =
        URL_SAFE_NO_PAD.decode(disclosure.as_bytes()).map_err(|_| DisclosureError::NotBase64Url)?;
    let val: Value = serde_json::from_slice(&raw).map_err(|_| DisclosureError::NotJson)?;
    let arr = match val {
        Value::Array(a) => a,
        _ => return Err(DisclosureError::NotThreeElements(0)),
    };
    let [salt_val, name_val, value] = match <[Value; 3]>::try_from(arr) {
        Ok(a) => a,
        Err(v) => return Err(DisclosureError::NotThreeElements(v.len())),
    };

    let salt = match salt_val {
        Value::String(s) => s,
        _ => return Err(DisclosureError::SaltNotString),
    };
    let name = match name_val {
        Value::String(s) => s,
        _ => return Err(DisclosureError::NameNotString),
    };

    crate::envelope::walk_integers(&value, 1, "").map_err(|_| DisclosureError::NonIntegerNumber)?;

    let digest = disclosure_digest(disclosure);
    Ok(DecodedDisclosure { salt, name, value, digest })
}

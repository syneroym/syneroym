//! The outer part of a deposit: what the mailbox reads and stores.

use std::{fmt, str::FromStr};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use syneroym_identity::substrate;

use crate::{MAX_SEALED_BYTES, crypto::SEAL_OVERHEAD, error::DepositError};

/// A random id the sender picks for one deposit. It is not the idempotency
/// key: Conversation keys come from the content, and a mailbox could then
/// confirm a guessed message by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct DepositId([u8; 16]);

/// The sealed inner part, base64 on the wire.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct SealedBytes(Vec<u8>);

/// What a sender gives the mailbox. The sender is not a field: the mailbox
/// takes it from the verified transport identity (ADR-0016). Used once:
/// a second deposit of the same call is a new `seal`.
///
/// Unknown fields are ignored, not refused: a mailbox reads deposits from
/// senders that may run a newer version, and every field it acts on is
/// checked by `check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deposit {
    /// Member master DID of the receiving service.
    pub destination: String,
    pub deposit_id: DepositId,
    pub ttl_secs: u64,
    pub sealed: SealedBytes,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DepositIdParseError {
    #[error("a deposit id is 32 lowercase hex characters")]
    Invalid,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealedBytesParseError {
    #[error("sealed bytes are base64 text")]
    NotBase64,
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
}

impl DepositId {
    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for DepositId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl FromStr for DepositId {
    type Err = DepositIdParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // `hex` accepts upper case. One id has one text form, so refuse it.
        if text.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(DepositIdParseError::Invalid);
        }
        let mut bytes = [0u8; 16];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| DepositIdParseError::Invalid)?;
        Ok(Self(bytes))
    }
}

impl From<DepositId> for String {
    fn from(id: DepositId) -> Self {
        id.to_string()
    }
}

impl TryFrom<String> for DepositId {
    type Error = DepositIdParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl SealedBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// The bytes are ciphertext, but a log line with 64 KiB of them helps nobody.
impl fmt::Debug for SealedBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealedBytes({} bytes)", self.0.len())
    }
}

impl From<SealedBytes> for String {
    fn from(sealed: SealedBytes) -> Self {
        STANDARD.encode(sealed.0)
    }
}

impl TryFrom<String> for SealedBytes {
    type Error = SealedBytesParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        // Base64 text is about 4/3 of the bytes it holds. Refusing a long
        // text first means a huge body is never decoded.
        let max_text = MAX_SEALED_BYTES.div_ceil(3) * 4;
        if text.len() > max_text {
            return Err(SealedBytesParseError::TooLarge {
                bytes: text.len() / 4 * 3,
                max: MAX_SEALED_BYTES,
            });
        }
        let bytes = STANDARD.decode(text).map_err(|_| SealedBytesParseError::NotBase64)?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(SealedBytesParseError::TooLarge {
                bytes: bytes.len(),
                max: MAX_SEALED_BYTES,
            });
        }
        Ok(Self(bytes))
    }
}

impl Deposit {
    /// The checks the mailbox makes on a deposit (before it stores it).
    pub fn check(&self, max_ttl_secs: u64) -> Result<(), DepositError> {
        substrate::resolve_did_key(&self.destination)
            .map_err(|_| DepositError::NotADidKey(self.destination.clone()))?;
        if self.ttl_secs == 0 {
            return Err(DepositError::ZeroTtl);
        }
        if self.ttl_secs > max_ttl_secs {
            return Err(DepositError::TtlTooLong {
                ttl_secs: self.ttl_secs,
                max_secs: max_ttl_secs,
            });
        }
        let bytes = self.sealed.len();
        if bytes > MAX_SEALED_BYTES {
            return Err(DepositError::TooLarge { bytes, max: MAX_SEALED_BYTES });
        }
        if bytes < SEAL_OVERHEAD {
            return Err(DepositError::TooSmall { bytes, min: SEAL_OVERHEAD });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

//! The mailbox key pair of a service member.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::crypto;

/// The public half of a service member's mailbox key. It goes into the
/// signed service record. Text form: z-base-32 of the 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct MailboxPublicKey([u8; 32]);

/// The private half: the 32-byte seed the X25519 key pair is derived from.
/// The deployer makes it with the member master and gives it to each
/// instance; it is never logged.
#[derive(Clone, ZeroizeOnDrop)]
pub struct MailboxSecretKey {
    seed: [u8; 32],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyParseError {
    #[error("a mailbox key is z-base-32 text")]
    NotZ32,
    #[error("a mailbox key is 32 bytes, not {0}")]
    WrongLength(usize),
}

fn decode_key(text: &str) -> Result<[u8; 32], KeyParseError> {
    let bytes = z32::decode(text.as_bytes()).map_err(|_| KeyParseError::NotZ32)?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| KeyParseError::WrongLength(bytes.len()))
}

impl MailboxSecretKey {
    /// A new random key.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed)?;
        Ok(Self { seed })
    }

    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { seed }
    }

    pub(crate) fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    #[must_use]
    pub fn public_key(&self) -> MailboxPublicKey {
        MailboxPublicKey(crypto::public_key_from_seed(&self.seed))
    }

    /// The text form for the deploy call. The wrapper clears the text when
    /// it is dropped.
    #[must_use]
    pub fn to_z32(&self) -> Zeroizing<String> {
        Zeroizing::new(z32::encode(&self.seed))
    }

    pub fn from_z32(text: &str) -> Result<Self, KeyParseError> {
        decode_key(text).map(Self::from_seed)
    }
}

impl fmt::Debug for MailboxSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MailboxSecretKey({})", self.public_key())
    }
}

impl MailboxPublicKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for MailboxPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&z32::encode(&self.0))
    }
}

impl FromStr for MailboxPublicKey {
    type Err = KeyParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        decode_key(text).map(Self)
    }
}

impl From<MailboxPublicKey> for String {
    fn from(key: MailboxPublicKey) -> Self {
        key.to_string()
    }
}

impl TryFrom<String> for MailboxPublicKey {
    type Error = KeyParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[cfg(test)]
mod tests;

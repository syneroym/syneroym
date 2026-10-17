//! The mailbox key pair of a service member.

use std::{fmt, str::FromStr};

use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use syneroym_identity::Identity;
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
    #[error("a mailbox key has one text form: 52 characters with zero spare bits")]
    NotCanonical,
}

/// The decoded bytes may be a secret seed, so every copy is cleared on drop.
fn decode_key(text: &str) -> Result<Zeroizing<[u8; 32]>, KeyParseError> {
    let bytes = Zeroizing::new(z32::decode(text.as_bytes()).map_err(|_| KeyParseError::NotZ32)?);
    let key = Zeroizing::new(
        <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| KeyParseError::WrongLength(bytes.len()))?,
    );
    // The decoder ignores the spare bits of the last character and reads a
    // missing last character as zero, so several texts give one key. The key
    // is stored and compared as text, so only the exact form is accepted.
    if *Zeroizing::new(z32::encode(&*key)) != *text {
        return Err(KeyParseError::NotCanonical);
    }
    Ok(key)
}

/// The HKDF `info` for the mailbox key of a member master. A new version
/// string gives a new, unrelated key.
const MASTER_DERIVATION_INFO: &[u8] = b"syneroym:mailbox-key:v1";

impl MailboxSecretKey {
    /// A new random key.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *seed)?;
        Ok(Self { seed: *seed })
    }

    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { seed }
    }

    /// The mailbox key of the member whose master is `master`. Every holder
    /// of the master gets the same key, so a member keeps one mailbox key on
    /// every deploy and every substrate. HKDF is one-way: the key tells
    /// nothing about the master.
    #[must_use]
    pub fn derive_from_master(master: &Identity) -> Self {
        let ikm = Zeroizing::new(master.to_bytes());
        let mut seed = Zeroizing::new([0u8; 32]);
        // A 32-byte output is always a valid HKDF-SHA256 length; the same
        // expect-and-allow pairing is used by `Identity::derive_service_identity`.
        #[allow(clippy::expect_used)]
        Hkdf::<Sha256>::new(None, ikm.as_slice())
            .expand(MASTER_DERIVATION_INFO, seed.as_mut_slice())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self::from_seed(*seed)
    }

    pub(crate) fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// The raw 32-byte seed, for a store that keeps it as bytes. The wrapper
    /// clears the copy when it is dropped.
    #[must_use]
    pub fn to_seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.seed)
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
        decode_key(text).map(|seed| Self::from_seed(*seed))
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
        decode_key(text).map(|key| Self(*key))
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

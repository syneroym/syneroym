//! HPKE sealing, the only place that names the `hpke` crate. A later move to
//! another `hpke` version changes this file only.

use hpke::{
    Deserializable, Kem as KemTrait, OpModeR, OpModeS, Serializable, aead::AesGcm256,
    kdf::HkdfSha256, kem::X25519HkdfSha256,
};
use rand::{rand_core::UnwrapErr, rngs::OsRng};

use crate::HPKE_INFO;

type MailboxKem = X25519HkdfSha256;
type Kdf = HkdfSha256;
type Aead = AesGcm256;

const ENC_LEN: usize = 32;
/// Bytes HPKE adds to a plaintext: the 32-byte encapsulated key and the
/// 16-byte AES-GCM tag.
pub(crate) const SEAL_OVERHEAD: usize = ENC_LEN + 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum CryptoError {
    #[error("sealed bytes are shorter than the HPKE overhead")]
    TooShort,
    #[error("the sealed bytes do not open with this key")]
    Undecryptable,
    #[error("the public key is not a valid X25519 key")]
    BadPublicKey,
    #[error("sealing failed: {0}")]
    Seal(String),
}

/// The public key of the X25519 key pair derived from the 32-byte seed
/// (RFC 9180 `DeriveKeyPair`).
pub(crate) fn public_key_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    let (_secret, public) = MailboxKem::derive_keypair(seed);
    public.to_bytes().into()
}

/// Returns `enc || ciphertext`. Every call uses a fresh ephemeral key.
pub(crate) fn seal_to(public_key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let public = <MailboxKem as KemTrait>::PublicKey::from_bytes(public_key)
        .map_err(|_| CryptoError::BadPublicKey)?;
    // This panics only if the OS random source fails. The node's own key
    // generation needs the same source, so it cannot run without it anyway.
    let mut rng = UnwrapErr(OsRng);
    let (enc, ciphertext) = hpke::single_shot_seal::<Aead, Kdf, MailboxKem, _>(
        &OpModeS::Base,
        &public,
        HPKE_INFO,
        plaintext,
        b"",
        &mut rng,
    )
    .map_err(|e| CryptoError::Seal(e.to_string()))?;
    let mut sealed = enc.to_bytes().to_vec();
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// Opens `enc || ciphertext` made by `seal_to`. Every decryption failure is
/// one `Undecryptable`: a wrong key and changed bytes look the same.
pub(crate) fn open_with(seed: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < SEAL_OVERHEAD {
        return Err(CryptoError::TooShort);
    }
    let (enc_bytes, ciphertext) = sealed.split_at(ENC_LEN);
    let enc = <MailboxKem as KemTrait>::EncappedKey::from_bytes(enc_bytes)
        .map_err(|_| CryptoError::Undecryptable)?;
    let (secret, _public) = MailboxKem::derive_keypair(seed);
    hpke::single_shot_open::<Aead, Kdf, MailboxKem>(
        &OpModeR::Base,
        &secret,
        &enc,
        HPKE_INFO,
        ciphertext,
        b"",
    )
    .map_err(|_| CryptoError::Undecryptable)
}

#[cfg(test)]
mod tests;

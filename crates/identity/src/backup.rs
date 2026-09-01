#![cfg(feature = "backup")]
//! An encrypted, transportable copy of a person's master key.
//!
//! Encrypted under a randomly generated 32-byte **recovery key**, shown to
//! the person once and never stored. Not under a passphrase: this
//! workspace has no password KDF, and a passphrase would add a
//! parameter-versioning problem and a strength failure mode nothing here
//! can measure. A `kdf` value this build does not know is refused, never
//! guessed.

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit, Payload},
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::{keys::Identity, substrate};

pub const IDENTITY_BACKUP_VERSION: u32 = 1;
pub const KDF_HKDF_SHA256: &str = "hkdf-sha256";
pub const CIPHER_AES_256_GCM: &str = "aes-256-gcm";
const HKDF_INFO: &[u8] = b"syneroym-identity-backup-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityBackup {
    pub backup_version: u32,
    /// The DID this backup restores. Public, and bound into the AEAD's
    /// additional data, so a backup cannot be relabelled as another
    /// person's without decryption failing.
    pub did: String,
    pub kdf: String,
    pub cipher: String,
    pub salt_z32: String,  // 16 bytes
    pub nonce_z32: String, // 12 bytes
    pub ciphertext_z32: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup version {0} is not understood by this build")]
    UnknownVersion(u32),
    #[error("unknown kdf '{0}'")]
    UnknownKdf(String),
    #[error("unknown cipher '{0}'")]
    UnknownCipher(String),
    #[error("recovery key is not 32 bytes of z-base-32")]
    RecoveryKey,
    #[error("could not decrypt: wrong recovery key, or the backup was altered")]
    Decrypt,
    #[error("backup json: {0}")]
    Json(String),
    #[error("restored key does not produce the DID this backup names")]
    DidMismatch,
    #[error("random generation error: {0}")]
    Getrandom(String),
}

pub fn generate_recovery_key() -> Result<[u8; 32], BackupError> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| BackupError::Getrandom(e.to_string()))?;
    Ok(key)
}

/// z-base-32, grouped `xxxxx-xxxxx-…` for reading aloud. Groups are
/// cosmetic: decoding strips `-` and whitespace and is case-insensitive,
/// because a person will retype this.
pub fn encode_recovery_key(key: &[u8; 32]) -> String {
    let encoded = z32::encode(key);
    let mut result = String::with_capacity(encoded.len() + 10);
    for (i, c) in encoded.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            result.push('-');
        }
        result.push(c);
    }
    result
}

pub fn decode_recovery_key(s: &str) -> Result<[u8; 32], BackupError> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let lower = cleaned.to_lowercase();
    let bytes = z32::decode(lower.as_bytes()).map_err(|_| BackupError::RecoveryKey)?;
    if bytes.len() != 32 {
        return Err(BackupError::RecoveryKey);
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

fn aad_bytes(version: u32, did: &str, kdf: &str, cipher: &str, salt_z32: &str) -> Vec<u8> {
    let val = serde_json::json!({
        "backup_version": version,
        "cipher": cipher,
        "did": did,
        "kdf": kdf,
        "salt_z32": salt_z32,
    });
    let canonical = substrate::canonicalize_json_value(&val);
    serde_json::to_vec(&canonical).unwrap_or_default()
}

pub fn export(identity: &Identity, recovery_key: &[u8; 32]) -> Result<IdentityBackup, BackupError> {
    let did = substrate::derive_did_key(&identity.public_key());
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| BackupError::Getrandom(e.to_string()))?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|e| BackupError::Getrandom(e.to_string()))?;

    let salt_z32 = z32::encode(&salt);
    let nonce_z32 = z32::encode(&nonce);

    let mut derived_key = [0u8; 32];
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), recovery_key);
    hkdf.expand(HKDF_INFO, &mut derived_key).map_err(|_| BackupError::Decrypt)?;

    let aad =
        aad_bytes(IDENTITY_BACKUP_VERSION, &did, KDF_HKDF_SHA256, CIPHER_AES_256_GCM, &salt_z32);

    let mut secret = identity.to_bytes();
    let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|_| BackupError::Decrypt)?;
    let nonce_ga = aes_gcm::Nonce::from_slice(&nonce);
    let ct = cipher
        .encrypt(nonce_ga, Payload { msg: &secret, aad: &aad })
        .map_err(|_| BackupError::Decrypt)?;

    secret.zeroize();
    derived_key.zeroize();

    Ok(IdentityBackup {
        backup_version: IDENTITY_BACKUP_VERSION,
        did,
        kdf: KDF_HKDF_SHA256.to_string(),
        cipher: CIPHER_AES_256_GCM.to_string(),
        salt_z32,
        nonce_z32,
        ciphertext_z32: z32::encode(&ct),
    })
}

pub fn import(backup: &IdentityBackup, recovery_key: &[u8; 32]) -> Result<Identity, BackupError> {
    if backup.backup_version != IDENTITY_BACKUP_VERSION {
        return Err(BackupError::UnknownVersion(backup.backup_version));
    }
    if backup.kdf != KDF_HKDF_SHA256 {
        return Err(BackupError::UnknownKdf(backup.kdf.clone()));
    }
    if backup.cipher != CIPHER_AES_256_GCM {
        return Err(BackupError::UnknownCipher(backup.cipher.clone()));
    }

    let salt = z32::decode(backup.salt_z32.as_bytes()).map_err(|_| BackupError::Decrypt)?;
    let nonce = z32::decode(backup.nonce_z32.as_bytes()).map_err(|_| BackupError::Decrypt)?;
    let ct = z32::decode(backup.ciphertext_z32.as_bytes()).map_err(|_| BackupError::Decrypt)?;

    let mut derived_key = [0u8; 32];
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), recovery_key);
    hkdf.expand(HKDF_INFO, &mut derived_key).map_err(|_| BackupError::Decrypt)?;

    let aad = aad_bytes(
        backup.backup_version,
        &backup.did,
        &backup.kdf,
        &backup.cipher,
        &backup.salt_z32,
    );

    let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|_| BackupError::Decrypt)?;
    let nonce_ga = aes_gcm::Nonce::from_slice(&nonce);
    let mut secret = cipher
        .decrypt(nonce_ga, Payload { msg: &ct, aad: &aad })
        .map_err(|_| BackupError::Decrypt)?;

    derived_key.zeroize();

    if secret.len() != 32 {
        secret.zeroize();
        return Err(BackupError::Decrypt);
    }

    let mut secret_arr = [0u8; 32];
    secret_arr.copy_from_slice(&secret);
    secret.zeroize();

    let identity = Identity::from_bytes(&secret_arr);
    secret_arr.zeroize();

    let derived_did = substrate::derive_did_key(&identity.public_key());
    if derived_did != backup.did {
        return Err(BackupError::DidMismatch);
    }

    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_key_encode_decode_round_trip() {
        let key = generate_recovery_key().unwrap();
        let encoded = encode_recovery_key(&key);
        // Retype with spaces, uppercase, extra dashes
        let retyped = format!(" {} - {} ", encoded.to_uppercase(), "---");
        let decoded = decode_recovery_key(&retyped).unwrap();
        assert_eq!(key, decoded);
    }

    #[test]
    fn export_import_round_trip() {
        let identity = Identity::generate().unwrap();
        let recovery_key = generate_recovery_key().unwrap();

        let backup = export(&identity, &recovery_key).unwrap();
        let restored = import(&backup, &recovery_key).unwrap();

        assert_eq!(identity.to_bytes(), restored.to_bytes());
    }

    #[test]
    fn flipped_ciphertext_byte_fails_decrypt() {
        let identity = Identity::generate().unwrap();
        let recovery_key = generate_recovery_key().unwrap();

        let mut backup = export(&identity, &recovery_key).unwrap();
        let mut ct_bytes = z32::decode(backup.ciphertext_z32.as_bytes()).unwrap();
        ct_bytes[0] ^= 0xff;
        backup.ciphertext_z32 = z32::encode(&ct_bytes);

        assert!(matches!(import(&backup, &recovery_key), Err(BackupError::Decrypt)));
    }

    #[test]
    fn changed_did_fails_decrypt() {
        let identity = Identity::generate().unwrap();
        let recovery_key = generate_recovery_key().unwrap();

        let mut backup = export(&identity, &recovery_key).unwrap();
        let other_identity = Identity::generate().unwrap();
        backup.did = substrate::derive_did_key(&other_identity.public_key());

        assert!(matches!(import(&backup, &recovery_key), Err(BackupError::Decrypt)));
    }

    #[test]
    fn unknown_kdf_refused() {
        let identity = Identity::generate().unwrap();
        let recovery_key = generate_recovery_key().unwrap();

        let mut backup = export(&identity, &recovery_key).unwrap();
        backup.kdf = "argon2id".to_string();

        assert!(matches!(import(&backup, &recovery_key), Err(BackupError::UnknownKdf(_))));
    }

    #[test]
    fn unknown_version_refused() {
        let identity = Identity::generate().unwrap();
        let recovery_key = generate_recovery_key().unwrap();

        let mut backup = export(&identity, &recovery_key).unwrap();
        backup.backup_version = 2;

        assert!(matches!(import(&backup, &recovery_key), Err(BackupError::UnknownVersion(2))));
    }
}

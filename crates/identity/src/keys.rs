//! Cryptographic identity and keypair management
//!
//! Defines the primary `Identity` struct utilizing Ed25519 dalek for key
//! generation, secure storage, signing, and DID document generation.

use std::{
    fmt::{self, Debug, Formatter},
    fs, io, mem,
    path::Path,
    slice,
};

use anyhow::Context;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use zeroize::Zeroize;

use crate::{IdentityDoc, substrate};

/// Helper function to lock memory pages to RAM and prevent them from being
/// written to swap or core dumps. Gracefully degrades with a warning log if it
/// fails.
pub fn lock_memory(ptr: *const u8, len: usize) {
    #[cfg(unix)]
    {
        // Safety: ptr is valid and points to initialized memory, len is the size of the
        // key bytes.
        #[allow(unsafe_code)]
        unsafe {
            if libc::mlock(ptr as *const libc::c_void, len) != 0 {
                let err = io::Error::last_os_error();
                tracing::warn!(
                    "Failed to lock memory using mlock: {}. Key memory might be swapped to disk.",
                    err
                );
            }
            // Some Unix systems (like macOS) might not define MADV_DONTDUMP. If not
            // available, we use a fallback or skip it. On macOS, MADV_DONTNEED
            // is defined, but we want to prevent dumping, which might not have a direct
            // equivalent in libc. Under Linux, MADV_DONTDUMP is 16. We can use
            // conditional compilation or look for MADV_DONTDUMP specifically.
            // Let's use MADV_DONTDUMP if it is defined in libc.
            #[cfg(target_os = "linux")]
            {
                if libc::madvise(ptr as *mut libc::c_void, len, libc::MADV_DONTDUMP) != 0 {
                    let err = io::Error::last_os_error();
                    tracing::warn!(
                        "Failed to set MADV_DONTDUMP via madvise: {}. Key memory might be \
                         included in core dumps.",
                        err
                    );
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                // On macOS/BSD etc., mprotect or other guards could be used,
                // but for now we skip or log warning
                // since MADV_DONTDUMP is linux-specific.
                tracing::debug!("MADV_DONTDUMP is not available on this platform");
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Graceful fallback for non-unix platforms
        tracing::warn!("Memory locking (mlock/madvise) is not supported on this platform.");
    }
}

///// A wrapper to ensure the cryptographic key is zeroed when dropped.
struct ZeroizingKey {
    key: SigningKey,
}

impl Drop for ZeroizingKey {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        unsafe {
            let ptr = &mut self.key as *mut SigningKey as *mut u8;
            let len = mem::size_of::<SigningKey>();
            let key_slice = slice::from_raw_parts_mut(ptr, len);
            Zeroize::zeroize(key_slice);
        }
    }
}

/// Represents the cryptographic identity of a Syneroym node.
pub struct Identity {
    signing_key: Box<ZeroizingKey>,
}

impl Debug for Identity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").field("public_key", &self.public_key()).finish()
    }
}

impl Identity {
    /// Generate a new random Ed25519 identity keypair.
    ///
    /// # Errors
    /// Returns an error if the system's random number generator fails (e.g., in
    /// sandboxed environments).
    pub fn generate() -> anyhow::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .context("Failed to generate random bytes for Ed25519 keypair")?;
        let signing_key = Box::new(ZeroizingKey { key: SigningKey::from_bytes(&bytes) });
        Zeroize::zeroize(&mut bytes);
        let id = Self { signing_key };
        lock_memory(
            &id.signing_key.key as *const SigningKey as *const u8,
            mem::size_of::<SigningKey>(),
        );
        Ok(id)
    }

    /// Load an identity from a 32-byte secret key slice.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = Box::new(ZeroizingKey { key: SigningKey::from_bytes(bytes) });
        let id = Self { signing_key };
        lock_memory(
            &id.signing_key.key as *const SigningKey as *const u8,
            mem::size_of::<SigningKey>(),
        );
        id
    }

    /// Load an identity from a file path.
    /// Expects a 32-byte secret key file.
    pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let mut bytes = fs::read(path)
            .with_context(|| format!("Failed to read identity file at {}", path.display()))?;
        let len = bytes.len();
        let mut bytes_array: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("Invalid key file size ({}) at {}", len, path.display())
        })?;
        Zeroize::zeroize(&mut bytes);
        let id = Self::from_bytes(&bytes_array);
        Zeroize::zeroize(&mut bytes_array);
        Ok(id)
    }

    /// Save the identity to a file path.
    /// Writes the 32-byte secret key.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directories for {}", path.display())
            })?;
        }
        let mut bytes = self.to_bytes();

        #[cfg(unix)]
        {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .and_then(|mut f| f.write_all(bytes.as_slice()))
                .with_context(|| format!("Failed to write identity file to {}", path.display()))?;
        }
        #[cfg(not(unix))]
        {
            fs::write(path, bytes.as_slice())
                .with_context(|| format!("Failed to write identity file to {}", path.display()))?;
        }

        Zeroize::zeroize(&mut bytes);
        Ok(())
    }

    /// Export the secret key as a 32-byte array.
    /// WARNING: This must be kept highly secure.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.key.to_bytes()
    }

    /// Get the public verifying key associated with this identity.
    #[must_use]
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.key.verifying_key()
    }

    /// Sign a message payload using this identity.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.key.sign(message)
    }

    /// Sign a JSON value using RFC 8785 (JSON Canonicalization Scheme).
    /// Returns a z-base-32 encoded signature.
    pub fn sign_json(&self, value: &serde_json::Value) -> anyhow::Result<String> {
        let canonical_value = substrate::canonicalize_json_value(value);
        let canonical_string = serde_json::to_string(&canonical_value)?;
        let signature = self.sign(canonical_string.as_bytes());
        Ok(z32::encode(&signature.to_bytes()))
    }

    /// Generate a public `IdentityDoc` for this node.
    #[must_use]
    pub fn to_doc(&self, created_at: u64) -> IdentityDoc {
        let pubkey_bytes = self.public_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let id = format!("did:syn:{pubkey_hex}");

        IdentityDoc { id, pubkey_hex, created_at }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use serde_json::json;

    use super::*;

    #[test]
    fn test_sign_json_deterministic() {
        let identity = Identity::generate().unwrap();
        let v1 = json!({"a": 1, "b": 2});
        let v2 = json!({"b": 2, "a": 1}); // Different key order

        let s1 = identity.sign_json(&v1).unwrap();
        let s2 = identity.sign_json(&v2).unwrap();

        assert_eq!(s1, s2, "Signatures should be identical due to canonicalization");
    }

    #[test]
    fn test_sign_json_nested() {
        let identity = Identity::generate().unwrap();
        let v1 = json!({"x": {"b": 2, "a": 1}, "y": [3, 2, 1]});
        let s1 = identity.sign_json(&v1).unwrap();
        assert!(!s1.is_empty());
    }

    #[test]
    #[allow(unsafe_code)]
    fn test_zeroize_on_drop() {
        use std::mem::ManuallyDrop;
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&[42u8; 32]);
        let mut zero_key =
            ManuallyDrop::new(ZeroizingKey { key: SigningKey::from_bytes(&key_bytes) });
        assert_eq!(zero_key.key.to_bytes(), [42u8; 32]);
        let ptr = &zero_key.key as *const SigningKey as *const [u8; 32];
        unsafe { ManuallyDrop::drop(&mut zero_key) };
        let after_drop: [u8; 32] = unsafe { ptr::read_volatile(ptr) };
        assert_eq!(after_drop, [0u8; 32], "Key bytes must be zeroed on drop");
    }

    #[test]
    #[cfg(unix)]
    fn test_save_to_path_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp_dir = tempfile::tempdir().unwrap();
        let key_path = temp_dir.path().join("test.key");
        let id = Identity::generate().unwrap();
        id.save_to_path(&key_path).unwrap();
        let metadata = fs::metadata(&key_path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn test_lock_memory_graceful_degradation() {
        // Test with invalid pointer/length to force mlock failure
        // and verify it degrades gracefully without panicking or returning error.
        lock_memory(ptr::null(), 999999);
    }

    #[test]
    fn test_substrate_start_mlock_unavailable() {
        // Force mlock failure with invalid pointer, then check if identity generation
        // and load still work.
        lock_memory(ptr::null(), 4096);
        let id = Identity::generate();
        assert!(id.is_ok());
    }
}

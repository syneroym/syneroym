use std::sync::{Arc, Mutex};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use rand::RngCore;
use rusqlite::{Connection, params};
use zeroize::Zeroizing;

use crate::{KeyStoreError, Result};

/// Substrate-global KeyStore managing KEK in memory (locked) and DEKs in
/// database.
pub struct KeyStore {
    kek: Arc<Mutex<Option<Zeroizing<[u8; 32]>>>>,
}

impl std::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let is_configured = self.kek.lock().map(|g| g.is_some()).unwrap_or(false);
        f.debug_struct("KeyStore").field("kek_configured", &is_configured).finish()
    }
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore {
    /// Creates a new empty `KeyStore`.
    pub fn new() -> Self {
        Self { kek: Arc::new(Mutex::new(None)) }
    }

    /// Checks if the KEK is currently loaded/injected in memory.
    pub fn kek_is_loaded(&self) -> bool {
        self.kek.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Injects the Key Encryption Key (KEK) into the KeyStore.
    /// Locks the key memory using `syneroym_identity::lock_memory`.
    pub fn inject_kek(&self, kek_bytes: [u8; 32], _scope: Option<&str>) -> Result<()> {
        let mut guard = self
            .kek
            .lock()
            .map_err(|e| KeyStoreError::Internal(anyhow::anyhow!("Mutex poisoned: {}", e)))?;
        if guard.is_some() {
            return Err(KeyStoreError::KekAlreadyInjected);
        }

        *guard = Some(Zeroizing::new(kek_bytes));

        // Lock KEK memory pages at its heap location
        if let Some(ref k) = *guard {
            syneroym_identity::lock_memory(k.as_ptr(), k.len());
        }

        Ok(())
    }

    /// Clears the KEK from memory (for testing or shutdown).
    pub fn clear_kek(&self) {
        if let Ok(mut guard) = self.kek.lock() {
            *guard = None;
        }
    }

    /// Helper to get a copy of KEK if injected.
    fn get_kek(&self) -> Result<Zeroizing<[u8; 32]>> {
        let guard = self
            .kek
            .lock()
            .map_err(|e| KeyStoreError::Internal(anyhow::anyhow!("Mutex poisoned: {}", e)))?;
        match &*guard {
            Some(k) => Ok(k.clone()),
            None => Err(KeyStoreError::KekRequired),
        }
    }

    /// Generates a new 32-byte DEK, encrypts it with KEK, and stores it in the
    /// database.
    pub fn generate_dek(&self, service_id: &str, conn: &Connection) -> Result<[u8; 32]> {
        let kek = self.get_kek()?;

        // Generate a new random DEK
        let mut dek = [0u8; 32];
        rand::rng().fill_bytes(&mut dek);

        // Encrypt the DEK using the KEK (AES-256-GCM)
        let key = Key::<Aes256Gcm>::from_slice(&*kek);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, dek.as_slice())
            .map_err(|e| KeyStoreError::Crypto(e.to_string()))?;

        // Store encrypted DEK in database
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO dek_store (service_id, encrypted_dek, nonce, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![service_id, ciphertext, nonce_bytes.as_slice(), now],
        )?;

        Ok(dek)
    }

    /// Loads and decrypts the DEK for a given service.
    pub fn load_dek(&self, service_id: &str, conn: &Connection) -> Result<[u8; 32]> {
        let kek = self.get_kek()?;

        let mut stmt =
            conn.prepare("SELECT encrypted_dek, nonce FROM dek_store WHERE service_id = ?1")?;

        let mut rows = stmt.query(params![service_id])?;
        if let Some(row) = rows.next()? {
            let ciphertext: Vec<u8> = row.get(0)?;
            let nonce_bytes: Vec<u8> = row.get(1)?;

            if nonce_bytes.len() != 12 {
                return Err(KeyStoreError::Crypto("Invalid nonce length stored in DB".into()));
            }

            let key = Key::<Aes256Gcm>::from_slice(&*kek);
            let cipher = Aes256Gcm::new(key);
            let nonce = Nonce::from_slice(&nonce_bytes);

            let decrypted = cipher
                .decrypt(nonce, ciphertext.as_slice())
                .map_err(|e| KeyStoreError::Crypto(e.to_string()))?;

            if decrypted.len() != 32 {
                return Err(KeyStoreError::Crypto("Decrypted DEK has invalid length".into()));
            }

            let mut dek = [0u8; 32];
            dek.copy_from_slice(&decrypted);
            Ok(dek)
        } else {
            Err(KeyStoreError::Database(rusqlite::Error::QueryReturnedNoRows))
        }
    }

    /// Rotates the KEK, re-encrypting all DEKs in a single database
    /// transaction.
    pub fn rotate_kek(&self, new_kek: [u8; 32], conn: &mut Connection) -> Result<()> {
        let new_kek = Zeroizing::new(new_kek);
        let old_kek = self.get_kek()?;

        // Perform the rotation in a transaction
        let tx = conn.transaction()?;

        let mut rotated_rows = Vec::new();

        {
            // Query all DEKs
            let mut stmt =
                tx.prepare("SELECT service_id, encrypted_dek, nonce, created_at FROM dek_store")?;
            let mut rows = stmt.query([])?;

            while let Some(row) = rows.next()? {
                let service_id: String = row.get(0)?;
                let ciphertext: Vec<u8> = row.get(1)?;
                let nonce_bytes: Vec<u8> = row.get(2)?;
                let created_at: i64 = row.get(3)?;

                if nonce_bytes.len() != 12 {
                    return Err(KeyStoreError::Crypto(format!(
                        "Invalid nonce length for {}",
                        service_id
                    )));
                }

                // Decrypt DEK using old KEK
                let old_key = Key::<Aes256Gcm>::from_slice(&*old_kek);
                let old_cipher = Aes256Gcm::new(old_key);
                let nonce = Nonce::from_slice(&nonce_bytes);
                let dek = old_cipher
                    .decrypt(nonce, ciphertext.as_slice())
                    .map_err(|e| KeyStoreError::Crypto(e.to_string()))?;

                // Re-encrypt using new KEK
                let new_key = Key::<Aes256Gcm>::from_slice(&*new_kek);
                let new_cipher = Aes256Gcm::new(new_key);

                let mut new_nonce_bytes = [0u8; 12];
                rand::rng().fill_bytes(&mut new_nonce_bytes);
                let new_nonce = Nonce::from_slice(&new_nonce_bytes);

                let new_ciphertext = new_cipher
                    .encrypt(new_nonce, dek.as_slice())
                    .map_err(|e| KeyStoreError::Crypto(e.to_string()))?;

                rotated_rows.push((service_id, new_ciphertext, new_nonce_bytes, created_at));
            }
        }

        // Update database with newly encrypted rows
        for (service_id, new_ciphertext, new_nonce_bytes, created_at) in rotated_rows {
            tx.execute(
                "INSERT OR REPLACE INTO dek_store (service_id, encrypted_dek, nonce, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![service_id, new_ciphertext, new_nonce_bytes.as_slice(), created_at],
            )?;
        }

        tx.commit()?;

        // Update active KEK in KeyStore
        let mut guard = self
            .kek
            .lock()
            .map_err(|e| KeyStoreError::Internal(anyhow::anyhow!("Mutex poisoned: {}", e)))?;
        *guard = Some(new_kek);

        // Lock new KEK memory pages at its heap location
        if let Some(ref k) = *guard {
            syneroym_identity::lock_memory(k.as_ptr(), k.len());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_load_dek_round_trip() {
        let db = Connection::open_in_memory().unwrap();
        db.execute(
            "CREATE TABLE dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();

        let ks = KeyStore::new();
        // Fails if KEK is not injected
        assert!(ks.generate_dek("svc-a", &db).is_err());

        // Inject KEK
        let kek = [7u8; 32];
        ks.inject_kek(kek, None).unwrap();

        // Injecting twice fails
        assert!(ks.inject_kek(kek, None).is_err());

        // Generate DEK
        let dek = ks.generate_dek("svc-a", &db).unwrap();
        assert_ne!(dek, [0u8; 32]);

        // Load DEK
        let loaded = ks.load_dek("svc-a", &db).unwrap();
        assert_eq!(dek, loaded);
    }

    #[test]
    fn test_rotate_kek() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute(
            "CREATE TABLE dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();

        let ks = KeyStore::new();
        let old_kek = [1u8; 32];
        ks.inject_kek(old_kek, None).unwrap();

        let dek_a = ks.generate_dek("svc-a", &db).unwrap();
        let dek_b = ks.generate_dek("svc-b", &db).unwrap();

        // Rotate KEK
        let new_kek = [2u8; 32];
        ks.rotate_kek(new_kek, &mut db).unwrap();

        // DEKs should load successfully and match original values
        let loaded_a = ks.load_dek("svc-a", &db).unwrap();
        let loaded_b = ks.load_dek("svc-b", &db).unwrap();

        assert_eq!(dek_a, loaded_a);
        assert_eq!(dek_b, loaded_b);
    }

    #[test]
    fn test_kek_zeroized_on_rotation() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute(
            "CREATE TABLE dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();

        let ks = KeyStore::new();
        let old_kek = [1u8; 32];
        ks.inject_kek(old_kek, None).unwrap();

        let mut old_kek_zeroizing = Box::new(ks.get_kek().unwrap());
        let ptr = old_kek_zeroizing.as_mut_ptr();

        let new_kek = [2u8; 32];
        ks.rotate_kek(new_kek, &mut db).unwrap();

        // Zeroize in-place before Box deallocation to prevent stack move copies
        zeroize::Zeroize::zeroize(&mut **old_kek_zeroizing);

        unsafe {
            let memory = std::slice::from_raw_parts(ptr, 32);
            assert_eq!(memory, &[0u8; 32]);
        }
    }
}

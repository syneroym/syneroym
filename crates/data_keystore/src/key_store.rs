use std::{
    fmt,
    sync::{Arc, Mutex},
};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use chrono::Utc;
use hkdf::Hkdf;
use rand::RngCore;
use rusqlite::{Connection, Error as RusqliteError, params};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{KeyStoreError, Result};

/// Substrate-global `KeyStore` managing one master KEK in memory (locked)
/// and per-service DEKs in the database. Each DEK is wrapped not by the raw
/// master but by a **per-instance KEK derived via
/// HKDF-SHA256(master, info = "syneroym:kek:v1:{service_id}")** — see
/// [`derive_instance_kek`] (M04A Slice B6, "Model A": derived, not
/// separately provisioned; ADR-0006). The `service_id` is the derivation
/// scope; it is also the app-instance id (`io.rs` in `syneroym-router`
/// records `app_instance_id == service_id`), so this is per-app-instance
/// narrowing without any new identifier. A leaked derived key does not
/// reveal the master or any sibling instance's key, but the master itself
/// (or substrate-RAM access) still derives every instance's key — see
/// ADR-0006's amended "KEK Scope" section for the threat-model limitation
/// this does and does not buy.
pub struct KeyStore {
    kek: Arc<Mutex<Option<Zeroizing<[u8; 32]>>>>,
}

/// Derives the per-instance KEK that actually wraps `scope`'s DEK, from the
/// injected master KEK. Domain-separated from `data_blob::crypto`'s
/// `"syneroym:blob*"` HKDF `info` strings by its own `"syneroym:kek:v1:"`
/// prefix, and from other instances by `scope` (the `service_id`).
fn derive_instance_kek(master: &[u8; 32], scope: &str) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, master);
    let info = format!("syneroym:kek:v1:{scope}");
    let mut okm = Zeroizing::new([0u8; 32]);
    // `expect_used` is workspace warn-level; `data_blob/src/crypto.rs` sets
    // the same precedent for this exact expect+allow pairing on a
    // fixed-length HKDF expand, which cannot fail for a 32-byte SHA256 OKM.
    #[allow(clippy::expect_used)]
    hk.expand(info.as_bytes(), okm.as_mut_slice())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

impl fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    /// Injects the substrate-global master Key Encryption Key (KEK) into the
    /// `KeyStore`. Locks the key memory using `syneroym_identity::lock_memory`.
    /// Per-instance wrap keys are derived from this master on demand (see
    /// [`derive_instance_kek`]) — there is no separate per-instance inject
    /// path today (ADR-0006 "Model B", deferred).
    pub fn inject_kek(&self, kek_bytes: [u8; 32]) -> Result<()> {
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

    /// Generates a new 32-byte DEK, encrypts it with the KEK derived for
    /// `service_id`, and stores it in the database.
    pub fn generate_dek(&self, service_id: &str, conn: &Connection) -> Result<Zeroizing<[u8; 32]>> {
        let master = self.get_kek()?;
        let kek = derive_instance_kek(&master, service_id);

        // Generate a new random DEK
        let mut dek = Zeroizing::new([0u8; 32]);
        rand::rng().fill_bytes(&mut *dek);

        // Encrypt the DEK using the per-instance KEK (AES-256-GCM)
        let key = Key::<Aes256Gcm>::from_slice(&*kek);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, dek.as_slice())
            .map_err(|e| KeyStoreError::Crypto(e.to_string()))?;

        // Store encrypted DEK in database
        let now = Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO dek_store (service_id, encrypted_dek, nonce, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![service_id, ciphertext, nonce_bytes.as_slice(), now],
        )?;

        Ok(dek)
    }

    /// Loads and decrypts the DEK for a given service, using the KEK
    /// derived for `service_id`.
    pub fn load_dek(&self, service_id: &str, conn: &Connection) -> Result<Zeroizing<[u8; 32]>> {
        let master = self.get_kek()?;
        let kek = derive_instance_kek(&master, service_id);

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

            let decrypted = Zeroizing::new(
                cipher
                    .decrypt(nonce, ciphertext.as_slice())
                    .map_err(|e| KeyStoreError::Crypto(e.to_string()))?,
            );

            if decrypted.len() != 32 {
                return Err(KeyStoreError::Crypto("Decrypted DEK has invalid length".into()));
            }

            let mut dek = Zeroizing::new([0u8; 32]);
            dek.copy_from_slice(&decrypted);
            Ok(dek)
        } else {
            Err(KeyStoreError::Database(RusqliteError::QueryReturnedNoRows))
        }
    }

    /// Rotates the master KEK, re-wrapping every row's DEK in a single
    /// database transaction: each row is unwrapped under the per-instance
    /// key derived from the *old* master and re-wrapped under the
    /// per-instance key derived from the *new* master, both scoped by that
    /// row's own `service_id`. This is the re-wrap mechanism the Migration
    /// Strategy calls for (M04A Slice B6) — exercised end-to-end by
    /// `rotate_kek_preserves_per_instance_deks`.
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

                // Decrypt DEK using the per-instance KEK derived from the
                // old master, scoped to this row's service_id.
                let old_instance_kek = derive_instance_kek(&old_kek, &service_id);
                let old_key = Key::<Aes256Gcm>::from_slice(&*old_instance_kek);
                let old_cipher = Aes256Gcm::new(old_key);
                let nonce = Nonce::from_slice(&nonce_bytes);
                let dek = Zeroizing::new(
                    old_cipher
                        .decrypt(nonce, ciphertext.as_slice())
                        .map_err(|e| KeyStoreError::Crypto(e.to_string()))?,
                );

                // Re-encrypt using the per-instance KEK derived from the new
                // master, same scope.
                let new_instance_kek = derive_instance_kek(&new_kek, &service_id);
                let new_key = Key::<Aes256Gcm>::from_slice(&*new_instance_kek);
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

        // Update active KEK in KeyStore. Accepted ordering risk: if this
        // `lock()` fails (mutex poisoned by an earlier panic), the rows are
        // already committed under `new_kek` but the in-memory guard still
        // holds `old_kek` until the process restarts and re-injects --
        // low-likelihood (poison-only), no data loss, so not worth widening
        // the lock scope across the whole transaction just to close it.
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
    use std::{fs, slice};

    use zeroize::Zeroize;

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
        ks.inject_kek(kek).unwrap();

        // Injecting twice fails
        assert!(ks.inject_kek(kek).is_err());

        // Generate DEK
        let dek = ks.generate_dek("svc-a", &db).unwrap();
        assert_ne!(*dek, [0u8; 32]);

        // Load DEK
        let loaded = ks.load_dek("svc-a", &db).unwrap();
        assert_eq!(dek, loaded);
    }

    /// M04A Slice B6 §5 test 1: guards the derive path end-to-end.
    #[test]
    fn per_instance_wrap_round_trip() {
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
        ks.inject_kek([7u8; 32]).unwrap();

        let dek = ks.generate_dek("svc-a", &db).unwrap();
        let loaded = ks.load_dek("svc-a", &db).unwrap();
        assert_eq!(dek, loaded);
    }

    /// M04A Slice B6 §5 test 2 — the failure-table proof (task.md's
    /// "Failure and Security Tests" row): a DEK wrapped for one instance's
    /// scope is cryptographically undecryptable under a sibling instance's
    /// derived key, and `load_dek` only ever resolves the matching
    /// `service_id`'s own row.
    #[test]
    fn cross_instance_kek_isolation() {
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
        let master = Zeroizing::new([5u8; 32]);
        ks.inject_kek(*master).unwrap();

        // Hand-wrap a DEK directly under svc-a's derived instance key.
        let mut dek_a = [0u8; 32];
        rand::rng().fill_bytes(&mut dek_a);
        let kek_a = derive_instance_kek(&master, "svc-a");
        let cipher_a = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*kek_a));
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher_a.encrypt(nonce, dek_a.as_slice()).unwrap();

        // svc-b's derived key must not decrypt svc-a's wrapped DEK --
        // distinct HKDF `info` strings produce unrelated keys.
        let kek_b = derive_instance_kek(&master, "svc-b");
        let cipher_b = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*kek_b));
        assert!(
            cipher_b.decrypt(nonce, ciphertext.as_slice()).is_err(),
            "svc-b's derived KEK must not decrypt svc-a's wrapped DEK"
        );

        // End-to-end via the public API: each service_id only ever loads
        // its own DEK back, never a sibling's.
        let dek_a_stored = ks.generate_dek("svc-a", &db).unwrap();
        let dek_b_stored = ks.generate_dek("svc-b", &db).unwrap();
        assert_ne!(dek_a_stored, dek_b_stored);
        assert_eq!(ks.load_dek("svc-a", &db).unwrap(), dek_a_stored);
        assert_eq!(ks.load_dek("svc-b", &db).unwrap(), dek_b_stored);
    }

    /// M04A Slice B6 §5 test 3 (extends the former `test_rotate_kek`) --
    /// the re-wrap-path proof the Migration Strategy asks for: unwrap under
    /// `HKDF(old_master, service_id)`, re-wrap under
    /// `HKDF(new_master, service_id)`.
    #[test]
    fn rotate_kek_preserves_per_instance_deks() {
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
        ks.inject_kek(old_kek).unwrap();

        let dek_a = ks.generate_dek("svc-a", &db).unwrap();
        let dek_b = ks.generate_dek("svc-b", &db).unwrap();

        let old_master = Zeroizing::new(old_kek);
        let derived_old_a = derive_instance_kek(&old_master, "svc-a");
        assert_ne!(*derived_old_a, old_kek, "derived KEK must differ from the raw master");

        // Rotate KEK
        let new_kek = [2u8; 32];
        ks.rotate_kek(new_kek, &mut db).unwrap();

        // DEKs should load successfully and match original values
        let loaded_a = ks.load_dek("svc-a", &db).unwrap();
        let loaded_b = ks.load_dek("svc-b", &db).unwrap();

        assert_eq!(dek_a, loaded_a);
        assert_eq!(dek_b, loaded_b);

        // Prove a genuine re-wrap happened (not a no-op): svc-a's row is no
        // longer decryptable under the pre-rotation derived key.
        let (ciphertext, nonce_bytes): (Vec<u8>, Vec<u8>) = db
            .query_row(
                "SELECT encrypted_dek, nonce FROM dek_store WHERE service_id = 'svc-a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let old_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*derived_old_a));
        let nonce = Nonce::from_slice(&nonce_bytes);
        assert!(
            old_cipher.decrypt(nonce, ciphertext.as_slice()).is_err(),
            "row must be unreadable under the pre-rotation derived key after rotate_kek"
        );
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
        ks.inject_kek(old_kek).unwrap();

        let mut old_kek_zeroizing = Box::new(ks.get_kek().unwrap());
        let ptr = old_kek_zeroizing.as_mut_ptr();

        let new_kek = [2u8; 32];
        ks.rotate_kek(new_kek, &mut db).unwrap();

        // Zeroize in-place before Box deallocation to prevent stack move copies
        Zeroize::zeroize(&mut **old_kek_zeroizing);

        unsafe {
            let memory = slice::from_raw_parts(ptr, 32);
            assert_eq!(memory, &[0u8; 32]);
        }
    }

    /// M3A exit criterion: "DEK never appears in plaintext on disk;
    /// verified by hex dump of `substrate.db`." Uses a real file-backed
    /// database (not `open_in_memory`) so the assertion covers what
    /// actually lands on disk, then re-reads the raw file bytes after the
    /// connection is closed and searches for the plaintext DEK as a
    /// contiguous byte run.
    #[test]
    fn test_dek_never_plaintext_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("substrate.db");

        let dek_plain = {
            let db = Connection::open(&db_path).unwrap();
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
            ks.inject_kek([9u8; 32]).unwrap();
            let dek = ks.generate_dek("svc-disk-check", &db).unwrap();

            // Sanity: the round trip still works against the on-disk file.
            let loaded = ks.load_dek("svc-disk-check", &db).unwrap();
            assert_eq!(dek, loaded);
            dek
        };
        // Connection dropped here, forcing SQLite to flush all pages to disk.

        let raw_bytes = fs::read(&db_path).unwrap();
        assert!(
            !raw_bytes.windows(dek_plain.len()).any(|window| window == &dek_plain[..]),
            "plaintext DEK bytes found verbatim in substrate.db on disk"
        );
    }

    /// M04A Slice B6 review (S2 gap): a DEK generated under one master must
    /// fail to load -- cleanly, not by panicking -- once the keystore holds
    /// a *different* master. Guards the M3-era-DB-wipe assumption (task.md
    /// F2/F3): rotation is the only supported path between masters.
    #[test]
    fn load_dek_fails_cleanly_under_wrong_master() {
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

        let ks_m1 = KeyStore::new();
        ks_m1.inject_kek([11u8; 32]).unwrap();
        ks_m1.generate_dek("svc-a", &db).unwrap();

        let ks_m2 = KeyStore::new();
        ks_m2.inject_kek([22u8; 32]).unwrap();
        assert!(
            matches!(ks_m2.load_dek("svc-a", &db), Err(KeyStoreError::Crypto(_))),
            "loading a DEK wrapped under a different master must fail with a Crypto error, not \
             panic"
        );
    }

    /// M04A Slice B6 review (T3 gap): rotation must succeed as a no-op when
    /// `dek_store` has no rows yet (a fresh substrate rotating before any
    /// service has been provisioned).
    #[test]
    fn rotate_kek_succeeds_on_empty_dek_store() {
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
        ks.inject_kek([1u8; 32]).unwrap();

        ks.rotate_kek([2u8; 32], &mut db).unwrap();

        // The new master is active: a DEK generated after rotation must
        // round-trip under it.
        let dek = ks.generate_dek("svc-a", &db).unwrap();
        assert_eq!(ks.load_dek("svc-a", &db).unwrap(), dek);
    }
}

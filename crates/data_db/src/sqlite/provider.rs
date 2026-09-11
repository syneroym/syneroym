use std::{
    collections::HashMap,
    fmt,
    fs::DirBuilder,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use deadpool_sqlite::{Config as PoolConfig, Hook, HookError, Runtime};
use regex::Regex;
use rusqlite::{Connection, Error as SqliteError, params};
use syneroym_data_keystore::{KeyStore, KeyStoreError};
use tokio::{sync::mpsc, task};
use zeroize::Zeroizing;

use super::{
    schema::SUBSTRATE_SCHEMA_VERSION,
    service_store::{SqliteServiceStore, run_writer_loop},
};
use crate::traits::{ServiceStore, StorageProvider};

// Real service ids are DIDs (e.g. `did:key:h7wy...`), which contain colons;
// `:` is not a path separator on any Rust-supported OS, so allowing it here
// does not weaken the path-traversal guard below (which still relies on
// rejecting `.`/`/`/`\` and on the `starts_with` descendant check).
#[allow(clippy::unwrap_used)]
static SERVICE_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_:\-]{1,128}$").unwrap());

/// SqliteStorageProvider manages the substrate.db (metadata) and per-service
/// encrypted databases.
pub struct SqliteStorageProvider {
    pub(crate) db_dir: PathBuf,
    pub(crate) substrate_conn: Arc<Mutex<Connection>>,
    pub(crate) service_stores: Arc<Mutex<HashMap<String, Arc<SqliteServiceStore>>>>,
    pub(crate) encryption_enabled: bool,
}

impl fmt::Debug for SqliteStorageProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteStorageProvider")
            .field("db_dir", &self.db_dir)
            .field("encryption_enabled", &self.encryption_enabled)
            .finish()
    }
}

impl SqliteStorageProvider {
    /// Creates a new `SqliteStorageProvider`. Runs schema migrations on
    /// `substrate.db`.
    pub fn new<P: AsRef<Path>>(db_dir: P, encryption_enabled: bool) -> anyhow::Result<Self> {
        let db_dir = db_dir.as_ref().to_path_buf();

        // Ensure db_dir exists
        if !db_dir.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&db_dir)?;
            }
        }

        let substrate_path = db_dir.join("substrate.db");
        let mut conn = Connection::open(&substrate_path)?;

        conn.execute("CREATE TABLE IF NOT EXISTS schema_version (version TEXT NOT NULL)", [])?;

        let stored_version: Option<String> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
            .map(Some)
            .or_else(|e| match e {
                SqliteError::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        let tx = conn.transaction()?;
        if let Some(version) = stored_version.as_deref()
            && version != SUBSTRATE_SCHEMA_VERSION
        {
            tracing::info!(
                from_version = version,
                to_version = SUBSTRATE_SCHEMA_VERSION,
                "Migrating substrate database schema"
            );
        }
        Self::run_m3a_migration(&tx)?;
        Self::run_m3b_migration(&tx)?;
        Self::run_fdae_migration(&tx)?;
        let updated =
            tx.execute("UPDATE schema_version SET version = ?1", [SUBSTRATE_SCHEMA_VERSION])?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [SUBSTRATE_SCHEMA_VERSION],
            )?;
        }
        tx.commit()?;

        Ok(Self {
            db_dir,
            substrate_conn: Arc::new(Mutex::new(conn)),
            service_stores: Arc::new(Mutex::new(HashMap::new())),
            encryption_enabled,
        })
    }

    pub(crate) fn run_m3a_migration(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS config_generations (
                service_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                config_blob TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, generation)
            )",
            [],
        )?;
        Ok(())
    }

    pub(crate) fn run_m3b_migration(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messaging_subscriptions (
                service_id TEXT NOT NULL,
                topic      TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, topic)
            )",
            [],
        )?;
        Ok(())
    }

    pub(crate) fn run_fdae_migration(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fdae_policies (
                service_id  TEXT PRIMARY KEY,
                policy_json TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    /// Internal helper to obtain KEK status and print warnings.
    pub(crate) fn verify_encryption_mode(&self, key_store: &Arc<KeyStore>) -> anyhow::Result<()> {
        if self.encryption_enabled {
            if !key_store.kek_is_loaded() {
                tracing::error!("Production encryption is enabled, but no KEK has been injected!");
                return Err(anyhow::anyhow!("EncryptionKeyRequired"));
            }
        } else {
            // Log security warning
            tracing::warn!(
                "INSECURE: storage encryption is disabled. Only use in development profiles."
            );
        }
        Ok(())
    }

    /// Resolves (generating on first use) the DEK for `service_id`.
    /// `Ok(None)` means encryption is disabled -- callers must not treat
    /// this as an error, it's a deliberate per-deployment mode. Does not
    /// call `verify_encryption_mode`; callers that need the
    /// `EncryptionKeyRequired` / insecure-mode-warning checks (e.g.
    /// `open_service_db`) must call it themselves first. Validates
    /// `service_id` against `SERVICE_ID_REGEX` itself (rather than relying on
    /// callers like `open_service_db` to have already done so via
    /// `resolve_service_db_dir`), so every path into the KEK/DEK layer --
    /// including `load_service_dek`, which has no filesystem path to guard
    /// on -- rejects the same malformed `service_id`s before they become an
    /// HKDF scope.
    pub(crate) fn resolve_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<Zeroizing<[u8; 32]>>> {
        if !self.encryption_enabled {
            return Ok(None);
        }
        if !SERVICE_ID_REGEX.is_match(service_id) {
            return Err(anyhow::anyhow!("Invalid service ID format: {service_id}"));
        }
        let conn =
            self.substrate_conn.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
        let dek = match key_store.load_dek(service_id, &conn) {
            Ok(bytes) => bytes,
            Err(KeyStoreError::Database(SqliteError::QueryReturnedNoRows)) => {
                key_store.generate_dek(service_id, &conn)?
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Some(dek))
    }

    /// Validates `service_id` and resolves its expected per-service database
    /// directory, guarding against path traversal. Does not touch the
    /// filesystem.
    pub(crate) fn resolve_service_db_dir(&self, service_id: &str) -> anyhow::Result<PathBuf> {
        if !SERVICE_ID_REGEX.is_match(service_id) {
            return Err(anyhow::anyhow!("Invalid service ID format: {service_id}"));
        }
        let services_dir = self.db_dir.join("services");
        let service_db_dir = services_dir.join(service_id);
        if !service_db_dir.starts_with(&services_dir) {
            return Err(anyhow::anyhow!(
                "Path traversal attempt rejected for service ID: {service_id}"
            ));
        }
        Ok(service_db_dir)
    }
}

#[async_trait]
impl StorageProvider for SqliteStorageProvider {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>> {
        let service_db_dir = self.resolve_service_db_dir(service_id)?;

        // Verify encryption requirements
        self.verify_encryption_mode(key_store)?;

        if let Some(store) = self
            .service_stores
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?
            .get(service_id)
            .cloned()
        {
            return Ok(Box::new(store));
        }

        // Ensure service_db_dir exists
        if !service_db_dir.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&service_db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&service_db_dir)?;
            }
        }

        let db_file_path = service_db_dir.join("state.db");

        // Resolve or generate DEK. `resolve_dek` returns `None` when
        // encryption is disabled; SQLCipher still needs *a* key to pragma,
        // so a dummy all-zero key is substituted here specifically (this
        // dummy-key behavior is local to the SQLite/SQLCipher path -- the
        // public `load_service_dek` trait method returns `None` as-is to
        // its callers instead of this substitution).
        let dek =
            self.resolve_dek(service_id, key_store)?.unwrap_or_else(|| Zeroizing::new([0u8; 32]));

        // Open service DB connection for the single writer actor
        let writer_conn = Connection::open(&db_file_path)?;
        if self.encryption_enabled {
            let pragma_val = Zeroizing::new(format!("x'{}'", hex::encode(*dek)));
            writer_conn.pragma_update(None, "key", &*pragma_val)?;
        }

        // Initialize vault table
        writer_conn.execute(
            "CREATE TABLE IF NOT EXISTS _vault (
                key        TEXT PRIMARY KEY,
                ciphertext BLOB NOT NULL,
                nonce      BLOB NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;

        // Start single-writer background loop
        let (writer_tx, writer_rx) = mpsc::channel(100);
        let dek_writer = dek.clone();
        task::spawn_blocking(move || {
            run_writer_loop(writer_conn, writer_rx, dek_writer);
        });

        // Initialize reader pool, used for all read-only operations (vault
        // reveal reads stay on the writer above for simplicity; CRUD reads
        // use this pool for concurrency).
        let cfg = PoolConfig::new(&db_file_path);
        let mut reader_pool_builder = cfg.builder(Runtime::Tokio1)?;
        if self.encryption_enabled {
            let reader_dek = dek.clone();
            reader_pool_builder =
                reader_pool_builder.post_create(Hook::async_fn(move |conn, _metrics| {
                    let reader_dek = reader_dek.clone();
                    Box::pin(async move {
                        let pragma_val = Zeroizing::new(format!("x'{}'", hex::encode(*reader_dek)));
                        conn.interact(move |conn| conn.pragma_update(None, "key", &*pragma_val))
                            .await
                            .map_err(|e| HookError::message(format!("Interact error: {e}")))?
                            .map_err(HookError::Backend)?;
                        Ok(())
                    })
                }));
        }
        let reader_pool = reader_pool_builder.build()?;

        let new_store = Arc::new(SqliteServiceStore { reader_pool, writer_tx });
        let store = {
            let mut stores =
                self.service_stores.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            stores.entry(service_id.to_string()).or_insert_with(|| new_store.clone()).clone()
        };

        Ok(Box::new(store))
    }

    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let ks = key_store.clone();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            ks.rotate_kek(new_kek, &mut conn)?;
            Ok(())
        })
        .await?
    }

    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        let service_db_dir = self.resolve_service_db_dir(service_id)?;
        Ok(service_db_dir.join("state.db").exists())
    }

    fn service_db_dir(&self, service_id: &str) -> anyhow::Result<PathBuf> {
        self.resolve_service_db_dir(service_id)
    }

    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<Zeroizing<[u8; 32]>>> {
        self.verify_encryption_mode(key_store)?;
        self.resolve_dek(service_id, key_store)
    }

    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let blob = config_blob.to_string();
        task::spawn_blocking(move || -> anyhow::Result<u64> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let tx = conn.transaction()?;

            let current_gen: Option<i64> = tx
                .query_row(
                    "SELECT MAX(generation) FROM config_generations WHERE service_id = ?1",
                    params![s_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    SqliteError::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?
                .flatten();

            let next_gen = current_gen.unwrap_or(0) + 1;
            let now = Utc::now().timestamp_millis();

            tx.execute(
                "INSERT INTO config_generations (service_id, generation, config_blob, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![s_id, next_gen, blob, now],
            )?;

            tx.commit()?;
            Ok(next_gen as u64)
        })
        .await?
    }

    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            conn.execute(
                "DELETE FROM config_generations WHERE service_id = ?1 AND generation = ?2",
                params![s_id, generation as i64],
            )?;
            Ok(())
        })
        .await?
    }

    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT config_blob FROM config_generations WHERE service_id = ?1 AND generation \
                 = ?2",
            )?;
            let mut rows = stmt.query(params![s_id, generation as i64])?;

            if let Some(row) = rows.next()? { Ok(Some(row.get(0)?)) } else { Ok(None) }
        })
        .await?
    }

    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<Option<(u64, String)>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT generation, config_blob FROM config_generations WHERE service_id = ?1 \
                 ORDER BY generation DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![s_id])?;

            if let Some(row) = rows.next()? {
                Ok(Some((row.get::<_, i64>(0)? as u64, row.get(1)?)))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let topic = topic.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let now = Utc::now().timestamp_millis();
            conn.execute(
                "INSERT INTO messaging_subscriptions (service_id, topic, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (service_id, topic) DO NOTHING",
                params![s_id, topic, now],
            )?;
            Ok(())
        })
        .await?
    }

    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let topic = topic.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            conn.execute(
                "DELETE FROM messaging_subscriptions WHERE service_id = ?1 AND topic = ?2",
                params![s_id, topic],
            )?;
            Ok(())
        })
        .await?
    }

    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            conn.execute(
                "DELETE FROM messaging_subscriptions WHERE service_id = ?1",
                params![s_id],
            )?;
            Ok(())
        })
        .await?
    }

    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        let conn_arc = self.substrate_conn.clone();
        task::spawn_blocking(move || -> anyhow::Result<Vec<(String, String)>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let mut stmt = conn.prepare("SELECT service_id, topic FROM messaging_subscriptions")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }

    async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let policy = policy_json.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let now = Utc::now().timestamp_millis();
            conn.execute(
                "INSERT INTO fdae_policies (service_id, policy_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (service_id) DO UPDATE SET
                     policy_json = excluded.policy_json,
                     updated_at = excluded.updated_at",
                params![s_id, policy, now],
            )?;
            Ok(())
        })
        .await?
    }

    async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            let mut stmt =
                conn.prepare("SELECT policy_json FROM fdae_policies WHERE service_id = ?1")?;
            let mut rows = stmt.query(params![s_id])?;
            if let Some(row) = rows.next()? { Ok(Some(row.get(0)?)) } else { Ok(None) }
        })
        .await?
    }

    async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {e}"))?;
            conn.execute("DELETE FROM fdae_policies WHERE service_id = ?1", params![s_id])?;
            Ok(())
        })
        .await?
    }
}

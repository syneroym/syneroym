use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use async_trait::async_trait;
use rand::RngCore;
use syneroym_key_store::KeyStore;
use zeroize::Zeroizing;

use crate::traits::{ServiceStore, StorageProvider};

#[allow(clippy::unwrap_used)]
static SERVICE_ID_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-zA-Z0-9_\-]{1,128}$").unwrap());

const SUBSTRATE_SCHEMA_VERSION: &str = "m3a";

/// SqliteStorageProvider manages the substrate.db (metadata) and per-service
/// encrypted databases.
pub struct SqliteStorageProvider {
    db_dir: PathBuf,
    substrate_conn: Arc<Mutex<rusqlite::Connection>>,
    service_stores: Arc<Mutex<HashMap<String, Arc<SqliteServiceStore>>>>,
    encryption_enabled: bool,
}

impl std::fmt::Debug for SqliteStorageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&db_dir)?;
            }
        }

        let substrate_path = db_dir.join("substrate.db");
        let mut conn = rusqlite::Connection::open(&substrate_path)?;

        conn.execute("CREATE TABLE IF NOT EXISTS schema_version (version TEXT NOT NULL)", [])?;

        let stored_version: Option<String> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
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

    fn run_m3a_migration(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    /// Internal helper to obtain KEK status and print warnings.
    fn verify_encryption_mode(&self, key_store: &Arc<KeyStore>) -> anyhow::Result<()> {
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
}

#[allow(dead_code)]
enum DbCommand {
    WriteSecret {
        key: String,
        secret_bytes: Vec<u8>,
        resp: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    RevealSecret {
        key: String,
        resp: tokio::sync::oneshot::Sender<anyhow::Result<Option<Vec<u8>>>>,
    },
    CreateCollection {
        resp: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    DropCollection {
        resp: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    ExecuteDdl {
        resp: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
}

fn data_layer_not_implemented() -> anyhow::Error {
    anyhow::anyhow!("data-layer CRUD host functions are not implemented until Slice 3A")
}

fn run_writer_loop(
    conn: rusqlite::Connection,
    mut rx: tokio::sync::mpsc::Receiver<DbCommand>,
    dek: Zeroizing<[u8; 32]>,
) {
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            DbCommand::WriteSecret { key, secret_bytes, resp } => {
                // Encrypt secret bytes with DEK (AES-256-GCM)
                let aes_key = Key::<Aes256Gcm>::from_slice(&*dek);
                let cipher = Aes256Gcm::new(aes_key);

                let mut nonce_bytes = [0u8; 12];
                rand::rng().fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ciphertext_res = cipher
                    .encrypt(nonce, secret_bytes.as_slice())
                    .map_err(|e| anyhow::anyhow!("Encryption failure: {}", e));

                let res = match ciphertext_res {
                    Ok(ciphertext) => {
                        let now = chrono::Utc::now().timestamp_millis();
                        conn.execute(
                            "INSERT OR REPLACE INTO _vault (key, ciphertext, nonce, updated_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![key, ciphertext, nonce_bytes.as_slice(), now],
                        )
                        .map(|_| ())
                        .map_err(|e| e.into())
                    }
                    Err(e) => Err(e),
                };
                let _ = resp.send(res);
            }
            DbCommand::RevealSecret { key, resp } => {
                let res = (|| -> anyhow::Result<Option<Vec<u8>>> {
                    let mut stmt =
                        conn.prepare("SELECT ciphertext, nonce FROM _vault WHERE key = ?1")?;
                    let mut rows = stmt.query(rusqlite::params![key])?;

                    if let Some(row) = rows.next()? {
                        let ciphertext: Vec<u8> = row.get(0)?;
                        let nonce_bytes: Vec<u8> = row.get(1)?;

                        if nonce_bytes.len() != 12 {
                            return Err(anyhow::anyhow!("Invalid stored nonce length"));
                        }

                        // Decrypt
                        let aes_key = Key::<Aes256Gcm>::from_slice(&*dek);
                        let cipher = Aes256Gcm::new(aes_key);
                        let nonce = Nonce::from_slice(&nonce_bytes);

                        let decrypted = cipher
                            .decrypt(nonce, ciphertext.as_slice())
                            .map_err(|e| anyhow::anyhow!("Decryption failure: {}", e))?;

                        Ok(Some(decrypted))
                    } else {
                        Ok(None)
                    }
                })();
                let _ = resp.send(res);
            }
            DbCommand::CreateCollection { resp } => {
                let _ = resp.send(Err(data_layer_not_implemented()));
            }
            DbCommand::DropCollection { resp } => {
                let _ = resp.send(Err(data_layer_not_implemented()));
            }
            DbCommand::ExecuteDdl { resp } => {
                let _ = resp.send(Err(data_layer_not_implemented()));
            }
        }
    }
}

#[async_trait]
impl StorageProvider for SqliteStorageProvider {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>> {
        // Validate service_id
        if !SERVICE_ID_REGEX.is_match(service_id) {
            return Err(anyhow::anyhow!("Invalid service ID format: {}", service_id));
        }

        // Validate path traversal
        let services_dir = self.db_dir.join("services");
        let service_db_dir = services_dir.join(service_id);

        // Ensure path does not escape services_dir
        if !service_db_dir.starts_with(&services_dir) {
            return Err(anyhow::anyhow!(
                "Path traversal attempt rejected for service ID: {}",
                service_id
            ));
        }

        // Verify encryption requirements
        self.verify_encryption_mode(key_store)?;

        if let Some(store) = self
            .service_stores
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?
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
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&service_db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&service_db_dir)?;
            }
        }

        let db_file_path = service_db_dir.join("state.db");

        // Resolve or generate DEK
        let dek = if self.encryption_enabled {
            let conn =
                self.substrate_conn.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            match key_store.load_dek(service_id, &conn) {
                Ok(bytes) => Zeroizing::new(bytes),
                Err(syneroym_key_store::KeyStoreError::Database(
                    rusqlite::Error::QueryReturnedNoRows,
                )) => {
                    // Generate new DEK
                    Zeroizing::new(key_store.generate_dek(service_id, &conn)?)
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            Zeroizing::new([0u8; 32]) // encryption disabled, use dummy KEK/DEK
        };

        // Open service DB connection for the single writer actor
        let writer_conn = rusqlite::Connection::open(&db_file_path)?;
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
        let (writer_tx, writer_rx) = tokio::sync::mpsc::channel(100);
        let dek_writer = dek.clone();
        tokio::task::spawn_blocking(move || {
            run_writer_loop(writer_conn, writer_rx, dek_writer);
        });

        // Initialize reader pool. Vault reads stay on the already-keyed writer;
        // this pool is reserved for future non-sensitive reads.
        let cfg = deadpool_sqlite::Config::new(&db_file_path);
        let mut reader_pool_builder = cfg.builder(deadpool_sqlite::Runtime::Tokio1)?;
        if self.encryption_enabled {
            let reader_dek = dek.clone();
            reader_pool_builder = reader_pool_builder.post_create(deadpool_sqlite::Hook::async_fn(
                move |conn, _metrics| {
                    let reader_dek = reader_dek.clone();
                    Box::pin(async move {
                        let pragma_val = Zeroizing::new(format!("x'{}'", hex::encode(*reader_dek)));
                        conn.interact(move |conn| conn.pragma_update(None, "key", &*pragma_val))
                            .await
                            .map_err(|e| {
                                deadpool_sqlite::HookError::message(format!(
                                    "Interact error: {}",
                                    e
                                ))
                            })?
                            .map_err(deadpool_sqlite::HookError::Backend)?;
                        Ok(())
                    })
                },
            ));
        }
        let reader_pool = reader_pool_builder.build()?;

        let new_store = Arc::new(SqliteServiceStore { _reader_pool: reader_pool, writer_tx });
        let store = {
            let mut stores =
                self.service_stores.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            stores.entry(service_id.to_string()).or_insert_with(|| new_store.clone()).clone()
        };

        Ok(Box::new(store))
    }

    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let ks = key_store.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            ks.rotate_kek(new_kek, &mut conn)?;
            Ok(())
        })
        .await?
    }
}

pub struct SqliteServiceStore {
    _reader_pool: deadpool_sqlite::Pool,
    writer_tx: tokio::sync::mpsc::Sender<DbCommand>,
}

impl std::fmt::Debug for SqliteServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteServiceStore").field("dek", &"<redacted>").finish()
    }
}

#[async_trait]
impl ServiceStore for SqliteServiceStore {
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.writer_tx
            .send(DbCommand::WriteSecret {
                key: key.to_string(),
                secret_bytes: secret_bytes.to_vec(),
                resp: resp_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.writer_tx
            .send(DbCommand::RevealSecret { key: key.to_string(), resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn create_collection(&self, _name: &str) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.writer_tx
            .send(DbCommand::CreateCollection { resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn drop_collection(&self, _name: &str) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.writer_tx
            .send(DbCommand::DropCollection { resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn execute_ddl(&self, _sql: &str) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.writer_tx
            .send(DbCommand::ExecuteDdl { resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }
}

#[async_trait]
impl ServiceStore for Arc<SqliteServiceStore> {
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()> {
        self.as_ref().write_secret(key, secret_bytes).await
    }

    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.as_ref().reveal_secret(key).await
    }

    async fn create_collection(&self, name: &str) -> anyhow::Result<()> {
        self.as_ref().create_collection(name).await
    }

    async fn drop_collection(&self, name: &str) -> anyhow::Result<()> {
        self.as_ref().drop_collection(name).await
    }

    async fn execute_ddl(&self, sql: &str) -> anyhow::Result<()> {
        self.as_ref().execute_ddl(sql).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_startup_migrates_previous_substrate_schema_to_m3a() {
        let dir = tempdir().unwrap();
        let substrate_path = dir.path().join("substrate.db");
        {
            let conn = rusqlite::Connection::open(&substrate_path).unwrap();
            conn.execute(
                "CREATE TABLE schema_version (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    version TEXT NOT NULL
                )",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO schema_version (id, version) VALUES (1, 'm2')", []).unwrap();
        }

        let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

        let conn = rusqlite::Connection::open(&substrate_path).unwrap();
        let version: String = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SUBSTRATE_SCHEMA_VERSION);

        let dek_store_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'dek_store'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dek_store_count, 1);
    }

    #[test]
    fn test_startup_uses_documented_schema_version_shape() {
        let dir = tempdir().unwrap();
        let substrate_path = dir.path().join("substrate.db");

        let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

        let conn = rusqlite::Connection::open(&substrate_path).unwrap();
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(schema_version)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };

        assert_eq!(columns, vec!["version"]);
    }

    #[tokio::test]
    async fn test_service_id_validation_and_path_traversal() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Valid ID
        assert!(provider.open_service_db("svc-1", &key_store).await.is_ok());
        assert!(provider.open_service_db("my_service-2", &key_store).await.is_ok());

        // Invalid IDs
        assert!(provider.open_service_db("svc/../../traversal", &key_store).await.is_err());
        assert!(provider.open_service_db("svc_with_spaces ", &key_store).await.is_err());
        assert!(provider.open_service_db("svc!", &key_store).await.is_err());
        assert!(provider.open_service_db("", &key_store).await.is_err());
    }

    #[tokio::test]
    async fn test_encryption_key_required() {
        let dir = tempdir().unwrap();
        // Encryption enabled
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Fails with EncryptionKeyRequired because KEK is not injected
        let res = provider.open_service_db("my-service", &key_store).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap().to_string(), "EncryptionKeyRequired");

        // Inject KEK
        key_store.inject_kek([9u8; 32], None).unwrap();

        // Now succeeds
        assert!(provider.open_service_db("my-service", &key_store).await.is_ok());
    }

    #[tokio::test]
    async fn test_vault_write_and_reveal() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([15u8; 32], None).unwrap();

        let store = provider.open_service_db("vault-test", &key_store).await.unwrap();

        // Reveal missing key returns None
        assert_eq!(store.reveal_secret("api_key").await.unwrap(), None);

        // Write secret
        let secret = b"super-secret-token-123";
        store.write_secret("api_key", secret).await.unwrap();

        // Reveal secret
        let revealed = store.reveal_secret("api_key").await.unwrap();
        assert_eq!(revealed, Some(secret.to_vec()));
    }

    #[tokio::test]
    async fn test_open_service_db_reuses_cached_store_actor() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([17u8; 32], None).unwrap();

        let first = provider.open_service_db("cached-vault", &key_store).await.unwrap();
        let second = provider.open_service_db("cached-vault", &key_store).await.unwrap();

        first.write_secret("api_key", b"cached-store-secret").await.unwrap();
        let revealed = second.reveal_secret("api_key").await.unwrap();

        assert_eq!(revealed, Some(b"cached-store-secret".to_vec()));
        assert_eq!(provider.service_stores.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_slice_3a_data_layer_methods_are_not_successful_noops() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());
        let store = provider.open_service_db("slice-3a-pending", &key_store).await.unwrap();

        let err = store.create_collection("profiles").await.unwrap_err();
        assert!(err.to_string().contains("Slice 3A"));
        let err = store.drop_collection("profiles").await.unwrap_err();
        assert!(err.to_string().contains("Slice 3A"));
        let err = store.execute_ddl("CREATE TABLE profiles(id TEXT)").await.unwrap_err();
        assert!(err.to_string().contains("Slice 3A"));
    }

    #[tokio::test]
    async fn test_path_traversal_etc_passwd() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Explicitly assert path traversal reject
        assert!(provider.open_service_db("../../etc/passwd", &key_store).await.is_err());
    }

    #[tokio::test]
    async fn test_restart_survival() {
        let dir = tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([42u8; 32], None).unwrap();

        // Write data on first boot
        {
            let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
            let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
            store.write_secret("secret_key", b"survival-data-100").await.unwrap();
        }

        // Read data after "restart" (re-instantiating SqliteStorageProvider)
        {
            let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
            let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
            let revealed = store.reveal_secret("secret_key").await.unwrap();
            assert_eq!(revealed, Some(b"survival-data-100".to_vec()));
        }
    }

    #[test]
    fn test_insecure_mode_warning() {
        use tracing_subscriber::prelude::*;

        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        struct MockWriter {
            logs: Arc<Mutex<Vec<u8>>>,
        }
        impl std::io::Write for MockWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.logs.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer);

        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            provider.verify_encryption_mode(&key_store).unwrap();
        });

        let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(logs_content.contains("INSECURE: storage encryption is disabled"));
    }
}

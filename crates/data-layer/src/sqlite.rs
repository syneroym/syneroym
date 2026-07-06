use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use async_trait::async_trait;
use rand::RngCore;
use rusqlite::params;
use syneroym_key_store::KeyStore;
use tokio::{sync::oneshot, task};
use zeroize::Zeroizing;

use crate::{
    errors::map_rusqlite_error,
    filter, host_store,
    traits::{ServiceStore, StorageProvider},
};

// Real service ids are DIDs (e.g. `did:key:h7wy...`), which contain colons;
// `:` is not a path separator on any Rust-supported OS, so allowing it here
// does not weaken the path-traversal guard below (which still relies on
// rejecting `.`/`/`/`\` and on the `starts_with` descendant check).
#[allow(clippy::unwrap_used)]
static SERVICE_ID_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-zA-Z0-9_:\-]{1,128}$").unwrap());

#[allow(clippy::unwrap_used)]
static IDENTIFIER_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]{0,63}$").unwrap());

const SUBSTRATE_SCHEMA_VERSION: &str = "m3a";

/// Hard upper bound on records returned per `query` page, enforced
/// regardless of what the guest requests via `query-options.limit`.
pub const MAX_QUERY_PAGE_SIZE: u32 = 1000;

/// Hard upper bound on the number of mutations accepted by a single
/// `batch-mutate` call, to bound how long the single-writer actor is
/// occupied by one request.
pub const MAX_BATCH_SIZE: usize = 200;

/// Validates a guest-supplied identifier (collection/table name, index
/// field name) before it is formatted into SQL text. Table and column names
/// cannot be bound as SQL parameters, so this allow-list is what stands in
/// for parameterization at the DDL boundary.
fn validate_identifier(name: &str) -> Result<(), host_store::DataLayerError> {
    if IDENTIFIER_REGEX.is_match(name) {
        Ok(())
    } else {
        Err(host_store::DataLayerError::SchemaViolation(format!("invalid identifier: {name}")))
    }
}

/// Applies an RFC 7396 JSON merge-patch: `patch` values overwrite `target`,
/// `null` values remove the key, and nested objects merge recursively.
fn apply_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let serde_json::Value::Object(patch_obj) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::Value::Object(serde_json::Map::new());
    }
    #[allow(clippy::expect_used)]
    let target_obj = target.as_object_mut().expect("target was just coerced into an object");
    for (key, value) in patch_obj {
        if value.is_null() {
            target_obj.remove(key);
        } else {
            let entry = target_obj.entry(key.clone()).or_insert(serde_json::Value::Null);
            apply_merge_patch(entry, value);
        }
    }
}

fn payload_to_text(payload: &[u8]) -> Result<String, host_store::DataLayerError> {
    let text = std::str::from_utf8(payload).map_err(|_| {
        host_store::DataLayerError::SchemaViolation("payload must be valid UTF-8".into())
    })?;
    serde_json::from_str::<serde_json::Value>(text).map_err(|e| {
        host_store::DataLayerError::SchemaViolation(format!("payload is not valid JSON: {e}"))
    })?;
    Ok(text.to_string())
}

fn do_create_collection(
    conn: &rusqlite::Connection,
    schema: &host_store::CollectionSchema,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(&schema.name)?;
    for idx in &schema.indexes {
        validate_identifier(&idx.field_name)?;
    }
    conn.execute(
        &format!(
            "CREATE TABLE IF NOT EXISTS {} (id TEXT PRIMARY KEY, payload JSON NOT NULL, \
             creator_id TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            schema.name
        ),
        [],
    )
    .map_err(map_rusqlite_error)?;
    for idx in &schema.indexes {
        let index_name = format!("idx_{}_{}", schema.name, idx.field_name);
        conn.execute(
            &format!(
                "CREATE INDEX IF NOT EXISTS {index_name} ON {}(json_extract(payload, '$.{}'))",
                schema.name, idx.field_name
            ),
            [],
        )
        .map_err(map_rusqlite_error)?;
    }
    Ok(())
}

fn do_drop_collection(
    conn: &rusqlite::Connection,
    name: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(name)?;
    conn.execute(&format!("DROP TABLE IF EXISTS {name}"), []).map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_execute_ddl(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<(), host_store::DataLayerError> {
    // Syntax-check first via a plain `prepare` (compiles without stepping the
    // statement, so nothing is mutated), then run the real statement(s).
    // NOTE: only the leading statement of a multi-statement `sql` is checked
    // this way; `execute_batch` below still validates the full batch.
    conn.prepare(&format!("EXPLAIN {sql}")).map_err(|e| {
        host_store::DataLayerError::Internal(format!("DDL syntax check failed: {e}"))
    })?;
    conn.execute_batch(sql)
        .map_err(|e| host_store::DataLayerError::Internal(format!("DDL execution failed: {e}")))?;
    Ok(())
}

fn do_put(
    conn: &rusqlite::Connection,
    collection: &str,
    value: &host_store::RecordWriteValue,
    creator_id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let payload_text = payload_to_text(&value.payload)?;
    let now = chrono::Utc::now().timestamp_millis();

    let existing_created_at: Option<i64> = conn
        .query_row(
            &format!("SELECT created_at FROM {collection} WHERE id = ?1"),
            params![value.id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(map_rusqlite_error)?;
    let created_at = existing_created_at.unwrap_or(now);

    conn.execute(
        &format!(
            "INSERT INTO {collection} (id, payload, creator_id, created_at, updated_at) VALUES \
             (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, \
             creator_id = excluded.creator_id, updated_at = excluded.updated_at"
        ),
        params![value.id, payload_text, creator_id, created_at, now],
    )
    .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_patch(
    conn: &rusqlite::Connection,
    collection: &str,
    id: &str,
    patch_json: &[u8],
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let existing_payload: String = conn
        .query_row(&format!("SELECT payload FROM {collection} WHERE id = ?1"), params![id], |row| {
            row.get(0)
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => host_store::DataLayerError::SchemaViolation(
                format!("record not found for patch: {id}"),
            ),
            other => map_rusqlite_error(other),
        })?;

    let mut target: serde_json::Value = serde_json::from_str(&existing_payload).map_err(|e| {
        host_store::DataLayerError::Internal(format!("stored payload is not valid JSON: {e}"))
    })?;
    let patch_text = std::str::from_utf8(patch_json).map_err(|_| {
        host_store::DataLayerError::SchemaViolation("patch-json must be valid UTF-8".into())
    })?;
    let patch_doc: serde_json::Value = serde_json::from_str(patch_text).map_err(|e| {
        host_store::DataLayerError::SchemaViolation(format!("patch-json is not valid JSON: {e}"))
    })?;
    apply_merge_patch(&mut target, &patch_doc);
    let merged_text = serde_json::to_string(&target)
        .map_err(|e| host_store::DataLayerError::Internal(e.to_string()))?;

    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        &format!("UPDATE {collection} SET payload = ?1, updated_at = ?2 WHERE id = ?3"),
        params![merged_text, now, id],
    )
    .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_delete(
    conn: &rusqlite::Connection,
    collection: &str,
    id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    // Idempotent: deleting a non-existent id is not an error, only a
    // non-existent collection is (surfaced via map_rusqlite_error below).
    conn.execute(&format!("DELETE FROM {collection} WHERE id = ?1"), params![id])
        .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_delete_many(
    conn: &rusqlite::Connection,
    collection: &str,
    filter_json: Option<&str>,
) -> Result<u64, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let compiled = filter::compile_filter(filter_json)?;
    let (where_sql, bound_params) = match &compiled {
        Some(cf) => (format!("WHERE {}", cf.where_clause), cf.params.clone()),
        None => (String::new(), Vec::new()),
    };
    let affected = conn
        .execute(
            &format!("DELETE FROM {collection} {where_sql}"),
            rusqlite::params_from_iter(bound_params.iter()),
        )
        .map_err(map_rusqlite_error)?;
    Ok(affected as u64)
}

fn do_batch_mutate(
    conn: &mut rusqlite::Connection,
    collection: &str,
    mutations: &[host_store::Mutation],
    creator_id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    if mutations.len() > MAX_BATCH_SIZE {
        return Err(host_store::DataLayerError::SchemaViolation(format!(
            "batch exceeds MAX_BATCH_SIZE ({MAX_BATCH_SIZE})"
        )));
    }
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    for mutation in mutations {
        match mutation {
            host_store::Mutation::Put(value) => do_put(&tx, collection, value, creator_id)?,
            host_store::Mutation::Patch(patch_mutation) => {
                do_patch(&tx, collection, &patch_mutation.id, &patch_mutation.patch_json)?
            }
            host_store::Mutation::Delete(id) => do_delete(&tx, collection, id)?,
        }
    }
    tx.commit().map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_get(
    conn: &rusqlite::Connection,
    collection: &str,
    id: &str,
) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let result = conn.query_row(
        &format!(
            "SELECT payload, creator_id, created_at, updated_at FROM {collection} WHERE id = ?1"
        ),
        params![id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    );
    match result {
        Ok((payload, creator_id, created_at, updated_at)) => {
            Ok(Some(host_store::RecordReadValue {
                id: id.to_string(),
                payload: payload.into_bytes(),
                creator_id,
                created_at: created_at as u64,
                updated_at: updated_at as u64,
            }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(map_rusqlite_error(e)),
    }
}

fn do_query(
    conn: &rusqlite::Connection,
    collection: &str,
    opts: &host_store::QueryOptions,
) -> Result<host_store::QueryResult, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let compiled = filter::compile_filter(opts.filter.as_deref())?;
    let limit = opts.limit.unwrap_or(MAX_QUERY_PAGE_SIZE).min(MAX_QUERY_PAGE_SIZE);

    let mut where_clauses = Vec::new();
    let mut bound_params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(cf) = &compiled {
        where_clauses.push(cf.where_clause.clone());
        bound_params.extend(cf.params.iter().cloned());
    }
    if let Some(cursor) = &opts.cursor {
        where_clauses.push("id > ?".to_string());
        bound_params.push(rusqlite::types::Value::Text(cursor.clone()));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    // Fetch one extra row past the page limit to determine whether a
    // next-cursor should be returned.
    bound_params.push(rusqlite::types::Value::Integer(i64::from(limit) + 1));

    let sql = format!(
        "SELECT id, payload, creator_id, created_at, updated_at FROM {collection} {where_sql} \
         ORDER BY id ASC LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql).map_err(map_rusqlite_error)?;
    let mut records = stmt
        .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
            Ok(host_store::RecordReadValue {
                id: row.get::<_, String>(0)?,
                payload: row.get::<_, String>(1)?.into_bytes(),
                creator_id: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
            })
        })
        .map_err(map_rusqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_rusqlite_error)?;

    let next_cursor = if records.len() as u32 > limit {
        records.truncate(limit as usize);
        records.last().map(|r| r.id.clone())
    } else {
        None
    };
    Ok(host_store::QueryResult { records, next_cursor })
}

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

    /// Validates `service_id` and resolves its expected per-service database
    /// directory, guarding against path traversal. Does not touch the
    /// filesystem.
    fn resolve_service_db_dir(&self, service_id: &str) -> anyhow::Result<PathBuf> {
        if !SERVICE_ID_REGEX.is_match(service_id) {
            return Err(anyhow::anyhow!("Invalid service ID format: {}", service_id));
        }
        let services_dir = self.db_dir.join("services");
        let service_db_dir = services_dir.join(service_id);
        if !service_db_dir.starts_with(&services_dir) {
            return Err(anyhow::anyhow!(
                "Path traversal attempt rejected for service ID: {}",
                service_id
            ));
        }
        Ok(service_db_dir)
    }
}

enum DbCommand {
    WriteSecret {
        key: String,
        secret_bytes: Vec<u8>,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
    RevealSecret {
        key: String,
        resp: oneshot::Sender<anyhow::Result<Option<Vec<u8>>>>,
    },
    CreateCollection {
        schema: host_store::CollectionSchema,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    DropCollection {
        name: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    ExecuteDdl {
        sql: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Put {
        collection: String,
        value: host_store::RecordWriteValue,
        creator_id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Patch {
        collection: String,
        id: String,
        patch_json: Vec<u8>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Delete {
        collection: String,
        id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    DeleteMany {
        collection: String,
        filter: Option<String>,
        resp: oneshot::Sender<Result<u64, host_store::DataLayerError>>,
    },
    BatchMutate {
        collection: String,
        mutations: Vec<host_store::Mutation>,
        creator_id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
}

fn run_writer_loop(
    mut conn: rusqlite::Connection,
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
            DbCommand::CreateCollection { schema, resp } => {
                let _ = resp.send(do_create_collection(&conn, &schema));
            }
            DbCommand::DropCollection { name, resp } => {
                let _ = resp.send(do_drop_collection(&conn, &name));
            }
            DbCommand::ExecuteDdl { sql, resp } => {
                let _ = resp.send(do_execute_ddl(&conn, &sql));
            }
            DbCommand::Put { collection, value, creator_id, resp } => {
                let _ = resp.send(do_put(&conn, &collection, &value, &creator_id));
            }
            DbCommand::Patch { collection, id, patch_json, resp } => {
                let _ = resp.send(do_patch(&conn, &collection, &id, &patch_json));
            }
            DbCommand::Delete { collection, id, resp } => {
                let _ = resp.send(do_delete(&conn, &collection, &id));
            }
            DbCommand::DeleteMany { collection, filter, resp } => {
                let _ = resp.send(do_delete_many(&conn, &collection, filter.as_deref()));
            }
            DbCommand::BatchMutate { collection, mutations, creator_id, resp } => {
                let _ = resp.send(do_batch_mutate(&mut conn, &collection, &mutations, &creator_id));
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
        let service_db_dir = self.resolve_service_db_dir(service_id)?;

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
        task::spawn_blocking(move || {
            run_writer_loop(writer_conn, writer_rx, dek_writer);
        });

        // Initialize reader pool, used for all read-only operations (vault
        // reveal reads stay on the writer above for simplicity; CRUD reads
        // use this pool for concurrency).
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

        let new_store = Arc::new(SqliteServiceStore { reader_pool, writer_tx });
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
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            ks.rotate_kek(new_kek, &mut conn)?;
            Ok(())
        })
        .await?
    }

    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        let service_db_dir = self.resolve_service_db_dir(service_id)?;
        Ok(service_db_dir.join("state.db").exists())
    }
}

pub struct SqliteServiceStore {
    reader_pool: deadpool_sqlite::Pool,
    writer_tx: tokio::sync::mpsc::Sender<DbCommand>,
}

impl std::fmt::Debug for SqliteServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteServiceStore").field("dek", &"<redacted>").finish()
    }
}

/// Sends a write command over the single-writer channel and awaits its
/// response, flattening channel-disconnect failures into a `DataLayerError`.
async fn send_write_command<T>(
    writer_tx: &tokio::sync::mpsc::Sender<DbCommand>,
    build: impl FnOnce(oneshot::Sender<Result<T, host_store::DataLayerError>>) -> DbCommand,
) -> Result<T, host_store::DataLayerError> {
    let (resp_tx, resp_rx) = oneshot::channel();
    writer_tx.send(build(resp_tx)).await.map_err(|_| {
        host_store::DataLayerError::Internal("writer task disconnected".to_string())
    })?;
    resp_rx
        .await
        .map_err(|_| host_store::DataLayerError::Internal("writer task disconnected".to_string()))?
}

#[async_trait]
impl ServiceStore for SqliteServiceStore {
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
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
        let (resp_tx, resp_rx) = oneshot::channel();
        self.writer_tx
            .send(DbCommand::RevealSecret { key: key.to_string(), resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn create_collection(
        &self,
        schema: &host_store::CollectionSchema,
    ) -> Result<(), host_store::DataLayerError> {
        let schema = schema.clone();
        send_write_command(&self.writer_tx, |resp| DbCommand::CreateCollection { schema, resp })
            .await
    }

    async fn drop_collection(&self, name: &str) -> Result<(), host_store::DataLayerError> {
        let name = name.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::DropCollection { name, resp }).await
    }

    async fn execute_ddl(&self, sql: &str) -> Result<(), host_store::DataLayerError> {
        let sql = sql.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::ExecuteDdl { sql, resp }).await
    }

    async fn put(
        &self,
        collection: &str,
        value: &host_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let value = value.clone();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Put {
            collection,
            value,
            creator_id,
            resp,
        })
        .await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        let patch_json = patch_json.to_vec();
        send_write_command(&self.writer_tx, |resp| DbCommand::Patch {
            collection,
            id,
            patch_json,
            resp,
        })
        .await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_get(conn, &collection, &id)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
    ) -> Result<host_store::QueryResult, host_store::DataLayerError> {
        let collection = collection.to_string();
        let opts = opts.clone();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_query(conn, &collection, &opts)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Delete { collection, id, resp }).await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, host_store::DataLayerError> {
        let collection = collection.to_string();
        let filter = filter.map(str::to_string);
        send_write_command(&self.writer_tx, |resp| DbCommand::DeleteMany {
            collection,
            filter,
            resp,
        })
        .await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let mutations = mutations.to_vec();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::BatchMutate {
            collection,
            mutations,
            creator_id,
            resp,
        })
        .await
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

    async fn create_collection(
        &self,
        schema: &host_store::CollectionSchema,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().create_collection(schema).await
    }

    async fn drop_collection(&self, name: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().drop_collection(name).await
    }

    async fn execute_ddl(&self, sql: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().execute_ddl(sql).await
    }

    async fn put(
        &self,
        collection: &str,
        value: &host_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().put(collection, value, creator_id).await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().patch(collection, id, patch_json).await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
        self.as_ref().get(collection, id).await
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
    ) -> Result<host_store::QueryResult, host_store::DataLayerError> {
        self.as_ref().query(collection, opts).await
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().delete(collection, id).await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, host_store::DataLayerError> {
        self.as_ref().delete_many(collection, filter).await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().batch_mutate(collection, mutations, creator_id).await
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
        // Real service ids are DIDs and contain colons.
        assert!(
            provider
                .open_service_db(
                    "did:key:h7wy4ppo5gystkfs71hf19qhmbaqc3yx7gpcbtg4s9h6ojozbgx61nco",
                    &key_store
                )
                .await
                .is_ok()
        );

        // Invalid IDs
        assert!(provider.open_service_db("svc/../../traversal", &key_store).await.is_err());
        assert!(provider.open_service_db("did:key:../../traversal", &key_store).await.is_err());
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

    #[tokio::test]
    async fn test_service_exists_reflects_persistent_db_state() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        assert!(!provider.service_exists("svc-a").await.unwrap());
        let _ = provider.open_service_db("svc-a", &key_store).await.unwrap();
        assert!(provider.service_exists("svc-a").await.unwrap());
    }
}

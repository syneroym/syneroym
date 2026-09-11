use std::{fmt, sync::Arc};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use async_trait::async_trait;
use chrono::Utc;
use deadpool_sqlite::Pool;
use rand::RngCore;
use rusqlite::Connection;
use syneroym_fdae::{CompiledSieve, Mode, compile_read};
use syneroym_ucan::Ability;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use super::{
    mutation::{
        do_authorized_delete, do_authorized_patch, do_authorized_put, do_batch_mutate,
        do_delete_many,
    },
    query::{do_aggregate, do_check_access, do_get, do_list_collections, do_query},
    query_raw::do_query_raw,
    schema::{VAULT_TABLE, do_create_collection, do_drop_collection, do_execute_ddl},
    sieve::{compile_sieve_for, compile_sieve_for_op, sieve_masked_fields},
};
use crate::{
    auth::{QueryAuth, ReadOutcome},
    host_store,
    traits::ServiceStore,
};

pub(super) enum DbCommand {
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
        // Boxed for the same `clippy::large_enum_variant` reason `DeleteMany`
        // already documents below.
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Patch {
        collection: String,
        id: String,
        patch_json: Vec<u8>,
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Delete {
        collection: String,
        id: String,
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    DeleteMany {
        collection: String,
        filter: Option<String>,
        // Boxed: `CompiledSieve` (now carrying a `DecisionTrace`) made this
        // the largest `DbCommand` variant by a wide margin --
        // `clippy::large_enum_variant` flags every other variant paying its
        // stack size on every `DbCommand` value.
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<u64, host_store::DataLayerError>>,
    },
    BatchMutate {
        collection: String,
        mutations: Vec<host_store::Mutation>,
        creator_id: String,
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
}

pub(super) fn run_writer_loop(
    mut conn: Connection,
    mut rx: mpsc::Receiver<DbCommand>,
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
                    .map_err(|e| anyhow::anyhow!("Encryption failure: {e}"));

                let res = match ciphertext_res {
                    Ok(ciphertext) => {
                        let now = Utc::now().timestamp_millis();
                        conn.execute(
                            &format!(
                                "INSERT OR REPLACE INTO {VAULT_TABLE} (key, ciphertext, nonce, \
                                 updated_at)
                             VALUES (?1, ?2, ?3, ?4)"
                            ),
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
                    let mut stmt = conn.prepare(&format!(
                        "SELECT ciphertext, nonce FROM {VAULT_TABLE} WHERE key = ?1"
                    ))?;
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
                            .map_err(|e| anyhow::anyhow!("Decryption failure: {e}"))?;

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
            DbCommand::Put { collection, value, creator_id, sieve, resp } => {
                let _ = resp.send(do_authorized_put(
                    &mut conn,
                    &collection,
                    &value,
                    &creator_id,
                    sieve.as_deref(),
                ));
            }
            DbCommand::Patch { collection, id, patch_json, sieve, resp } => {
                let _ = resp.send(do_authorized_patch(
                    &mut conn,
                    &collection,
                    &id,
                    &patch_json,
                    sieve.as_deref(),
                ));
            }
            DbCommand::Delete { collection, id, sieve, resp } => {
                let _ =
                    resp.send(do_authorized_delete(&mut conn, &collection, &id, sieve.as_deref()));
            }
            DbCommand::DeleteMany { collection, filter, sieve, resp } => {
                let _ = resp.send(do_delete_many(
                    &conn,
                    &collection,
                    filter.as_deref(),
                    sieve.as_deref(),
                ));
            }
            DbCommand::BatchMutate { collection, mutations, creator_id, sieve, resp } => {
                let _ = resp.send(do_batch_mutate(
                    &mut conn,
                    &collection,
                    &mutations,
                    &creator_id,
                    sieve.as_deref(),
                ));
            }
        }
    }
}

pub struct SqliteServiceStore {
    pub(super) reader_pool: Pool,
    pub(super) writer_tx: mpsc::Sender<DbCommand>,
}

impl fmt::Debug for SqliteServiceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteServiceStore").field("dek", &"<redacted>").finish()
    }
}

/// Sends a write command over the single-writer channel and awaits its
/// response, flattening channel-disconnect failures into a `DataLayerError`.
async fn send_write_command<T>(
    writer_tx: &mpsc::Sender<DbCommand>,
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
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        let sieve =
            compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
                .map(Box::new);
        let collection = collection.to_string();
        let value = value.clone();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Put {
            collection,
            value,
            creator_id,
            sieve,
            resp,
        })
        .await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        let sieve =
            compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
                .map(Box::new);
        let collection = collection.to_string();
        let id = id.to_string();
        let patch_json = patch_json.to_vec();
        send_write_command(&self.writer_tx, |resp| DbCommand::Patch {
            collection,
            id,
            patch_json,
            sieve,
            resp,
        })
        .await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<ReadOutcome<Option<host_store::RecordReadValue>>, host_store::DataLayerError> {
        let sieve = compile_sieve_for(auth, collection, Mode::PointInTime { id: id.to_string() })?;
        let masked_fields = sieve_masked_fields(&sieve);
        let collection = collection.to_string();
        let id = id.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        let value = conn
            .interact(move |conn| do_get(conn, &collection, &id, sieve.as_ref()))
            .await
            .map_err(|e| {
                host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
            })??;
        Ok(ReadOutcome { value, masked_fields })
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<ReadOutcome<host_store::QueryResult>, host_store::DataLayerError> {
        let sieve = compile_sieve_for(auth, collection, Mode::Filter)?;
        let masked_fields = sieve_masked_fields(&sieve);
        let collection = collection.to_string();
        let opts = opts.clone();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        let value = conn
            .interact(move |conn| do_query(conn, &collection, &opts, sieve.as_ref()))
            .await
            .map_err(|e| {
                host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
            })??;
        Ok(ReadOutcome { value, masked_fields })
    }

    async fn aggregate(
        &self,
        collection: &str,
        pipeline: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        let sieve = compile_sieve_for(auth, collection, Mode::Filter)?;
        let collection = collection.to_string();
        let pipeline = pipeline.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_aggregate(conn, &collection, &pipeline, sieve.as_ref()))
            .await
            .map_err(|e| {
                host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
            })?
    }

    async fn delete(
        &self,
        collection: &str,
        id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        let sieve =
            compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
                .map(Box::new);
        let collection = collection.to_string();
        let id = id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Delete {
            collection,
            id,
            sieve,
            resp,
        })
        .await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<u64, host_store::DataLayerError> {
        // Deleting is a write (D2): a read-only permission's `paths` must not
        // become "these rows are deletable" through this path.
        let sieve =
            compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
                .map(Box::new);
        let collection = collection.to_string();
        let filter = filter.map(str::to_string);
        send_write_command(&self.writer_tx, |resp| DbCommand::DeleteMany {
            collection,
            filter,
            sieve,
            resp,
        })
        .await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        let sieve =
            compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
                .map(Box::new);
        let collection = collection.to_string();
        let mutations = mutations.to_vec();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::BatchMutate {
            collection,
            mutations,
            creator_id,
            sieve,
            resp,
        })
        .await
    }

    async fn query_raw(
        &self,
        sql: &str,
        params: &[host_store::SqlValue],
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        let sql = sql.to_string();
        let params = params.to_vec();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_query_raw(conn, &sql, &params)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }

    async fn check_access(
        &self,
        collection: &str,
        id: &str,
        operation: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<bool, host_store::DataLayerError> {
        // Fail-closed (ADR-0017 §4): a malformed policy must never be
        // mistaken for "allowed", so a `PolicyError` here returns `Ok(false)`
        // rather than propagating -- unlike Mode B's `query`/`get`, where a
        // compile error is a loud `Err` (a broken policy isn't "zero rows").
        let sieve = match auth {
            // Honor a caller-pre-resolved sieve exactly like
            // `compile_sieve_for_op` does, before ever calling the
            // local-only `compile_read` -- including that function's
            // operation-match assertion: a pre-resolved sieve compiled for a
            // different ability than requested here must not be trusted
            // verbatim.
            Some(a) if a.resolved_sieve.is_some() => {
                match &a.resolved_sieve {
                    Some(s) if s.trace.operation != operation => return Ok(false),
                    _ => {}
                }
                a.resolved_sieve.clone()
            }
            Some(a) => match compile_read(
                a.policy,
                collection,
                a.session,
                a.service_id,
                &Ability(operation.to_string()),
                Mode::PointInTime { id: id.to_string() },
            ) {
                Ok(s) => s,
                Err(_) => return Ok(false),
            },
            None => None,
        };
        let collection = collection.to_string();
        let id = id.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_check_access(conn, &collection, &id, sieve.as_ref()))
            .await
            .map_err(|e| {
                host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
            })?
    }

    async fn list_collections(&self) -> Result<Vec<String>, host_store::DataLayerError> {
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(do_list_collections).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
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
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().put(collection, value, creator_id, auth).await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().patch(collection, id, patch_json, auth).await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<ReadOutcome<Option<host_store::RecordReadValue>>, host_store::DataLayerError> {
        self.as_ref().get(collection, id, auth).await
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<ReadOutcome<host_store::QueryResult>, host_store::DataLayerError> {
        self.as_ref().query(collection, opts, auth).await
    }

    async fn aggregate(
        &self,
        collection: &str,
        pipeline: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        self.as_ref().aggregate(collection, pipeline, auth).await
    }

    async fn delete(
        &self,
        collection: &str,
        id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().delete(collection, id, auth).await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<u64, host_store::DataLayerError> {
        self.as_ref().delete_many(collection, filter, auth).await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().batch_mutate(collection, mutations, creator_id, auth).await
    }

    async fn query_raw(
        &self,
        sql: &str,
        params: &[host_store::SqlValue],
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        self.as_ref().query_raw(sql, params).await
    }

    async fn check_access(
        &self,
        collection: &str,
        id: &str,
        operation: &str,
        auth: Option<&QueryAuth<'_>>,
    ) -> Result<bool, host_store::DataLayerError> {
        self.as_ref().check_access(collection, id, operation, auth).await
    }

    async fn list_collections(&self) -> Result<Vec<String>, host_store::DataLayerError> {
        self.as_ref().list_collections().await
    }
}

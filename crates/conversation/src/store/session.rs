//! `sessions` and `local_identity` table operations: reading and upserting
//! per-peer X3DH sessions (`SessionRow`), storing the node's own identity
//! key material, and initialising prekey bundles. Entirely independent of
//! the DAG and message tables; only the transport layer reads these rows.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use zeroize::Zeroizing;

use super::{ConversationStore, LocalIdentityRow, SessionRow, now_ms};

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn session(&self, peer_address: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT peer_address, pinned_sig_key, state FROM sessions WHERE peer_address = ?1",
            params![peer_address],
            |r| {
                let sig_key: Vec<u8> = r.get(1)?;
                // A wrong-length blob means the row is corrupt. Fail
                // loudly: a silently zero-padded key would compare as a
                // partially-zero array, which is incorrect key material.
                let pinned: [u8; 32] = sig_key.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("pinned_sig_key must be exactly 32 bytes")),
                    )
                })?;
                Ok(SessionRow { peer_address: r.get(0)?, pinned_sig_key: pinned, state: r.get(2)? })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_session(&self, row: &SessionRow, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        Self::upsert_session_conn(&conn, row, now_ms)
    }

    pub fn upsert_session_in(
        &self,
        tx: &Transaction<'_>,
        row: &SessionRow,
        now_ms: i64,
    ) -> Result<()> {
        Self::upsert_session_conn(tx, row, now_ms)
    }

    /// `pub(crate)`: `crypto.rs`'s `SessionCrypto::commit_in` calls this
    /// directly, since it has a `&Transaction` but no `&ConversationStore`.
    pub(crate) fn upsert_session_conn(
        conn: &Connection,
        row: &SessionRow,
        now_ms: i64,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO sessions (peer_address, pinned_sig_key, state, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(peer_address) DO UPDATE SET state = ?3, updated_at = ?4",
            params![row.peer_address, row.pinned_sig_key.as_slice(), row.state, now_ms],
        )?;
        Ok(())
    }

    // -- local identity -------------------------------------------------

    /// Loads this service's own conversation identity, generating one on
    /// first use.
    pub fn local_identity_or_generate(
        &self,
        generate: impl FnOnce() -> (Vec<u8>, Vec<u8>),
    ) -> Result<LocalIdentityRow> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let existing: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT account_state, sig_secret FROM local_identity WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (account_state, sig_secret) = match existing {
            Some(pair) => pair,
            None => {
                let (dh, sig) = generate();
                conn.execute(
                    "INSERT INTO local_identity (id, account_state, sig_secret, created_at) \
                     VALUES (1, ?1, ?2, ?3)",
                    params![dh, sig, now_ms()],
                )?;
                (dh, sig)
            }
        };
        Ok(LocalIdentityRow {
            account_state: Zeroizing::new(account_state),
            sig_secret: Zeroizing::new(sig_secret),
        })
    }

    /// Persists a mutated ratchet account (`crypto.rs`'s `save_account`) --
    /// the row a one-time-key consumption or a fresh batch must land in
    /// before anything else can rely on it surviving a restart.
    pub fn save_local_account(&self, account_state: &[u8]) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE local_identity SET account_state = ?1 WHERE id = 1",
            params![account_state],
        )?;
        Ok(())
    }

    // -- prekey rate limiting -------------------------------------------------

    /// Increments this hour's request count for `caller_did` and returns
    /// whether it is still within budget. `window_start` is the request's
    /// own hour bucket. Rows older than 24 hours are pruned on every call
    /// to keep the table bounded for long-running services.
    pub fn record_prekey_request(&self, caller_did: &str, now_ms: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let window_start = now_ms - (now_ms % 3_600_000);
        let cutoff = now_ms - 86_400_000; // 24 hours
        conn.execute("DELETE FROM prekey_requests WHERE window_start < ?1", params![cutoff])?;
        conn.execute(
            "INSERT INTO prekey_requests (caller_did, window_start, count) VALUES (?1, ?2, 1)
             ON CONFLICT(caller_did, window_start) DO UPDATE SET count = count + 1",
            params![caller_did, window_start],
        )?;
        let count: i64 = conn.query_row(
            "SELECT count FROM prekey_requests WHERE caller_did = ?1 AND window_start = ?2",
            params![caller_did, window_start],
            |r| r.get(0),
        )?;
        Ok(count as u32 <= self.config.prekey_requests_per_peer_per_hour)
    }
}

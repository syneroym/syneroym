//! The last signed service record seen for each peer that has a mailbox,
//! kept on disk until the record's `not_after`.
//!
//! A sender reads it when the registry cannot be reached or no longer holds
//! the record (a receiver that sleeps for hours has none there), so it can
//! still find the receiver's mailbox. Only peers that were seen with a
//! mailbox get a row: a record without one is of no use for a deposit.
//!
//! A newer record without a mailbox does not delete the row. It replaces the
//! record in it and marks the row as having no mailbox (a tombstone). Then an
//! older record that still names the mailbox cannot bring it back, for
//! example one that arrives late from the DHT fallback of a lookup.
//!
//! The newer-wins check and the write are one SQL statement under the
//! connection lock, because calls to one peer are concurrent. The file is
//! plaintext, like the endpoint registry: the records are public, but the list
//! of peers this node called is not.

use std::{
    fmt, fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use anyhow::Context;
use dashmap::DashMap;
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_core::dht_registry::{EndpointType, SignedEndpointInfo};
use tokio::task;

/// What a stored row holds, kept in memory so the common case (the same
/// record seen again) costs no disk access. It is only changed while the
/// connection lock is held, so it always agrees with the file.
struct Known {
    packet_hex: String,
    has_mailbox: bool,
}

pub struct PeerRecordStore {
    conn: Arc<Mutex<Connection>>,
    known: Arc<DashMap<String, Known>>,
}

impl fmt::Debug for PeerRecordStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerRecordStore").field("peers", &self.known.len()).finish()
    }
}

fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(PoisonError::into_inner)
}

fn to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A record to store, checked and ready.
struct Entry {
    service_id: String,
    json: String,
    packet_hex: String,
    signed_at: u64,
    not_after: u64,
    has_mailbox: bool,
}

impl PeerRecordStore {
    /// Opens (and creates, if needed) the store at `path`. Rows past their
    /// `not_after` at `now_secs` are deleted.
    pub fn open(path: &Path, now_secs: u64) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        Self::from_connection(Connection::open(path)?, now_secs)
    }

    /// A store that lives only as long as the value, for tests.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, 0)
    }

    fn from_connection(conn: Connection, now_secs: u64) -> anyhow::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS peer_records (
                service_id  TEXT PRIMARY KEY,
                record_json TEXT NOT NULL,
                signed_at   INTEGER NOT NULL,
                not_after   INTEGER NOT NULL,
                has_mailbox INTEGER NOT NULL
            );",
        )?;
        conn.execute("DELETE FROM peer_records WHERE not_after < ?1", params![to_sql(now_secs)])?;

        let known = DashMap::new();
        let mut unreadable = Vec::new();
        {
            let mut stmt =
                conn.prepare("SELECT service_id, record_json, has_mailbox FROM peer_records")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
            })?;
            for row in rows {
                let (service_id, json, has_mailbox) = row?;
                match serde_json::from_str::<SignedEndpointInfo>(&json) {
                    Ok(record) => {
                        known.insert(
                            service_id,
                            Known { packet_hex: record.pkarr_packet_hex, has_mailbox },
                        );
                    }
                    Err(_) => unreadable.push(service_id),
                }
            }
        }
        for service_id in unreadable {
            conn.execute("DELETE FROM peer_records WHERE service_id = ?1", params![service_id])?;
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)), known: Arc::new(known) })
    }

    /// Keeps `record` if it is a service record that is newer than the one
    /// stored. A peer that was never seen with a mailbox is not stored unless
    /// the record names one. A record that does not verify is an error and
    /// changes nothing.
    pub async fn remember(&self, record: &SignedEndpointInfo) -> anyhow::Result<()> {
        if record.info.endpoint_type != EndpointType::Service {
            return Ok(());
        }
        let seen = self.known.get(&record.info.service_id);
        if let Some(seen) = &seen
            && seen.packet_hex == record.pkarr_packet_hex
        {
            return Ok(());
        }
        let has_mailbox = record.info.mailbox.is_some();
        if !has_mailbox && seen.is_none() {
            return Ok(());
        }
        drop(seen);

        let entry = Entry {
            service_id: record.info.service_id.clone(),
            json: serde_json::to_string(record)?,
            packet_hex: record.pkarr_packet_hex.clone(),
            signed_at: record.verify()?.as_u64(),
            not_after: record.info.not_after,
            has_mailbox,
        };
        let (conn, known) = (self.conn.clone(), self.known.clone());
        task::spawn_blocking(move || store_if_newer(&conn, &known, entry)).await?
    }

    /// The stored record of `service_id`, if it names a mailbox and is still
    /// valid. A stored record that no longer verifies (it passed its
    /// `not_after`) is deleted.
    pub async fn recall(&self, service_id: &str) -> Option<SignedEndpointInfo> {
        if !self.known.get(service_id)?.has_mailbox {
            return None;
        }
        let (conn, id) = (self.conn.clone(), service_id.to_string());
        let json = task::spawn_blocking(move || read_row(&conn, &id)).await.ok()?.ok()??;
        let record = serde_json::from_str::<SignedEndpointInfo>(&json)
            .ok()
            .filter(|record| record.info.service_id == service_id && record.verify().is_ok());
        if record.is_none() {
            self.forget(service_id).await;
        }
        record
    }

    async fn forget(&self, service_id: &str) {
        let (conn, known, id) = (self.conn.clone(), self.known.clone(), service_id.to_string());
        let _ = task::spawn_blocking(move || {
            let guard = lock(&conn);
            let deleted = guard
                .execute("DELETE FROM peer_records WHERE service_id = ?1", params![id])
                .is_ok();
            if deleted {
                known.remove(&id);
            }
        })
        .await;
    }
}

/// The compare and the write in one statement, with `known` updated before
/// the lock is released. A record with a mailbox is inserted or replaces an
/// older row. A record without one only replaces an older row, so a peer
/// never seen with a mailbox gets none.
fn store_if_newer(
    conn: &Mutex<Connection>,
    known: &DashMap<String, Known>,
    entry: Entry,
) -> anyhow::Result<()> {
    let guard = lock(conn);
    let values = params![
        entry.service_id,
        entry.json,
        to_sql(entry.signed_at),
        to_sql(entry.not_after),
        entry.has_mailbox
    ];
    let changed = if entry.has_mailbox {
        guard.execute(
            "INSERT INTO peer_records (service_id, record_json, signed_at, not_after, has_mailbox)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(service_id) DO UPDATE SET
                 record_json = excluded.record_json,
                 signed_at   = excluded.signed_at,
                 not_after   = excluded.not_after,
                 has_mailbox = excluded.has_mailbox
             WHERE excluded.signed_at > peer_records.signed_at",
            values,
        )?
    } else {
        guard.execute(
            "UPDATE peer_records SET record_json = ?2, signed_at = ?3, not_after = ?4,
                 has_mailbox = ?5
             WHERE service_id = ?1 AND signed_at < ?3",
            values,
        )?
    };
    if changed > 0 {
        known.insert(
            entry.service_id,
            Known { packet_hex: entry.packet_hex, has_mailbox: entry.has_mailbox },
        );
    }
    Ok(())
}

fn read_row(conn: &Mutex<Connection>, service_id: &str) -> anyhow::Result<Option<String>> {
    Ok(lock(conn)
        .query_row(
            "SELECT record_json FROM peer_records WHERE service_id = ?1 AND has_mailbox = 1",
            params![service_id],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests;

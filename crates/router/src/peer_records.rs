//! The last signed service record seen for each peer that has a mailbox,
//! kept on disk until the record's `not_after`.
//!
//! A sender reads it when the registry cannot be reached or no longer holds
//! the record (a receiver that sleeps for hours has none there), so it can
//! still find the receiver's mailbox. Only records with a mailbox are kept:
//! a record without one is of no use for a deposit. The file is plaintext,
//! like the endpoint registry: the records are public, but the list of peers
//! this node called is not.

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
/// record seen again) costs no disk access.
struct Known {
    /// The pkarr timestamp of the stored record, in microseconds.
    signed_at: u64,
    packet_hex: String,
}

pub struct PeerRecordStore {
    conn: Arc<Mutex<Connection>>,
    known: DashMap<String, Known>,
}

impl fmt::Debug for PeerRecordStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerRecordStore").field("peers", &self.known.len()).finish()
    }
}

fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(PoisonError::into_inner)
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
                not_after   INTEGER NOT NULL
            );",
        )?;
        let now = i64::try_from(now_secs).unwrap_or(i64::MAX);
        conn.execute("DELETE FROM peer_records WHERE not_after < ?1", params![now])?;

        let known = DashMap::new();
        let mut unreadable = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT service_id, record_json FROM peer_records")?;
            let rows =
                stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            for row in rows {
                let (service_id, json) = row?;
                match serde_json::from_str::<SignedEndpointInfo>(&json) {
                    Ok(record) => {
                        let signed_at = record.verify().map(|t| t.as_u64()).unwrap_or(0);
                        known.insert(
                            service_id,
                            Known { signed_at, packet_hex: record.pkarr_packet_hex },
                        );
                    }
                    Err(_) => unreadable.push(service_id),
                }
            }
        }
        for service_id in unreadable {
            conn.execute("DELETE FROM peer_records WHERE service_id = ?1", params![service_id])?;
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)), known })
    }

    /// Keeps `record` if it is a service record with a mailbox that is newer
    /// than the one stored. A newer record without a mailbox deletes the
    /// stored one: the deployer removed the mailbox. A record that does not
    /// verify is an error and changes nothing.
    pub async fn remember(&self, record: &SignedEndpointInfo) -> anyhow::Result<()> {
        if record.info.endpoint_type != EndpointType::Service {
            return Ok(());
        }
        let service_id = record.info.service_id.clone();
        let stored = self.known.get(&service_id).map(|k| (k.signed_at, k.packet_hex.clone()));
        if let Some((_, packet_hex)) = &stored
            && *packet_hex == record.pkarr_packet_hex
        {
            return Ok(());
        }
        if record.info.mailbox.is_none() && stored.is_none() {
            return Ok(());
        }
        let signed_at = record.verify()?.as_u64();
        if let Some((stored_at, _)) = stored
            && stored_at >= signed_at
        {
            return Ok(());
        }

        let conn = self.conn.clone();
        if record.info.mailbox.is_none() {
            let id = service_id.clone();
            task::spawn_blocking(move || delete_row(&conn, &id)).await??;
            self.known.remove(&service_id);
            return Ok(());
        }
        let json = serde_json::to_string(record)?;
        let (id, not_after) = (service_id.clone(), record.info.not_after);
        task::spawn_blocking(move || insert_row(&conn, &id, &json, signed_at, not_after)).await??;
        self.known
            .insert(service_id, Known { signed_at, packet_hex: record.pkarr_packet_hex.clone() });
        Ok(())
    }

    /// The stored record of `service_id`, if it is still valid. A stored
    /// record that no longer verifies (it passed its `not_after`) is deleted.
    pub async fn recall(&self, service_id: &str) -> Option<SignedEndpointInfo> {
        self.known.get(service_id)?;
        let conn = self.conn.clone();
        let id = service_id.to_string();
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
        let conn = self.conn.clone();
        let id = service_id.to_string();
        let _ = task::spawn_blocking(move || delete_row(&conn, &id)).await;
        self.known.remove(service_id);
    }
}

fn delete_row(conn: &Mutex<Connection>, service_id: &str) -> anyhow::Result<()> {
    lock(conn).execute("DELETE FROM peer_records WHERE service_id = ?1", params![service_id])?;
    Ok(())
}

fn insert_row(
    conn: &Mutex<Connection>,
    service_id: &str,
    json: &str,
    signed_at: u64,
    not_after: u64,
) -> anyhow::Result<()> {
    lock(conn).execute(
        "INSERT OR REPLACE INTO peer_records (service_id, record_json, signed_at, not_after)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            service_id,
            json,
            i64::try_from(signed_at).unwrap_or(i64::MAX),
            i64::try_from(not_after).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(())
}

fn read_row(conn: &Mutex<Connection>, service_id: &str) -> anyhow::Result<Option<String>> {
    Ok(lock(conn)
        .query_row(
            "SELECT record_json FROM peer_records WHERE service_id = ?1",
            params![service_id],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests;

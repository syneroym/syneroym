//! The lease table and deny list of one service, kept in SQLite across
//! restarts, with an in-memory index for admission checks.

use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock},
};

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, Row, params};
use serde_json::{Map, Value};

use crate::{error::RegisterError, issue::PassportId, passport_use::PassportUse};

mod cache;

use cache::Cache;

/// Access for one subject under one passport, until `lease_ends_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub passport_id: PassportId,
    pub owner_did: String,
    pub subject: String,
    pub lease_ends_at: u64,
    /// When this lease was last written (first registration or renewal).
    pub registered_at: u64,
    /// The signed facts of the passport (never of the child token). The
    /// mailbox reads its raised quotas from these.
    pub limits: Map<String, Value>,
}

/// The lease table and deny list of one service for one use. The store is
/// bound to the ability of that use when it is opened, and a file made for
/// another use is refused: a lease of one use must never admit a subject
/// under another. SQLite holds the data across restarts; an in-memory index
/// answers admission checks without I/O. One process owns the file: the index
/// is loaded only at `open`, so a write by another process is not seen until a
/// restart. Deny and list therefore go through the running service.
#[derive(Debug, Clone)]
pub struct LeaseStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// The ability of the use this store belongs to.
    ability: &'static str,
    /// Also serialises writers, so the count and the insert of `grant` are
    /// one atomic step.
    conn: Mutex<Connection>,
    cache: RwLock<Cache>,
}

fn store_err(e: impl fmt::Display) -> RegisterError {
    RegisterError::Store(e.to_string())
}

fn to_sql_time(secs: u64) -> Result<i64, RegisterError> {
    i64::try_from(secs).map_err(store_err)
}

impl LeaseStore {
    /// Open (or create) `<dir>/<db_name>` for `use_` and load the running
    /// leases and the deny list. Rows whose lease has ended are deleted. An
    /// existing file that was made for another use is an error.
    pub fn open(
        dir: impl AsRef<Path>,
        db_name: &str,
        use_: &PassportUse,
        now_secs: u64,
    ) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {db_name}"));
        }
        let conn = Connection::open(dir.as_ref().join(db_name))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::from_connection(conn, use_, now_secs)
    }

    /// A store that lives only in memory, for tests and tools.
    pub fn open_in_memory(use_: &PassportUse) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, use_, 0)
    }

    fn from_connection(conn: Connection, use_: &PassportUse, now_secs: u64) -> Result<Self> {
        init_schema(&conn)?;
        bind_to_use(&conn, use_.ability)?;
        conn.execute(
            "DELETE FROM passport_leases WHERE lease_ends_at <= ?1",
            [to_sql_time(now_secs)?],
        )?;
        let cache = load_cache(&conn)?;
        Ok(Self {
            inner: Arc::new(Inner {
                ability: use_.ability,
                conn: Mutex::new(conn),
                cache: RwLock::new(cache),
            }),
        })
    }

    /// The ability of the use this store was opened for.
    #[must_use]
    pub fn ability(&self) -> &'static str {
        self.inner.ability
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.inner.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read<T>(&self, f: impl FnOnce(&Cache) -> T) -> T {
        f(&self.inner.cache.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn write<T>(&self, f: impl FnOnce(&mut Cache) -> T) -> T {
        f(&mut self.inner.cache.write().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether `subject` has a running lease. Reads memory only.
    #[must_use]
    pub fn is_admitted(&self, subject: &str, now_secs: u64) -> bool {
        self.read(|c| c.is_admitted(subject, now_secs))
    }

    /// The running lease that applies to `subject`, with its limits. With
    /// leases under several passports, the newest registration wins. Reads
    /// memory only.
    #[must_use]
    pub fn lease_for(&self, subject: &str, now_secs: u64) -> Option<Lease> {
        self.read(|c| c.lease_for(subject, now_secs))
    }

    /// Every running lease, sorted by passport id, then subject.
    #[must_use]
    pub fn running_leases(&self, now_secs: u64) -> Vec<Lease> {
        self.read(|c| c.running(now_secs))
    }

    #[must_use]
    pub fn is_denied(&self, id: &PassportId) -> bool {
        self.read(|c| c.is_denied(id))
    }

    /// Deny `id`: refuse it from now on and end its leases. Returns how many
    /// leases ended. A deny of an id with no leases is still stored.
    pub fn deny(&self, id: &PassportId, now_secs: u64) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO passport_denied (passport_id, denied_at) VALUES (?1, ?2)",
            params![id.as_str(), to_sql_time(now_secs)?],
        )?;
        let removed =
            tx.execute("DELETE FROM passport_leases WHERE passport_id = ?1", [id.as_str()])?;
        tx.commit()?;
        self.write(|c| {
            c.deny(id.clone());
            c.remove_passport(id);
        });
        Ok(removed)
    }

    /// Write `lease` if the passport is not denied and `limit` allows it.
    pub(crate) fn grant(
        &self,
        lease: Lease,
        limit: Option<u64>,
        now_secs: u64,
    ) -> Result<(), RegisterError> {
        let mut conn = self.conn();
        self.read(|c| {
            if c.is_denied(&lease.passport_id) {
                return Err(RegisterError::Denied);
            }
            match limit {
                Some(limit)
                    if c.other_running_subjects(&lease.passport_id, &lease.subject, now_secs)
                        >= limit =>
                {
                    Err(RegisterError::LimitReached { limit })
                }
                _ => Ok(()),
            }
        })?;
        write_lease(&mut conn, &lease, now_secs)?;
        self.write(|c| {
            c.prune(now_secs);
            c.insert(lease);
        });
        Ok(())
    }
}

fn write_lease(conn: &mut Connection, lease: &Lease, now_secs: u64) -> Result<(), RegisterError> {
    let limits = serde_json::to_string(&lease.limits).map_err(store_err)?;
    let tx = conn.transaction().map_err(store_err)?;
    tx.execute("DELETE FROM passport_leases WHERE lease_ends_at <= ?1", [to_sql_time(now_secs)?])
        .map_err(store_err)?;
    tx.execute(
        "INSERT INTO passport_leases
             (passport_id, owner_did, subject, lease_ends_at, registered_at, limits)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (passport_id, subject) DO UPDATE
         SET owner_did = excluded.owner_did, lease_ends_at = excluded.lease_ends_at,
             registered_at = excluded.registered_at, limits = excluded.limits",
        params![
            lease.passport_id.as_str(),
            lease.owner_did,
            lease.subject,
            to_sql_time(lease.lease_ends_at)?,
            to_sql_time(lease.registered_at)?,
            limits,
        ],
    )
    .map_err(store_err)?;
    tx.commit().map_err(store_err)
}

fn init_schema(conn: &Connection) -> Result<()> {
    // Unconditional, not gated on a schema version: pre-release, schema
    // changes are made in place, and `IF NOT EXISTS` is already idempotent.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS passport_leases (
             passport_id   TEXT NOT NULL,
             owner_did     TEXT NOT NULL,
             subject       TEXT NOT NULL,
             lease_ends_at INTEGER NOT NULL,
             registered_at INTEGER NOT NULL,
             limits        TEXT NOT NULL,
             PRIMARY KEY (passport_id, subject)
         );
         CREATE INDEX IF NOT EXISTS idx_passport_leases_end ON passport_leases(lease_ends_at);
         CREATE TABLE IF NOT EXISTS passport_store (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             ability   TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS passport_denied (
             passport_id TEXT PRIMARY KEY,
             denied_at   INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

/// A file keeps the ability of the use that first opened it.
fn bind_to_use(conn: &Connection, ability: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO passport_store (singleton, ability) VALUES (1, ?1)",
        [ability],
    )?;
    let stored: String =
        conn.query_row("SELECT ability FROM passport_store WHERE singleton = 1", [], |r| r.get(0))?;
    ensure!(stored == ability, "this lease file belongs to {stored}, not to {ability}");
    Ok(())
}

fn load_cache(conn: &Connection) -> Result<Cache> {
    let mut cache = Cache::default();
    let mut stmt = conn.prepare(
        "SELECT passport_id, owner_did, subject, lease_ends_at, registered_at, limits
         FROM passport_leases",
    )?;
    for row in stmt.query_map([], StoredLease::read)? {
        cache.insert(row?.into_lease()?);
    }
    let mut stmt = conn.prepare("SELECT passport_id FROM passport_denied")?;
    for id in stmt.query_map([], |row| row.get::<_, String>(0))? {
        cache.deny(id?.parse().context("a stored denied passport id")?);
    }
    Ok(cache)
}

/// One lease row as SQLite returns it, before its values are checked.
struct StoredLease {
    passport_id: String,
    owner_did: String,
    subject: String,
    lease_ends_at: i64,
    registered_at: i64,
    limits: String,
}

impl StoredLease {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            passport_id: row.get(0)?,
            owner_did: row.get(1)?,
            subject: row.get(2)?,
            lease_ends_at: row.get(3)?,
            registered_at: row.get(4)?,
            limits: row.get(5)?,
        })
    }

    fn into_lease(self) -> Result<Lease> {
        let limits = match serde_json::from_str(&self.limits)? {
            Value::Object(map) => map,
            _ => return Err(anyhow!("stored lease limits are not a JSON object")),
        };
        Ok(Lease {
            passport_id: self.passport_id.parse().context("a stored passport id")?,
            owner_did: self.owner_did,
            subject: self.subject,
            lease_ends_at: u64::try_from(self.lease_ends_at)?,
            registered_at: u64::try_from(self.registered_at)?,
            limits,
        })
    }
}

#[cfg(test)]
mod tests;

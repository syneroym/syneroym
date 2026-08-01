//! Desired state for every app instance this supervisor manages, over the
//! same `Arc<Mutex<Connection>>` that backs `DeploymentJournal` and
//! `AlertStore` (D-A5-11): one SQLite file, three concerns.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_app_orchestration::{AlertStore, DeploymentJournal};

/// One app instance's desired state, as last submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredState {
    pub app_instance_id: String,
    /// Compiled `DeploymentPlan`, artifacts inlined, masters substituted.
    pub plan_json: String,
    /// Alias -> `{did, api-url, ucan}`.
    pub inventory_json: String,
    pub owner_did: String,
    pub generation: u64,
    pub paused: bool,
    pub retired: bool,
    pub submitted_at: i64,
    pub updated_at: i64,
}

/// One SQLite file holding desired state, the deployment journal, and
/// alerts (D-A5-11) -- what A4 promised A5 "the schema, the types, and the
/// folding logic, not the file", cashed in.
#[derive(Debug, Clone)]
pub struct SupervisorStore {
    conn: Arc<Mutex<Connection>>,
    pub journal: DeploymentJournal,
    pub alerts: AlertStore,
}

impl SupervisorStore {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {}", db_name));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        Self::init_schema(&conn)?;
        let conn = Arc::new(Mutex::new(conn));
        let journal = DeploymentJournal::from_connection(conn.clone())?;
        let alerts = AlertStore::from_connection(conn.clone())?;
        Ok(Self { conn, journal, alerts })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Unconditional, not gated on `PRAGMA user_version`: pre-release,
        // schema changes are made in place with no version ladder, and
        // `IF NOT EXISTS` is already idempotent.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS desired_state (
                app_instance_id TEXT PRIMARY KEY,
                plan_json       TEXT NOT NULL,
                inventory_json  TEXT NOT NULL,
                owner_did       TEXT NOT NULL,
                generation      INTEGER NOT NULL DEFAULT 0,
                paused          INTEGER NOT NULL DEFAULT 0,
                retired         INTEGER NOT NULL DEFAULT 0,
                submitted_at    INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Replaces desired state for `app_instance_id`, keeping exactly one
    /// row per instance -- a re-submit is a full replacement, not an
    /// additional version. Refused once the instance is retired: retiring
    /// is meant to hand an instance back to manual operation, and a submit
    /// that landed anyway would silently resume supervision behind the
    /// operator's back. `supervisor retire` is the only way back in.
    pub fn submit(
        &self,
        app_instance_id: &str,
        plan_json: &str,
        inventory_json: &str,
        owner_did: &str,
        generation: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let retired: Option<i64> = conn
            .query_row(
                "SELECT retired FROM desired_state WHERE app_instance_id = ?1",
                params![app_instance_id],
                |row| row.get(0),
            )
            .optional()?;
        if retired == Some(1) {
            return Err(anyhow!(
                "app instance '{app_instance_id}' is retired; run `supervisor release` or \
                 re-adopt before submitting new desired state"
            ));
        }
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO desired_state
                (app_instance_id, plan_json, inventory_json, owner_did, generation, paused,
                 retired, submitted_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6, ?6)
             ON CONFLICT(app_instance_id) DO UPDATE SET
                plan_json = excluded.plan_json,
                inventory_json = excluded.inventory_json,
                owner_did = excluded.owner_did,
                generation = excluded.generation,
                updated_at = excluded.updated_at",
            params![app_instance_id, plan_json, inventory_json, owner_did, generation as i64, now],
        )?;
        Ok(())
    }

    pub fn get(&self, app_instance_id: &str) -> Result<Option<DesiredState>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT app_instance_id, plan_json, inventory_json, owner_did, generation, paused, \
             retired, submitted_at, updated_at
             FROM desired_state WHERE app_instance_id = ?1",
            params![app_instance_id],
            |row| {
                Ok(DesiredState {
                    app_instance_id: row.get(0)?,
                    plan_json: row.get(1)?,
                    inventory_json: row.get(2)?,
                    owner_did: row.get(3)?,
                    generation: row.get::<_, i64>(4)? as u64,
                    paused: row.get::<_, i64>(5)? != 0,
                    retired: row.get::<_, i64>(6)? != 0,
                    submitted_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Every non-retired, non-paused instance -- the resident loop's own
    /// work list, and `status`'s universe for "does this instance exist".
    pub fn all_active(&self) -> Result<Vec<DesiredState>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT app_instance_id, plan_json, inventory_json, owner_did, generation, paused, \
             retired, submitted_at, updated_at
             FROM desired_state WHERE retired = 0 ORDER BY app_instance_id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(DesiredState {
                app_instance_id: row.get(0)?,
                plan_json: row.get(1)?,
                inventory_json: row.get(2)?,
                owner_did: row.get(3)?,
                generation: row.get::<_, i64>(4)? as u64,
                paused: row.get::<_, i64>(5)? != 0,
                retired: row.get::<_, i64>(6)? != 0,
                submitted_at: row.get(7)?,
                updated_at: row.get(8)?,
            });
        }
        Ok(out)
    }

    fn set_flag(&self, app_instance_id: &str, column: &str, value: bool) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let now = chrono::Utc::now().timestamp();
        let sql = format!(
            "UPDATE desired_state SET {column} = ?1, updated_at = ?2 WHERE app_instance_id = ?3"
        );
        let updated = conn.execute(&sql, params![i64::from(value), now, app_instance_id])?;
        if updated == 0 {
            return Err(anyhow!("no desired state submitted for app instance '{app_instance_id}'"));
        }
        Ok(())
    }

    pub fn pause(&self, app_instance_id: &str) -> Result<()> {
        self.set_flag(app_instance_id, "paused", true)
    }

    pub fn resume(&self, app_instance_id: &str) -> Result<()> {
        self.set_flag(app_instance_id, "paused", false)
    }

    /// Terminal: a later `submit` is refused until the instance is
    /// re-adopted (D-A5-20's substrate-side release is the caller's
    /// counterpart -- clearing the substrate's own stamp, not this row).
    pub fn retire(&self, app_instance_id: &str) -> Result<()> {
        self.set_flag(app_instance_id, "retired", true)
    }

    /// Updates the held generation in place, without touching the rest of
    /// desired state -- `adopt`'s own write, distinct from a re-`submit`.
    pub fn set_generation(&self, app_instance_id: &str, generation: u64) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let now = chrono::Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE desired_state SET generation = ?1, updated_at = ?2 WHERE app_instance_id = ?3",
            params![generation as i64, now, app_instance_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("no desired state submitted for app instance '{app_instance_id}'"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submitting_twice_replaces_desired_state_and_keeps_one_row() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{\"v\":1}", "{}", "did:key:owner", 0).unwrap();
        store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();

        let state = store.get("inst-1").unwrap().unwrap();
        assert_eq!(state.plan_json, "{\"v\":2}");

        let conn = store.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM desired_state WHERE app_instance_id = 'inst-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        assert!(!store.get("inst-1").unwrap().unwrap().paused);

        store.pause("inst-1").unwrap();
        assert!(store.get("inst-1").unwrap().unwrap().paused);

        store.resume("inst-1").unwrap();
        assert!(!store.get("inst-1").unwrap().unwrap().paused);
    }

    #[test]
    fn retire_is_terminal_and_a_later_submit_is_refused() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        store.retire("inst-1").unwrap();
        assert!(store.get("inst-1").unwrap().unwrap().retired);

        let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 1).unwrap_err();
        assert!(err.to_string().contains("retired"), "{err}");
    }
}

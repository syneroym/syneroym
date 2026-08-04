//! Desired state for every app instance this supervisor manages, over the
//! same `Arc<Mutex<Connection>>` that backs `DeploymentJournal`,
//! `AlertStore`, and (M05B B1, D-B1-5) the durable outbox queue: one SQLite
//! file, four concerns.

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_app_orchestration::{AlertStore, DeploymentJournal};
use syneroym_async_queue::{Queue, QueueConfig};
use syneroym_core::config::SupervisorRole;

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
    /// The app instance's own master DID (M05A A7, D-A7-2/D-A7-4), empty
    /// until the instance's next `adopt` mints or resolves one -- the vault
    /// cannot be enumerated and this instance appears in no plan, so this
    /// column is the only index, not a cache of something else readable.
    pub app_master_did: String,
}

/// One service's bounded-restart bookkeeping (§14 step 6, D-A5c-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemediationState {
    pub attempts: u32,
    pub last_attempt_at: Option<i64>,
    pub terminal: bool,
}

/// One SQLite file holding desired state, the deployment journal, alerts,
/// and (M05B B1, D-B1-5) the durable outbox queue -- what A4 promised A5
/// "the schema, the types, and the folding logic, not the file", cashed in,
/// now a fourth time.
#[derive(Debug, Clone)]
pub struct SupervisorStore {
    conn: Arc<Mutex<Connection>>,
    pub journal: DeploymentJournal,
    pub alerts: AlertStore,
    pub queue: Queue,
}

// Lock-poisoning from a panicking holder is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here, matching `StaticInventory`'s
// (`crates/app_orchestration/src/resolver.rs`).
#[allow(clippy::expect_used)]
impl SupervisorStore {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self> {
        Self::open_with_role(dir, db_name, &SupervisorRole::default())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, &SupervisorRole::default())
    }

    /// The construction path `runtime.rs` actually uses: `role`'s five
    /// `queue_*` fields size the outbox's attempt budget, backoff ceiling,
    /// visibility timeout, and DLQ cap (D-B1-13). Every other caller --
    /// tests overwhelmingly among them -- goes through `open`/
    /// `open_in_memory` and gets the same defaults `SupervisorRole::default`
    /// does.
    pub fn open_with_role<P: AsRef<Path>>(
        dir: P,
        db_name: &str,
        role: &SupervisorRole,
    ) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {}", db_name));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::from_connection(conn, role)
    }

    fn from_connection(conn: Connection, role: &SupervisorRole) -> Result<Self> {
        Self::init_schema(&conn)?;
        let conn = Arc::new(Mutex::new(conn));
        let journal = DeploymentJournal::from_connection(conn.clone())?;
        let alerts = AlertStore::from_connection(conn.clone())?;
        let queue = Queue::from_connection(conn.clone(), QueueConfig::from(role))?;
        Ok(Self { conn, journal, alerts, queue })
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
                updated_at      INTEGER NOT NULL,
                app_master_did  TEXT NOT NULL DEFAULT ''
             );
             -- M05A A5c D-A5c-4 (§19.3): one counter per *dependent
             -- service*, not per dependency -- every binding that service
             -- emits shares this one value. An absent row reads as epoch 0,
             -- meaning \"no supervisor has written here\" (the same meaning
             -- `roymctl app deploy`'s own epoch-0 writes already carry), so
             -- a hand-deployed instance this supervisor has never pushed to
             -- converges rather than reading as a false negative.
             CREATE TABLE IF NOT EXISTS binding_epochs (
                app_instance_id TEXT NOT NULL,
                logical_ref     TEXT NOT NULL,
                epoch           INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (app_instance_id, logical_ref)
             );
             -- §14 step 6 / D-A5c-20: attempt bookkeeping for bounded
             -- restart-in-place remediation, durable across a supervisor
             -- restart. `terminal` is cleared by `force-reconcile` and
             -- `adopt` (D-A5c-20), not only by a healthy sweep -- a
             -- terminal `InstanceNotRunning` service is never restarted
             -- again, so the sweep that would otherwise clear it never
             -- fires.
             CREATE TABLE IF NOT EXISTS remediation (
                app_instance_id TEXT NOT NULL,
                logical_ref     TEXT NOT NULL,
                attempts        INTEGER NOT NULL DEFAULT 0,
                last_attempt_at INTEGER,
                terminal        INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (app_instance_id, logical_ref)
             );
             -- M05A A5d: when each managed master's anchor was last
             -- republished. Keyed by master DID rather than by instance,
             -- because the anchor belongs to the master: two instances
             -- naming the same master must not each refresh it on their
             -- own schedule. Read every pass and compared against
             -- `master_anchor_refresh_interval_secs`, so the refresh needs
             -- no timer of its own.
             CREATE TABLE IF NOT EXISTS master_anchor_refresh (
                master_did        TEXT PRIMARY KEY,
                last_refreshed_at INTEGER NOT NULL
             );
             -- M05A A5d: placements whose instance key an operator has
             -- revoked. Read by *every* path that can mint a certificate
             -- -- the renewal work-list, `submit`, and `force-reconcile`
             -- alike -- because revoking a key and then silently
             -- re-minting one on the next ordinary redeploy is not a
             -- revocation at all.
             CREATE TABLE IF NOT EXISTS revoked_placements (
                app_instance_id TEXT NOT NULL,
                logical_ref     TEXT NOT NULL,
                revoked_at      INTEGER NOT NULL,
                PRIMARY KEY (app_instance_id, logical_ref)
             );
             -- A member whose renewal installed a fresh certificate but
             -- whose `restart-on-rotation` restart then failed. The
             -- certificate alone settles the health poll, so nothing else
             -- remembers the process still needs that restart -- this is
             -- the one thing that does, independent of the renewal
             -- alert's own lifecycle.
             CREATE TABLE IF NOT EXISTS pending_rotation_restarts (
                app_instance_id TEXT NOT NULL,
                logical_ref     TEXT NOT NULL,
                marked_at       INTEGER NOT NULL,
                PRIMARY KEY (app_instance_id, logical_ref)
             );",
        )?;
        // M05A A7 (D-A7-2): `desired_state` predates this column, so the
        // `CREATE TABLE IF NOT EXISTS` above is a no-op on any database that
        // already exists -- it never adds a column to a table already
        // there. This `ALTER TABLE` is the one idempotent way to get the
        // column onto a database that opened before it existed; not the
        // version ladder AGENTS.md rules out, since there is no schema
        // version to track. Same shape as `RegistryStore`'s own
        // `manifest_hash` column (`crates/data_db/src/registry_store.rs`).
        // "duplicate column name" is the expected outcome on every open
        // after the first.
        if let Err(err) = conn.execute(
            "ALTER TABLE desired_state ADD COLUMN app_master_did TEXT NOT NULL DEFAULT ''",
            [],
        ) && !err.to_string().contains("duplicate column name")
        {
            return Err(err.into());
        }
        Ok(())
    }

    /// This dependent service's current binding epoch, or `0` if it has
    /// never written one (D-A5c-4) -- the reading a hand-deployed or
    /// never-pushed-to service must have for the convergence join to be
    /// correct rather than a false negative.
    pub fn binding_epoch(&self, app_instance_id: &str, logical_ref: &str) -> Result<u64> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT epoch FROM binding_epochs WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map_or(Ok(0), |e| Ok(e as u64))
    }

    /// Advances this dependent service's binding epoch by one and returns
    /// the new value. The supervisor's own invariant (D-A5c-4): this is
    /// called before *every* write this service's bindings are part of --
    /// a push or a deploy alike -- so any write this supervisor issues
    /// always carries a strictly higher epoch than anything it has itself
    /// written before, which is what makes `install_app_context`'s
    /// unguarded `save_binding` correct rather than a regression.
    pub fn advance_binding_epoch(&self, app_instance_id: &str, logical_ref: &str) -> Result<u64> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO binding_epochs (app_instance_id, logical_ref, epoch)
             VALUES (?1, ?2, 1)
             ON CONFLICT(app_instance_id, logical_ref) DO UPDATE SET epoch = epoch + 1",
            params![app_instance_id, logical_ref],
        )?;
        conn.query_row(
            "SELECT epoch FROM binding_epochs WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
            |row| row.get::<_, i64>(0),
        )
        .map(|e| e as u64)
        .map_err(Into::into)
    }

    /// Sets this dependent service's binding epoch to `epoch` if that is
    /// higher than what is currently held, otherwise leaves it unchanged
    /// -- the retry half of D-A5c-19 (§19.19/F4): a `Stale(held)` push
    /// outcome retries at `held + 1`, which the caller computes and
    /// passes here directly (no re-read: `Stale` already carries the
    /// number), so the supervisor's own table agrees with the substrate
    /// afterward. Unlike `advance_binding_epoch`, this sets an absolute
    /// value, not a `+1` delta.
    pub fn set_binding_epoch_at_least(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        epoch: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO binding_epochs (app_instance_id, logical_ref, epoch)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(app_instance_id, logical_ref) DO UPDATE SET
                epoch = MAX(epoch, excluded.epoch)",
            params![app_instance_id, logical_ref, epoch as i64],
        )?;
        Ok(())
    }

    /// This service's remediation bookkeeping, if any restart has ever
    /// been attempted for it.
    pub fn remediation_state(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
    ) -> Result<Option<RemediationState>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT attempts, last_attempt_at, terminal FROM remediation
             WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
            |row| {
                Ok(RemediationState {
                    attempts: row.get::<_, i64>(0)? as u32,
                    last_attempt_at: row.get(1)?,
                    terminal: row.get::<_, i64>(2)? != 0,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Records one restart attempt, incrementing the counter and stamping
    /// `now`. Returns the new attempt count -- the loop's own remediation
    /// policy (§14 step 3, Phase 6) compares this against
    /// `max_restart_attempts` to decide whether this is the attempt that
    /// goes terminal.
    pub fn record_restart_attempt(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        now: i64,
    ) -> Result<u32> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO remediation (app_instance_id, logical_ref, attempts, last_attempt_at, \
             terminal)
             VALUES (?1, ?2, 1, ?3, 0)
             ON CONFLICT(app_instance_id, logical_ref) DO UPDATE SET
                attempts = attempts + 1, last_attempt_at = ?3",
            params![app_instance_id, logical_ref, now],
        )?;
        conn.query_row(
            "SELECT attempts FROM remediation WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
            |row| row.get::<_, i64>(0),
        )
        .map(|a| a as u32)
        .map_err(Into::into)
    }

    /// Marks this service's remediation terminal -- matrix row 13:
    /// exceeding `max_restart_attempts` stops the restart loop for this
    /// service until `force-reconcile` or `adopt` clears it (D-A5c-20).
    pub fn mark_remediation_terminal(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "UPDATE remediation SET terminal = 1
             WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
        )?;
        Ok(())
    }

    /// Clears one service's remediation row entirely -- a healthy sweep's
    /// own clearing path (§14 step 6): the service recovered on its own
    /// (an out-of-band restart, a container restart policy) or the bounded
    /// restart itself succeeded, so the next fault starts counting from
    /// zero rather than compounding a stale attempt count.
    pub fn clear_remediation(&self, app_instance_id: &str, logical_ref: &str) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "DELETE FROM remediation WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
        )?;
        Ok(())
    }

    /// Clears every remediation row for an instance -- `force-reconcile`
    /// and `adopt`'s own clearing path (D-A5c-20 / F5): both are a fresh
    /// start by construction, so a terminal `InstanceNotRunning` service
    /// that nothing will ever restart again becomes escapable through
    /// them rather than staying stuck forever.
    pub fn clear_remediation_for_instance(&self, app_instance_id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "DELETE FROM remediation WHERE app_instance_id = ?1",
            params![app_instance_id],
        )?;
        Ok(())
    }

    /// When this master's anchor was last republished, or `None` if this
    /// supervisor never has. An absent row reads as "overdue", which is the
    /// correct first-pass behavior: an anchor this supervisor has no record
    /// of publishing may not exist at the registry at all, and until it
    /// does every certificate the master issues is unusable on the wire.
    pub fn last_master_anchor_refresh(&self, master_did: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT last_refreshed_at FROM master_anchor_refresh WHERE master_did = ?1",
            params![master_did],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Stamps a successful anchor republication. Only called after
    /// `refresh_master_anchor` returns cleanly -- a failed publish must
    /// leave the previous stamp alone so the next pass retries.
    pub fn record_master_anchor_refresh(&self, master_did: &str, at: i64) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO master_anchor_refresh (master_did, last_refreshed_at)
             VALUES (?1, ?2)
             ON CONFLICT(master_did) DO UPDATE SET last_refreshed_at = excluded.last_refreshed_at",
            params![master_did, at],
        )?;
        Ok(())
    }

    /// Records that this placement's instance key has been revoked. From
    /// here on, no path mints a certificate for it -- not the resident
    /// loop's renewal work-list, not `submit`, not `force-reconcile`.
    /// Idempotent: revoking twice keeps the first timestamp, which is the
    /// one an operator would want to read.
    pub fn revoke_placement(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        revoked_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO revoked_placements (app_instance_id, logical_ref, revoked_at)
             VALUES (?1, ?2, ?3)",
            params![app_instance_id, logical_ref, revoked_at],
        )?;
        Ok(())
    }

    /// Every revoked placement of one app instance, by logical ref. Read
    /// once per pass and once per operator write, then consulted in memory
    /// -- a plan's services are checked against it individually.
    pub fn revoked_placements(&self, app_instance_id: &str) -> Result<BTreeSet<String>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT logical_ref FROM revoked_placements WHERE app_instance_id = ?1
             ORDER BY logical_ref ASC",
        )?;
        let mut rows = stmt.query(params![app_instance_id])?;
        let mut out = BTreeSet::new();
        while let Some(row) = rows.next()? {
            out.insert(row.get::<_, String>(0)?);
        }
        Ok(out)
    }

    /// Records that this member installed a fresh certificate but its
    /// `restart-on-rotation` restart failed, so it still owes one.
    /// Idempotent, keeping the first timestamp, the same shape as
    /// `revoke_placement`.
    pub fn mark_rotation_restart_owed(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        marked_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO pending_rotation_restarts (app_instance_id, logical_ref, \
             marked_at) VALUES (?1, ?2, ?3)",
            params![app_instance_id, logical_ref, marked_at],
        )?;
        Ok(())
    }

    /// Every member of one app instance still owing a rotation restart, by
    /// logical ref. Read once per pass, the same shape as
    /// `revoked_placements`.
    pub fn pending_rotation_restarts(&self, app_instance_id: &str) -> Result<BTreeSet<String>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT logical_ref FROM pending_rotation_restarts WHERE app_instance_id = ?1
             ORDER BY logical_ref ASC",
        )?;
        let mut rows = stmt.query(params![app_instance_id])?;
        let mut out = BTreeSet::new();
        while let Some(row) = rows.next()? {
            out.insert(row.get::<_, String>(0)?);
        }
        Ok(out)
    }

    /// Clears a member's owed rotation restart -- called once the restart
    /// actually succeeds, whatever pass it succeeds on.
    pub fn clear_rotation_restart_owed(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "DELETE FROM pending_rotation_restarts WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref],
        )?;
        Ok(())
    }

    /// Replaces desired state for `app_instance_id`, keeping exactly one
    /// row per instance -- a re-submit is a full replacement, not an
    /// additional version. Refused once the instance is retired: retiring
    /// is meant to hand an instance back to manual operation, and a submit
    /// that landed anyway would silently resume supervision behind the
    /// operator's back. `adopt` is the only way back in -- it clears the
    /// flag on a successful claim (`record_adopt`, below); plain `release`
    /// does not, since it only clears the substrate-side stamp and says
    /// nothing about whether *this* supervisor should resume managing the
    /// instance (N3, Slice A5b review round 2 -- the message here used to
    /// name `release` as an alternative, which changed nothing relevant
    /// and left the caller stuck with no error to explain why).
    ///
    /// Also refused when `generation` does not match the generation
    /// already on record for an existing instance: ADR-0021 §4 says the
    /// generation is minted by `adopt` and never otherwise, but nothing
    /// stopped a caller from presenting any value here, upward or
    /// downward, with no relation to the current one -- `adopt` was not
    /// really the only minter (H3, Slice A5b review). A brand-new
    /// instance (no existing row) is unaffected: it has no generation to
    /// contradict yet.
    pub fn submit(
        &self,
        app_instance_id: &str,
        plan_json: &str,
        inventory_json: &str,
        owner_did: &str,
        generation: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let existing: Option<(i64, i64)> = conn
            .query_row(
                "SELECT retired, generation FROM desired_state WHERE app_instance_id = ?1",
                params![app_instance_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((retired, existing_generation)) = existing {
            if retired == 1 {
                return Err(anyhow!(
                    "app instance '{app_instance_id}' is retired; run `supervisor adopt` to \
                     resume managing it before submitting new desired state"
                ));
            }
            if generation != existing_generation as u64 {
                return Err(anyhow!(
                    "submit presented generation {generation}, but app instance \
                     '{app_instance_id}' is on record at generation {existing_generation}; only \
                     `adopt` mints a new one -- run `supervisor adopt`, or omit --generation to \
                     resubmit at the current one"
                ));
            }
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
             retired, submitted_at, updated_at, app_master_did
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
                    app_master_did: row.get(9)?,
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
             retired, submitted_at, updated_at, app_master_did
             FROM desired_state WHERE retired = 0 AND paused = 0 ORDER BY app_instance_id ASC",
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
                app_master_did: row.get(9)?,
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

    /// A later `submit` is refused until the instance is re-adopted
    /// (D-A5-20's substrate-side release is the caller's counterpart --
    /// clearing the substrate's own stamp, not this row). Not terminal:
    /// `handle_adopt` un-retires as part of `record_adopt`'s combined
    /// write on a successful claim (M05A A7 review finding 6), which is
    /// the "re-adopted" this doc comment and every refusal message
    /// promise (N3, Slice A5b review round 2).
    pub fn retire(&self, app_instance_id: &str) -> Result<()> {
        self.set_flag(app_instance_id, "retired", true)
    }

    /// Updates the held generation in place, without touching the rest of
    /// desired state. Test-only hook (M05A A7 review round 2, finding C):
    /// `record_adopt`, below, is what `handle_adopt` actually calls in
    /// production, and it writes the generation together with the
    /// un-retired flag and the app master DID in one statement -- this
    /// method exists only to seed a generation in a test's setup step
    /// without also touching those other two columns, which
    /// `record_adopt` cannot do alone.
    #[cfg(test)]
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

    /// `adopt`'s own combined write, once the claim has succeeded: the
    /// generation, the un-retired flag, and the resolved app master DID,
    /// together in one statement (M05A A7 review finding 6). Before this,
    /// `handle_adopt` called `set_generation`/`un_retire`/
    /// `set_app_master_did` as three separate writes, so a crash between
    /// them could leave a claimed generation with no recorded app
    /// master -- exactly the state D-A7-4's "the row always agrees with
    /// the vault" claim rests on not happening.
    ///
    /// `clear_remediation_for_instance` stays a separate call at the
    /// caller: unlike these three fields, its own failure has never
    /// blocked `adopt` from succeeding (it is a `let _ =`-ignored
    /// best-effort clear today), and folding it in here would change
    /// that.
    pub fn record_adopt(
        &self,
        app_instance_id: &str,
        generation: u64,
        app_master_did: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let now = chrono::Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE desired_state SET generation = ?1, retired = 0, app_master_did = ?2, \
             updated_at = ?3 WHERE app_instance_id = ?4",
            params![generation as i64, app_master_did, now, app_instance_id],
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

    /// Test 26 (M05B B1, D-B1-5, failure-matrix row 13's supervisor half):
    /// the queue's tables live in the same database file `desired_state`
    /// does, not one of their own, so they inherit its protection posture
    /// rather than becoming a second unencrypted store beside it.
    #[test]
    fn the_queue_lives_in_supervisor_db_under_the_same_protection_as_desired_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        store.queue.enqueue("k", b"payload", 1_000).unwrap();

        assert!(
            !dir.path().join("supervisor.db-outbox").exists()
                && !dir.path().join("queue.db").exists(),
            "the queue must not open a database file of its own"
        );
        let conn = store.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "the outbox table must live in supervisor.db's own connection");
    }

    /// H3 (Slice A5b review): before the generation check, `submit` wrote
    /// whatever generation it was given straight over the stored one, so
    /// a caller could take an instance by submitting a large number
    /// rather than adopting -- `adopt` was not really the only minter
    /// (ADR-0021 §4). A resubmit at the *current* generation must still
    /// work (`submitting_twice_replaces_desired_state_and_keeps_one_row`
    /// above pins that at generation 0).
    #[test]
    fn submit_refuses_a_generation_that_does_not_match_the_one_on_record() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        store.set_generation("inst-1", 3).unwrap();

        let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 4).unwrap_err();
        assert!(err.to_string().contains("generation"), "{err}");
        let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap_err();
        assert!(err.to_string().contains("generation"), "{err}");

        store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 3).unwrap();
        assert_eq!(store.get("inst-1").unwrap().unwrap().plan_json, "{\"v\":2}");
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

    /// H2 (Slice A5b review): the doc comment on `all_active` promises
    /// "every non-retired, non-paused instance", but the query used to
    /// filter `retired` only -- free to fix before anything calls it, and
    /// a live bug the moment A5c's loop uses this as its work list.
    #[test]
    fn all_active_excludes_both_retired_and_paused_instances() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("running", "{}", "{}", "did:key:owner", 0).unwrap();
        store.submit("paused", "{}", "{}", "did:key:owner", 0).unwrap();
        store.pause("paused").unwrap();
        store.submit("retired", "{}", "{}", "did:key:owner", 0).unwrap();
        store.retire("retired").unwrap();

        let active: Vec<String> =
            store.all_active().unwrap().into_iter().map(|s| s.app_instance_id).collect();
        assert_eq!(active, vec!["running".to_string()]);
    }

    #[test]
    fn retire_refuses_a_later_submit_until_un_retired() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        store.retire("inst-1").unwrap();
        assert!(store.get("inst-1").unwrap().unwrap().retired);

        let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 1).unwrap_err();
        assert!(err.to_string().contains("retired"), "{err}");

        // N3 (Slice A5b review round 2): `retire` is not a dead end --
        // `record_adopt` (called by `handle_adopt` on a successful claim,
        // M05A A7 review finding 6) un-retires as part of its combined
        // write, which is the "run `supervisor adopt`" the refusal above
        // names.
        store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
        assert!(!store.get("inst-1").unwrap().unwrap().retired);
        store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();
        assert_eq!(store.get("inst-1").unwrap().unwrap().plan_json, "{\"v\":2}");
    }

    // ── M05A A5c: binding epochs (D-A5c-4) ──────────────────────────────

    /// The epoch is held **per dependent service**, not per dependency
    /// (§19.3, revised after review F2): there is only one row for
    /// "frontend", so every dependency it declares reads the identical
    /// value -- there is no per-dependency key to diverge in the first
    /// place.
    #[test]
    fn a_written_epoch_is_held_per_dependent_and_shared_by_its_dependencies() {
        let store = SupervisorStore::open_in_memory().unwrap();
        let epoch = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
        assert_eq!(epoch, 1);
        assert_eq!(store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 1);
    }

    /// F2's failure, pinned: the first draft's scalar `ApplyRequest.epoch`
    /// let a redeploy write a *lower* epoch over a higher one a push had
    /// already reached, so the next push then conflicted at an epoch the
    /// substrate had already served. The counter must only ever advance.
    #[test]
    fn a_redeploy_after_a_push_carries_an_epoch_above_what_was_pushed() {
        let store = SupervisorStore::open_in_memory().unwrap();
        let pushed = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
        let redeployed = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
        assert!(redeployed > pushed, "redeploy epoch {redeployed} must exceed push epoch {pushed}");
    }

    /// F3's false negative, pinned: an instance this supervisor has never
    /// written a binding for must read epoch 0, which is also what a
    /// hand-deployed substrate reports (`roymctl app deploy` emits every
    /// binding at epoch 0) -- so the pair reads converged, not stale.
    #[test]
    fn an_absent_row_reads_as_epoch_zero_so_a_hand_deployed_binding_converges() {
        let store = SupervisorStore::open_in_memory().unwrap();
        assert_eq!(store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 0);
    }

    // ── M05A A5c: remediation bookkeeping (§14 step 6, D-A5c-20) ────────

    /// §14 step 6's durability claim: a supervisor restart must resume
    /// remediation state, not reset every service's attempt count back to
    /// zero and re-earn the same restart budget again.
    #[test]
    fn remediation_attempts_survive_a_store_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
            store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
            store.record_restart_attempt("inst-1", "inst-1/backend", 1030).unwrap();
        }
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        let state = store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
        assert_eq!(state.attempts, 2);
        assert_eq!(state.last_attempt_at, Some(1030));
        assert!(!state.terminal);
    }

    #[test]
    fn a_healthy_sweep_clears_the_remediation_row() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
        assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_some());

        store.clear_remediation("inst-1", "inst-1/backend").unwrap();
        assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
    }

    /// F5 / D-A5c-20: `force-reconcile` is the escape hatch from a
    /// terminal remediation row -- a service nothing will restart again
    /// cannot become healthy on its own, so the healthy-sweep clearing
    /// path above never fires for it.
    #[test]
    fn force_reconcile_clears_a_terminal_remediation_row() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
        store.mark_remediation_terminal("inst-1", "inst-1/backend").unwrap();
        assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().terminal);

        // `force-reconcile`'s own clearing path: every remediation row for
        // the instance, not just this one service.
        store.clear_remediation_for_instance("inst-1").unwrap();
        assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
    }

    // ── M05A A5d: anchor-refresh bookkeeping and revoked placements ─────

    /// The refresh cadence is evaluated against this persisted fact on the
    /// ordinary pass tick rather than by a timer of its own, so the fact
    /// has to survive a supervisor restart -- otherwise every restart
    /// republishes every anchor immediately, which is load without
    /// benefit.
    #[test]
    fn store_persists_and_reads_back_last_refreshed_at_per_master() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
            assert_eq!(
                store.last_master_anchor_refresh("did:key:zMasterA").unwrap(),
                None,
                "a master this supervisor has never published for must read as overdue"
            );
            store.record_master_anchor_refresh("did:key:zMasterA", 1_000).unwrap();
            store.record_master_anchor_refresh("did:key:zMasterB", 2_000).unwrap();
            store.record_master_anchor_refresh("did:key:zMasterA", 3_000).unwrap();
        }
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        assert_eq!(store.last_master_anchor_refresh("did:key:zMasterA").unwrap(), Some(3_000));
        assert_eq!(store.last_master_anchor_refresh("did:key:zMasterB").unwrap(), Some(2_000));
    }

    /// D-A5d-15: the revocation exclusion is a persisted fact because more
    /// than one caller reads it -- the renewal work-list, `submit`, and
    /// `force-reconcile` -- and because it must outlive the process that
    /// recorded it.
    #[test]
    fn a_revoked_placement_is_recorded_per_member_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
            assert!(store.revoked_placements("inst-1").unwrap().is_empty());
            store.revoke_placement("inst-1", "inst-1/backend", 1_000).unwrap();
            // Idempotent, and scoped to the one member named.
            store.revoke_placement("inst-1", "inst-1/backend", 2_000).unwrap();
            store.revoke_placement("inst-2", "inst-2/backend", 1_000).unwrap();
        }
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        assert_eq!(
            store.revoked_placements("inst-1").unwrap(),
            BTreeSet::from(["inst-1/backend".to_string()])
        );
        assert_eq!(
            store.revoked_placements("inst-2").unwrap(),
            BTreeSet::from(["inst-2/backend".to_string()])
        );
    }

    /// F5 / D-A5c-20's second clearing path: `adopt` mints a new
    /// generation, a fresh start by construction, so it clears the same
    /// way `force-reconcile` does.
    #[test]
    fn adopt_clears_a_terminal_remediation_row() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
        store.mark_remediation_terminal("inst-1", "inst-1/backend").unwrap();

        // `adopt`'s clearing path is the identical store method --
        // both verbs mean "start over", so both call it.
        store.clear_remediation_for_instance("inst-1").unwrap();
        assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
    }

    // ── M05A A7: the app master column (D-A7-2/D-A7-11) ─────────────────

    /// D-A7-2: `CREATE TABLE IF NOT EXISTS` is a no-op on a database that
    /// already has `desired_state`, so a column added only there never
    /// reaches a pre-existing file -- every `desired_state` read then fails
    /// at runtime with "no such column". Opens a store, drops the column
    /// back out (simulating a pre-A7 database), reopens, and confirms the
    /// idempotent `ALTER TABLE` puts it back.
    #[test]
    fn a_database_that_predates_the_app_master_column_gains_it_on_open() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
            store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
            let conn = store.conn.lock().unwrap();
            conn.execute("ALTER TABLE desired_state DROP COLUMN app_master_did", [])
                .expect("this rusqlite's bundled sqlite must support DROP COLUMN");
        }
        // Reopening must not fail, and the reader must see the column back,
        // defaulted empty for the row that predates it.
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        let state = store.get("inst-1").unwrap().unwrap();
        assert_eq!(state.app_master_did, "");
    }

    /// D-A7-11: an `app-<instance>` vault key must never be forgotten --
    /// the same standing constraint the milestone already carries for
    /// member masters. Covers the two store-level paths that could
    /// plausibly clear it: a later `submit` (whose `ON CONFLICT` update
    /// list must leave `app_master_did` out) and `retire` (a `set_flag`
    /// call touching only its own column). `release` needs no case here --
    /// `SupervisorService::handle_release` never writes to `desired_state`
    /// at all, so it cannot clear this column any more than it clears
    /// anything else on the row.
    #[test]
    fn a_recorded_app_master_survives_a_resubmit_and_a_retire() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{\"v\":1}", "{}", "did:key:owner", 0).unwrap();
        store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
        assert_eq!(store.get("inst-1").unwrap().unwrap().app_master_did, "did:key:zAppMaster");

        // A later submit at the current generation replaces the plan but
        // must leave the app master alone.
        store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();
        let state = store.get("inst-1").unwrap().unwrap();
        assert_eq!(state.plan_json, "{\"v\":2}");
        assert_eq!(state.app_master_did, "did:key:zAppMaster");

        store.retire("inst-1").unwrap();
        assert_eq!(store.get("inst-1").unwrap().unwrap().app_master_did, "did:key:zAppMaster");
    }

    /// M05A A7 review finding 6: `handle_adopt`'s combined write -- the
    /// generation, the un-retired flag, and the app master DID all land
    /// in one statement, so there is no window where a crash could leave
    /// a claimed generation with no recorded DID.
    #[test]
    fn record_adopt_writes_generation_retired_and_app_master_did_together() {
        let store = SupervisorStore::open_in_memory().unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        store.retire("inst-1").unwrap();
        assert!(store.get("inst-1").unwrap().unwrap().retired);

        store.record_adopt("inst-1", 3, "did:key:zAppMaster").unwrap();

        let state = store.get("inst-1").unwrap().unwrap();
        assert_eq!(state.generation, 3);
        assert!(!state.retired, "record_adopt must also un-retire, like un_retire did");
        assert_eq!(state.app_master_did, "did:key:zAppMaster");
    }

    #[test]
    fn record_adopt_fails_for_an_instance_with_no_desired_state() {
        let store = SupervisorStore::open_in_memory().unwrap();
        let err = store.record_adopt("never-submitted", 1, "did:key:zAppMaster").unwrap_err();
        assert!(err.to_string().contains("no desired state"), "{err}");
    }
}

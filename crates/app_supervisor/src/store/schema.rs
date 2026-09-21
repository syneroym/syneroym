use super::*;

// Lock-poisoning from a panicking holder is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here, matching `StaticInventory`'s
// (`crates/app_orchestration/src/resolver.rs`).
impl SupervisorStore {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self> {
        Self::open_with_role(dir, db_name, &SupervisorRole::default())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, &SupervisorRole::default())
    }

    /// The construction path `runtime.rs` actually uses: `role`'s five
    /// `queue_*` fields size the outbox's attempt budget, backoff ceiling,
    /// visibility timeout, and DLQ cap. Every other caller --
    /// tests overwhelmingly among them -- goes through `open`/
    /// `open_in_memory` and gets the same defaults `SupervisorRole::default`
    /// does.
    pub fn open_with_role<P: AsRef<Path>>(
        dir: P,
        db_name: &str,
        role: &SupervisorRole,
    ) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {db_name}"));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::from_connection(conn, role)
    }

    fn from_connection(conn: Connection, role: &SupervisorRole) -> Result<Self> {
        Self::init_schema(&conn)?;
        // `QueueConfig::from` clamps a configured 0 to 1 silently -- it has
        // no `tracing` dependency of its own to log through. This is the
        // one caller that constructs it from operator config, so it is the
        // one that warns, mirroring `SupervisorService::new`'s existing
        // `max_renewals_per_pass == 0` clamp.
        if role.queue_max_attempts == 0 {
            tracing::warn!(
                "supervisor.queue_max_attempts was configured to 0, which would dead-letter every \
                 queued item on its first failure; clamped to 1"
            );
        }
        let conn = Arc::new(Mutex::new(conn));
        let journal = DeploymentJournal::from_connection(conn.clone())?;
        let alerts = AlertStore::from_connection(conn.clone())?;
        let queue = Queue::from_connection(conn.clone(), QueueConfig::from(role))?;
        Ok(Self { conn, journal, alerts, queue })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        for statement in SCHEMA_STATEMENTS {
            conn.execute_batch(statement)?;
        }
        Self::ensure_app_master_did_column(conn)
    }

    /// `desired_state` predates the `app_master_did` column, so the
    /// `CREATE TABLE IF NOT EXISTS` in `init_schema` is a no-op on any
    /// database that already exists -- it never adds a column to a table
    /// already there. This `ALTER TABLE` is the one idempotent way to get
    /// the column onto a database that opened before it existed; not the
    /// version ladder AGENTS.md rules out, since there is no schema version
    /// to track. Same shape as `RegistryStore`'s own `manifest_hash` column
    /// (`crates/data_db/src/registry_store.rs`). "duplicate column name" is
    /// the expected outcome on every open after the first.
    fn ensure_app_master_did_column(conn: &Connection) -> Result<()> {
        if let Err(err) = conn.execute(
            "ALTER TABLE desired_state ADD COLUMN app_master_did TEXT NOT NULL DEFAULT ''",
            [],
        ) && !err.to_string().contains("duplicate column name")
        {
            return Err(err.into());
        }
        Ok(())
    }
}

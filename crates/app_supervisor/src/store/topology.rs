use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
    /// `0` when nothing has ever been recorded -- "no epoch claimed", the
    /// same reading `EndpointInfo.generation`'s own default carries.
    pub fn topology_epoch(&self, app_instance_id: &str, service_name: &str) -> Result<u64> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT epoch FROM topology_epochs WHERE app_instance_id = ?1 AND service_name = ?2",
            params![app_instance_id, service_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|epoch| epoch.unwrap_or(0) as u64)
        .map_err(Into::into)
    }

    /// Records `fingerprint`, returning the epoch that now applies:
    /// unchanged when the fingerprint matches what is stored, `stored + 1`
    /// otherwise, `1` when nothing is stored. One statement, read back
    /// under the same store mutex, so two callers cannot interleave a read
    /// and a write around it.
    ///
    /// **`handle_submit` only.** This form advances the epoch, and
    /// advancing is only safe while the instance lock is held: the caller
    /// must have read the plan it is fingerprinting and written it durably
    /// with no other writer in between. `handle_resolve` holds no lock and
    /// uses `initialise_topology_epoch` instead.
    pub fn record_topology_fingerprint(
        &self,
        app_instance_id: &str,
        service_name: &str,
        fingerprint: &str,
    ) -> Result<u64> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO topology_epochs (app_instance_id, service_name, epoch, fingerprint)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(app_instance_id, service_name) DO UPDATE SET
                epoch = epoch + 1,
                fingerprint = excluded.fingerprint
             WHERE fingerprint != excluded.fingerprint",
            params![app_instance_id, service_name, fingerprint],
        )?;
        conn.query_row(
            "SELECT epoch FROM topology_epochs WHERE app_instance_id = ?1 AND service_name = ?2",
            params![app_instance_id, service_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|epoch| epoch as u64)
        .map_err(Into::into)
    }

    /// The insert-only counterpart, for a caller holding no instance lock:
    /// inserts at epoch `1` when no row exists -- the backfill an instance
    /// with no prior fingerprint record needs -- and otherwise changes
    /// nothing at all.
    ///
    /// Returns `(epoch, stored_fingerprint)`. The caller **must** compare
    /// the returned fingerprint with the one it passed: they differ exactly
    /// when a `submit` landed between the caller's plan read and this call,
    /// meaning the plan in hand is already stale. Signing then would pair
    /// the previous plan's members with the new plan's epoch, inside a
    /// signature, where it cannot be corrected afterwards.
    pub fn initialise_topology_epoch(
        &self,
        app_instance_id: &str,
        service_name: &str,
        fingerprint: &str,
    ) -> Result<(u64, String)> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO topology_epochs (app_instance_id, service_name, epoch, fingerprint)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(app_instance_id, service_name) DO NOTHING",
            params![app_instance_id, service_name, fingerprint],
        )?;
        conn.query_row(
            "SELECT epoch, fingerprint FROM topology_epochs
             WHERE app_instance_id = ?1 AND service_name = ?2",
            params![app_instance_id, service_name],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
        )
        .map_err(Into::into)
    }
}

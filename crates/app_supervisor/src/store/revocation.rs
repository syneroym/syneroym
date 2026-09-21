use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
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
}

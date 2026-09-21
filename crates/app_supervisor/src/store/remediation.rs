use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
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
    /// policy compares this against
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

    /// Marks this service's remediation terminal: exceeding
    /// `max_restart_attempts` stops the restart loop for this service
    /// until `force-reconcile` or `adopt` clears it.
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
    /// own clearing path: the service recovered on its own
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
}

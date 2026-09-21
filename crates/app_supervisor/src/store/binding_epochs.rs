use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
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
    /// -- the retry half of epoch-conflict handling: a `Stale(held)` push
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
}

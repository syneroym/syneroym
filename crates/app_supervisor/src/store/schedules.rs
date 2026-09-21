use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
    /// Every schedule state this instance has, keyed by logical ref -- the
    /// pass's own read before computing `schedule_decisions`.
    pub fn schedule_states(
        &self,
        app_instance_id: &str,
    ) -> Result<BTreeMap<String, ScheduleState>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT logical_ref, evaluated_at, last_run_at, last_member_index, last_error
             FROM scheduled_runs WHERE app_instance_id = ?1",
        )?;
        let mut rows = stmt.query(params![app_instance_id])?;
        let mut out = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let logical_ref: String = row.get(0)?;
            out.insert(
                logical_ref,
                ScheduleState {
                    evaluated_at: row.get(1)?,
                    last_run_at: row.get(2)?,
                    last_member_index: row.get::<_, Option<i64>>(3)?.map(|i| i as u32),
                    last_error: row.get(4)?,
                },
            );
        }
        Ok(out)
    }

    /// Advances the watermark and nothing else -- a pass that looked and
    /// found nothing due (no occurrence in the window, no healthy member,
    /// an unresolvable target). This is what makes a missed tick a skip
    /// rather than a backlog.
    pub fn record_schedule_evaluated(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO scheduled_runs (app_instance_id, logical_ref, evaluated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(app_instance_id, logical_ref) DO UPDATE SET
                evaluated_at = excluded.evaluated_at",
            params![app_instance_id, logical_ref, at],
        )?;
        Ok(())
    }

    /// Watermark, run time, and selected member, in one statement -- written
    /// **before** the call, not after, so a supervisor that dies inside the
    /// call skips this tick on restart rather than repeating it.
    pub fn record_schedule_started(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        at: i64,
        member_index: u32,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO scheduled_runs
                (app_instance_id, logical_ref, evaluated_at, last_run_at, last_member_index)
             VALUES (?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(app_instance_id, logical_ref) DO UPDATE SET
                evaluated_at = excluded.evaluated_at,
                last_run_at = excluded.last_run_at,
                last_member_index = excluded.last_member_index",
            params![app_instance_id, logical_ref, at, member_index],
        )?;
        Ok(())
    }

    /// Sets or clears `last_error` after the call returns -- `None` on
    /// success, `Some(detail)` on failure or timeout. Never touches
    /// `evaluated_at`/`last_run_at`/`last_member_index`, which
    /// `record_schedule_started` already wrote for this tick.
    pub fn record_schedule_outcome(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "UPDATE scheduled_runs SET last_error = ?3
             WHERE app_instance_id = ?1 AND logical_ref = ?2",
            params![app_instance_id, logical_ref, error],
        )?;
        Ok(())
    }

    /// Drops the rows of an instance's schedules that its plan no longer
    /// declares. A `schedule` block removed from a manifest and resubmitted
    /// otherwise leaves its row behind forever: nothing else deletes one
    /// short of retiring the whole instance, and `schedules` reads from the
    /// plan, so the row is invisible as well as unreachable. Called from
    /// the reconcile pass, which is the one place that already holds the
    /// full declared set.
    pub fn prune_schedule_states(
        &self,
        app_instance_id: &str,
        declared: &BTreeSet<String>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        let mut stmt =
            conn.prepare("SELECT logical_ref FROM scheduled_runs WHERE app_instance_id = ?1")?;
        let stored: Vec<String> = stmt
            .query_map(params![app_instance_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for logical_ref in stored.iter().filter(|r| !declared.contains(*r)) {
            conn.execute(
                "DELETE FROM scheduled_runs WHERE app_instance_id = ?1 AND logical_ref = ?2",
                params![app_instance_id, logical_ref],
            )?;
        }
        Ok(())
    }

    /// Clears an instance's per-run schedule bookkeeping -- `adopt` and
    /// `force-reconcile`'s own fresh-start path, the same shape
    /// `clear_remediation_for_instance` already takes and called from the
    /// same two sites.
    ///
    /// `evaluated_at` is deliberately kept. It is not per-run state: it
    /// records how far this instance's schedules have already been looked
    /// at, and a supervisor that is running and reachable enough to be
    /// force-reconciled has genuinely looked that far. Deleting the row
    /// would make the next pass treat the schedule as first-sight, which
    /// fires nothing -- so a `force-reconcile` at 03:00:30 would swallow
    /// the 03:00 tick even though nothing was ever down. Keeping the
    /// watermark cannot produce a backlog either: the grace window still
    /// bounds how far back the next pass may look.
    pub fn clear_schedule_state_for_instance(&self, app_instance_id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "UPDATE scheduled_runs
                SET last_run_at = NULL, last_member_index = NULL, last_error = NULL
             WHERE app_instance_id = ?1",
            params![app_instance_id],
        )?;
        Ok(())
    }
}

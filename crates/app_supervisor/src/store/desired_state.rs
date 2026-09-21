use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
    /// The instance this app master DID belongs to. `None` for a DID no
    /// instance on this supervisor has ever recorded -- which `resolve`
    /// deliberately reports identically to "not authorized".
    pub fn instance_by_app_master_did(&self, app_master_did: &str) -> Result<Option<DesiredState>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT app_instance_id, plan_json, inventory_json, owner_did, generation, paused, \
             retired, submitted_at, updated_at, app_master_did
             FROM desired_state WHERE app_master_did = ?1 AND app_master_did != ''",
            params![app_master_did],
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

    /// Replaces desired state for `app_instance_id`, keeping exactly one
    /// row per instance -- a re-submit is a full replacement, not an
    /// additional version. Refused once the instance is retired: retiring
    /// is meant to hand an instance back to manual operation, and a submit
    /// that landed anyway would silently resume supervision behind the
    /// operator's back. `adopt` is the only way back in -- it clears the
    /// flag on a successful claim (`record_adopt`, below); plain `release`
    /// does not, since it only clears the substrate-side stamp and says
    /// nothing about whether *this* supervisor should resume managing the
    /// instance. The refusal message names only `adopt`: naming `release`
    /// as an alternative changed nothing relevant and left the caller
    /// stuck with no error to explain why.
    ///
    /// Also refused when `generation` does not match the generation
    /// already on record for an existing instance: ADR-0021 §4 says the
    /// generation is minted by `adopt` and never otherwise, but nothing
    /// stopped a caller from presenting any value here, upward or
    /// downward, with no relation to the current one -- `adopt` was not
    /// really the only minter. A brand-new instance (no existing row) is
    /// unaffected: it has no generation to contradict yet.
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

    /// A later `submit` is refused until the instance is re-adopted (the
    /// substrate-side release is the caller's counterpart -- clearing the
    /// substrate's own stamp, not this row). Not terminal: `handle_adopt`
    /// un-retires as part of `record_adopt`'s combined write on a
    /// successful claim, which is the "re-adopted" this doc comment and
    /// every refusal message promise.
    pub fn retire(&self, app_instance_id: &str) -> Result<()> {
        self.set_flag(app_instance_id, "retired", true)
    }

    /// Updates the held generation in place, without touching the rest of
    /// desired state. Test-only hook:
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
    /// together in one statement. Before this, `handle_adopt` called
    /// `set_generation`/`un_retire`/`set_app_master_did` as three separate
    /// writes, so a crash between them could leave a claimed generation
    /// with no recorded app master -- breaking the invariant that the row
    /// always agrees with the vault.
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

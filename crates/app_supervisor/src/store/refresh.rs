use super::*;

#[allow(clippy::expect_used)]
impl SupervisorStore {
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

    /// When this app instance's Tier-1 registry record was last
    /// republished, or `None` if this supervisor never has. An absent row
    /// reads as "overdue", the same first-pass reading
    /// `last_master_anchor_refresh` gives -- a Tier-1 record this
    /// supervisor has no record of publishing may not exist at the
    /// registry at all.
    pub fn last_tier1_refresh(&self, app_did: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.query_row(
            "SELECT last_refreshed_at FROM app_tier1_refresh WHERE app_did = ?1",
            params![app_did],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Stamps a successful Tier-1 publish. Only called after the writer
    /// returns cleanly -- a failed publish must leave the previous stamp
    /// alone so the next pass retries.
    pub fn record_tier1_refresh(&self, app_did: &str, at: i64) -> Result<()> {
        let conn = self.conn.lock().expect("supervisor store connection lock poisoned");
        conn.execute(
            "INSERT INTO app_tier1_refresh (app_did, last_refreshed_at)
             VALUES (?1, ?2)
             ON CONFLICT(app_did) DO UPDATE SET last_refreshed_at = excluded.last_refreshed_at",
            params![app_did, at],
        )?;
        Ok(())
    }
}

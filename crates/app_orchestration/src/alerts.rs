//! Alerts raised by a health sweep, and their active/cleared lifecycle.
//!
//! Deliberately its own store rather than more tables on `DeploymentJournal`:
//! A4's sweep is an operator-local process writing beside `deployments.db`,
//! while A5's supervisor is a substrate role with its own database, and a
//! substrate cannot open a client-side file. What carries across is this
//! schema, these types, and `sdk::health::record_report`'s folding logic --
//! A5 changes only the `Connection` handed to `AlertStore::open`.

use std::{
    fmt,
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::models::AppInstanceId;

/// Why an alert was raised. One variant per distinct signal, because
/// remediation differs per signal (task.md A4) -- collapsing them would
/// erase the distinction A4 exists to establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlertKind {
    /// The substrate itself did not answer. Raised once per substrate, never
    /// per service (D-A4-13).
    SubstrateUnreachable,
    /// The substrate answered and says the instance is down or absent.
    InstanceNotRunning,
    /// The declared readiness probe fails. Reachable from a `running` phase
    /// and from an `unknown` one -- a `tcp` service has no other signal.
    ProbeFailing,
    /// The installed instance certificate is within 25% of its lifetime of
    /// expiring.
    CertificateNearExpiry,
    /// The installed instance certificate's validity window has already
    /// ended -- a current outage (failure-matrix rows 1/3 under the
    /// attended posture), not a renewal reminder (A4-04).
    CertificateExpired,
    /// A managed substrate reports a held generation higher than this
    /// supervisor's own (ADR-0021 §4, failure-matrix row 9): another
    /// supervisor has adopted the instance. This supervisor stops managing
    /// it rather than bumping its own generation to match.
    SupervisorSuperseded,
    /// Bounded restart-in-place exhausted `max_restart_attempts` without the
    /// service becoming healthy (M05A A5c, failure-matrix row 13). Terminal:
    /// the service is not restarted again until `force-reconcile` or `adopt`
    /// clears it (D-A5c-20).
    RemediationExhausted,
    /// A binding write landed at the current epoch with different content
    /// than the substrate already held -- the two-writer signal ADR-0021 §3
    /// requires be reported distinctly from a stale rejection (M05A A5c,
    /// D-A5c-4/D-A5c-19).
    BindingConflict,
    /// A re-submit or `force-reconcile` tried to move a service to a
    /// different substrate than where it is already landed. Refused, not
    /// retried -- relocation is a milestone non-goal (M05A A5c, D-A5c-1).
    PlacementChangeRefused,
    /// A service the stored plan no longer names, but that the loop found
    /// still running -- dropped by a plan-level edit rather than by the
    /// operator. Not undeployed (D-A5c-3): undeploying a stateful service
    /// because a manifest was edited is destructive, so this is a standing
    /// alert instead.
    OrphanedService,
    /// The supervisor's own vault holds no KEK, so it cannot read the
    /// member master this service's certificate must be reissued from
    /// (M05A A5d). The ordinary state of a freshly-booted supervisor,
    /// since the KEK arrives by `security.inject-kek` and does not survive
    /// a restart -- and, once renewal is automated, the single thing
    /// standing between a routine restart and every managed member's
    /// handshakes failing closed. Raised per affected member, the same
    /// fan-out `SubstrateUnreachable` uses for one root cause touching
    /// several rows.
    VaultLocked,
    /// An operator revoked this placement's instance key, so nothing
    /// reinstalls or re-certifies it -- not the resident loop, not
    /// `submit`, not `force-reconcile` (M05A A5d). Distinct from
    /// `OrphanedService`: that one is a plan edit the supervisor declined
    /// to act on, this one is an operator decision it is actively
    /// enforcing.
    InstanceRevoked,
    /// A renewal installed a fresh certificate but the member's
    /// `restart-on-rotation` restart then failed. The certificate alone
    /// settles the health poll, so this is the one
    /// alert kind that survives a healthy renewal -- cleared only once the
    /// restart itself succeeds, not by the certificate window closing.
    RotationRestartPending,
    /// A queued binding write exhausted its delivery attempt budget
    /// (M05B B1, D-B1-6). One standing row per `(instance, logical_ref,
    /// substrate)` with the current dead-letter count in `detail`,
    /// refreshed as more accumulate -- `AlertStore`'s unique index cannot
    /// express one row per item, and an operator wants the standing fact
    /// ("this member has undeliverable work") over a flood of individual
    /// rows anyway. Cleared when that key's last dead letter is gone, by
    /// replay or by prune.
    DeliveryExhausted,
}

impl fmt::Display for AlertKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::SubstrateUnreachable => "SUBSTRATE_UNREACHABLE",
            Self::InstanceNotRunning => "INSTANCE_NOT_RUNNING",
            Self::ProbeFailing => "PROBE_FAILING",
            Self::CertificateNearExpiry => "CERTIFICATE_NEAR_EXPIRY",
            Self::CertificateExpired => "CERTIFICATE_EXPIRED",
            Self::SupervisorSuperseded => "SUPERVISOR_SUPERSEDED",
            Self::RemediationExhausted => "REMEDIATION_EXHAUSTED",
            Self::BindingConflict => "BINDING_CONFLICT",
            Self::PlacementChangeRefused => "PLACEMENT_CHANGE_REFUSED",
            Self::OrphanedService => "ORPHANED_SERVICE",
            Self::VaultLocked => "VAULT_LOCKED",
            Self::InstanceRevoked => "INSTANCE_REVOKED",
            Self::RotationRestartPending => "ROTATION_RESTART_PENDING",
            Self::DeliveryExhausted => "DELIVERY_EXHAUSTED",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for AlertKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SUBSTRATE_UNREACHABLE" => Ok(Self::SubstrateUnreachable),
            "INSTANCE_NOT_RUNNING" => Ok(Self::InstanceNotRunning),
            "PROBE_FAILING" => Ok(Self::ProbeFailing),
            "CERTIFICATE_NEAR_EXPIRY" => Ok(Self::CertificateNearExpiry),
            "CERTIFICATE_EXPIRED" => Ok(Self::CertificateExpired),
            "SUPERVISOR_SUPERSEDED" => Ok(Self::SupervisorSuperseded),
            "REMEDIATION_EXHAUSTED" => Ok(Self::RemediationExhausted),
            "BINDING_CONFLICT" => Ok(Self::BindingConflict),
            "PLACEMENT_CHANGE_REFUSED" => Ok(Self::PlacementChangeRefused),
            "ORPHANED_SERVICE" => Ok(Self::OrphanedService),
            "VAULT_LOCKED" => Ok(Self::VaultLocked),
            "INSTANCE_REVOKED" => Ok(Self::InstanceRevoked),
            "ROTATION_RESTART_PENDING" => Ok(Self::RotationRestartPending),
            "DELIVERY_EXHAUSTED" => Ok(Self::DeliveryExhausted),
            _ => Err(anyhow!("Unknown alert kind: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRecord {
    pub id: i64,
    pub instance_id: AppInstanceId,
    /// `None` for a substrate-level alert, which belongs to no one service.
    pub logical_ref: Option<String>,
    pub substrate_alias: Option<String>,
    pub substrate_did: String,
    pub kind: AlertKind,
    pub detail: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    /// `None` while the signal is still present.
    pub cleared_at: Option<i64>,
}

/// `conn: Arc<Mutex<Connection>>` (M05A A5b, D-A5-8) -- see
/// `DeploymentJournal`'s identical field doc for why.
#[derive(Debug, Clone)]
pub struct AlertStore {
    conn: Arc<Mutex<Connection>>,
}

// Lock-poisoning from a panicking holder is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here, matching `StaticInventory`'s.
#[allow(clippy::expect_used)]
impl AlertStore {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {}", db_name));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Wraps an already-open connection -- see
    /// `DeploymentJournal::from_connection`.
    pub fn from_connection(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        Self::init_schema(&conn.lock().expect("alert store connection lock poisoned"))?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Unconditional, not gated on `PRAGMA user_version`: pre-release, schema
        // changes are made in place with no version ladder, and `IF NOT EXISTS`
        // is already idempotent.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS alerts (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                instance_id     TEXT NOT NULL,
                logical_ref     TEXT,
                substrate_alias TEXT,
                substrate_did   TEXT NOT NULL,
                kind            TEXT NOT NULL,
                detail          TEXT NOT NULL,
                first_seen_at   INTEGER NOT NULL,
                last_seen_at    INTEGER NOT NULL,
                cleared_at      INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_alerts_instance ON alerts(instance_id);
             -- One *active* row per (instance, ref, substrate, kind). A partial
             -- unique index rather than application-side checking: a second
             -- sweep must refresh the existing row, never open a duplicate, and
             -- the same signal seen again after being cleared is a genuinely
             -- new incident with its own row.
             CREATE UNIQUE INDEX IF NOT EXISTS idx_alerts_active
                ON alerts(instance_id, IFNULL(logical_ref,''), substrate_did, kind)
                WHERE cleared_at IS NULL;",
        )?;
        Ok(())
    }

    /// Raises `kind`, or refreshes it if an active row already exists.
    /// Returns `true` when this call opened a **new** incident, so a caller
    /// can print only the transitions rather than the whole standing set on
    /// every sweep.
    #[allow(clippy::too_many_arguments)]
    pub fn raise(
        &self,
        instance_id: &AppInstanceId,
        logical_ref: Option<&str>,
        substrate_alias: Option<&str>,
        substrate_did: &str,
        kind: AlertKind,
        detail: &str,
    ) -> Result<bool> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().expect("alert store connection lock poisoned");
        let updated = conn.execute(
            "UPDATE alerts SET last_seen_at = ?1, detail = ?2
             WHERE instance_id = ?3 AND IFNULL(logical_ref,'') = IFNULL(?4,'')
               AND substrate_did = ?5 AND kind = ?6 AND cleared_at IS NULL",
            params![
                now,
                detail,
                instance_id.as_str(),
                logical_ref,
                substrate_did,
                kind.to_string()
            ],
        )?;
        if updated > 0 {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO alerts (instance_id, logical_ref, substrate_alias, substrate_did, kind, \
             detail, first_seen_at, last_seen_at, cleared_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL)",
            params![
                instance_id.as_str(),
                logical_ref,
                substrate_alias,
                substrate_did,
                kind.to_string(),
                detail,
                now
            ],
        )?;
        Ok(true)
    }

    /// Marks the matching active alert cleared. Returns `true` when one was
    /// actually cleared. Idempotent: clearing a signal that was never raised
    /// is a no-op, which is the ordinary case on a healthy sweep.
    pub fn clear(
        &self,
        instance_id: &AppInstanceId,
        logical_ref: Option<&str>,
        substrate_did: &str,
        kind: AlertKind,
    ) -> Result<bool> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().expect("alert store connection lock poisoned");
        let updated = conn.execute(
            "UPDATE alerts SET cleared_at = ?1
             WHERE instance_id = ?2 AND IFNULL(logical_ref,'') = IFNULL(?3,'')
               AND substrate_did = ?4 AND kind = ?5 AND cleared_at IS NULL",
            params![now, instance_id.as_str(), logical_ref, substrate_did, kind.to_string()],
        )?;
        Ok(updated > 0)
    }

    /// Active alerts for an instance, oldest first.
    pub fn active(&self, instance_id: &AppInstanceId) -> Result<Vec<AlertRecord>> {
        self.query(
            "SELECT id, instance_id, logical_ref, substrate_alias, substrate_did, kind, detail, \
             first_seen_at, last_seen_at, cleared_at
             FROM alerts WHERE instance_id = ?1 AND cleared_at IS NULL ORDER BY id ASC",
            instance_id,
        )
    }

    /// Every alert for an instance including cleared ones, oldest first.
    pub fn all(&self, instance_id: &AppInstanceId) -> Result<Vec<AlertRecord>> {
        self.query(
            "SELECT id, instance_id, logical_ref, substrate_alias, substrate_did, kind, detail, \
             first_seen_at, last_seen_at, cleared_at
             FROM alerts WHERE instance_id = ?1 ORDER BY id ASC",
            instance_id,
        )
    }

    fn query(&self, sql: &str, instance_id: &AppInstanceId) -> Result<Vec<AlertRecord>> {
        let conn = self.conn.lock().expect("alert store connection lock poisoned");
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query(params![instance_id.as_str()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let inst_str: String = row.get(1)?;
            let kind_str: String = row.get(5)?;
            out.push(AlertRecord {
                id: row.get(0)?,
                instance_id: AppInstanceId::new(inst_str),
                logical_ref: row.get(2)?,
                substrate_alias: row.get(3)?,
                substrate_did: row.get(4)?,
                kind: kind_str.parse()?,
                detail: row.get(6)?,
                first_seen_at: row.get(7)?,
                last_seen_at: row.get(8)?,
                cleared_at: row.get(9)?,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(s: &str) -> AppInstanceId {
        AppInstanceId::new(s)
    }

    #[test]
    fn raising_the_same_alert_twice_refreshes_one_row_rather_than_opening_two() {
        let store = AlertStore::open_in_memory().unwrap();
        let inst = instance("inst-1");
        let opened_first = store
            .raise(
                &inst,
                Some("backend"),
                Some("edge-b"),
                "did:key:b",
                AlertKind::ProbeFailing,
                "boom",
            )
            .unwrap();
        let opened_second = store
            .raise(
                &inst,
                Some("backend"),
                Some("edge-b"),
                "did:key:b",
                AlertKind::ProbeFailing,
                "boom again",
            )
            .unwrap();
        assert!(opened_first);
        assert!(!opened_second);
        let active = store.active(&inst).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].detail, "boom again");
    }

    #[test]
    fn clearing_an_alert_that_was_never_raised_is_a_no_op() {
        let store = AlertStore::open_in_memory().unwrap();
        let inst = instance("inst-1");
        let cleared = store
            .clear(&inst, Some("backend"), "did:key:b", AlertKind::InstanceNotRunning)
            .unwrap();
        assert!(!cleared);
        assert!(store.active(&inst).unwrap().is_empty());
    }

    #[test]
    fn an_alert_raised_again_after_clearing_is_a_new_incident() {
        let store = AlertStore::open_in_memory().unwrap();
        let inst = instance("inst-1");
        store
            .raise(&inst, Some("backend"), None, "did:key:b", AlertKind::InstanceNotRunning, "down")
            .unwrap();
        assert!(
            store
                .clear(&inst, Some("backend"), "did:key:b", AlertKind::InstanceNotRunning)
                .unwrap()
        );
        let opened_again = store
            .raise(
                &inst,
                Some("backend"),
                None,
                "did:key:b",
                AlertKind::InstanceNotRunning,
                "down again",
            )
            .unwrap();
        assert!(opened_again);
        let all = store.all(&inst).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(store.active(&inst).unwrap().len(), 1);
    }

    /// M05A A5c §19.6: `AlertKind` has a `Display` and a `FromStr`, and a
    /// variant added to one and not the other makes its own stored rows
    /// unreadable at the next `alerts` call with no compile error. Every
    /// variant, named explicitly rather than derived, so a future addition
    /// here must also be added to this list.
    #[test]
    fn every_alert_kind_round_trips_through_display_and_from_str() {
        let all = [
            AlertKind::SubstrateUnreachable,
            AlertKind::InstanceNotRunning,
            AlertKind::ProbeFailing,
            AlertKind::CertificateNearExpiry,
            AlertKind::CertificateExpired,
            AlertKind::SupervisorSuperseded,
            AlertKind::RemediationExhausted,
            AlertKind::BindingConflict,
            AlertKind::PlacementChangeRefused,
            AlertKind::OrphanedService,
            AlertKind::VaultLocked,
            AlertKind::InstanceRevoked,
            AlertKind::RotationRestartPending,
            AlertKind::DeliveryExhausted,
        ];
        for kind in all {
            let round_tripped: AlertKind = kind.to_string().parse().unwrap();
            assert_eq!(round_tripped, kind, "{kind} did not round-trip");
        }
    }

    #[test]
    fn active_excludes_cleared_ones_and_all_includes_them() {
        let store = AlertStore::open_in_memory().unwrap();
        let inst = instance("inst-1");
        store
            .raise(
                &inst,
                None,
                Some("edge-b"),
                "did:key:b",
                AlertKind::SubstrateUnreachable,
                "down",
            )
            .unwrap();
        store.clear(&inst, None, "did:key:b", AlertKind::SubstrateUnreachable).unwrap();
        assert!(store.active(&inst).unwrap().is_empty());
        assert_eq!(store.all(&inst).unwrap().len(), 1);
    }
}

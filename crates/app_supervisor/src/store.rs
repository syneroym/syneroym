//! Desired state for every app instance this supervisor manages, over the
//! same `Arc<Mutex<Connection>>` that backs `DeploymentJournal`,
//! `AlertStore`, and the durable outbox queue: one SQLite file, four
//! concerns.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_app_orchestration::{AlertStore, DeploymentJournal};
use syneroym_async_queue::{Queue, QueueConfig};
use syneroym_core::config::SupervisorRole;

mod binding_epochs;
mod desired_state;
mod refresh;
mod remediation;
mod revocation;
mod schedules;
mod schema;
mod topology;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

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
    /// The app instance's own master DID, empty
    /// until the instance's next `adopt` mints or resolves one -- the vault
    /// cannot be enumerated and this instance appears in no plan, so this
    /// column is the only index, not a cache of something else readable.
    pub app_master_did: String,
}

/// One service's bounded-restart bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemediationState {
    pub attempts: u32,
    pub last_attempt_at: Option<i64>,
    pub terminal: bool,
}

/// One logical service's scheduled-task bookkeeping (ADR-0023 §6).
/// `evaluated_at` is the watermark that makes a missed tick a
/// skip rather than a backlog: it advances on every pass that looks at the
/// schedule, whether or not a run happens. `last_run_at` and
/// `last_member_index` describe only the most recent *run*, which is why
/// they can lag `evaluated_at` on a pass that looked and found nothing due,
/// and why both are `None` until a run has actually happened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleState {
    pub evaluated_at: i64,
    pub last_run_at: Option<i64>,
    pub last_member_index: Option<u32>,
    pub last_error: Option<String>,
}

/// One SQLite file holding desired state, the deployment journal, alerts,
/// and the durable outbox queue -- one file, four concerns, sharing the one
/// connection.
#[derive(Debug, Clone)]
pub struct SupervisorStore {
    conn: Arc<Mutex<Connection>>,
    pub journal: DeploymentJournal,
    pub alerts: AlertStore,
    pub queue: Queue,
}

/// Every `CREATE TABLE` this store needs, one statement per entry, applied
/// in order on open. Unconditional, not gated on `PRAGMA user_version`:
/// pre-release, schema changes are made in place with no version ladder,
/// and `IF NOT EXISTS` is already idempotent.
const SCHEMA_STATEMENTS: &[&str] = &[
    DESIRED_STATE_TABLE,
    BINDING_EPOCHS_TABLE,
    REMEDIATION_TABLE,
    MASTER_ANCHOR_REFRESH_TABLE,
    APP_TIER1_REFRESH_TABLE,
    TOPOLOGY_EPOCHS_TABLE,
    REVOKED_PLACEMENTS_TABLE,
    PENDING_ROTATION_RESTARTS_TABLE,
    SCHEDULED_RUNS_TABLE,
];

const DESIRED_STATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS desired_state (
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
     );";

// One counter per *dependent service*, not per dependency -- every binding
// that service emits shares this one value. An absent row reads as epoch 0,
// meaning "no supervisor has written here" (the same meaning
// `roymctl app deploy`'s own epoch-0 writes already carry), so a
// hand-deployed instance this supervisor has never pushed to converges
// rather than reading as a false negative.
const BINDING_EPOCHS_TABLE: &str = "CREATE TABLE IF NOT EXISTS binding_epochs (
        app_instance_id TEXT NOT NULL,
        logical_ref     TEXT NOT NULL,
        epoch           INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (app_instance_id, logical_ref)
     );";

// Attempt bookkeeping for bounded restart-in-place remediation, durable
// across a supervisor restart. `terminal` is cleared by `force-reconcile`
// and `adopt`, not only by a healthy sweep -- a terminal
// `InstanceNotRunning` service is never restarted again, so the sweep that
// would otherwise clear it never fires.
const REMEDIATION_TABLE: &str = "CREATE TABLE IF NOT EXISTS remediation (
        app_instance_id TEXT NOT NULL,
        logical_ref     TEXT NOT NULL,
        attempts        INTEGER NOT NULL DEFAULT 0,
        last_attempt_at INTEGER,
        terminal        INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (app_instance_id, logical_ref)
     );";

// When each managed master's anchor was last republished. Keyed by master
// DID rather than by instance, because the anchor belongs to the master:
// two instances naming the same master must not each refresh it on their
// own schedule. Read every pass and compared against
// `master_anchor_refresh_interval_secs`, so the refresh needs no timer of
// its own.
const MASTER_ANCHOR_REFRESH_TABLE: &str = "CREATE TABLE IF NOT EXISTS master_anchor_refresh (
        master_did        TEXT PRIMARY KEY,
        last_refreshed_at INTEGER NOT NULL
     );";

// ADR-0022 §2: when each app instance's Tier-1 registry record was last
// republished. Keyed by app DID rather than by `app_instance_id`, the same
// reasoning `master_anchor_refresh` uses for member masters -- and, unlike
// that table, there is exactly one app master per instance, so no fan-out
// case applies. Read every pass and compared against
// `master_anchor_refresh_interval_secs` (the same cadence, reused rather
// than given a second config field), so this needs no timer of its own.
const APP_TIER1_REFRESH_TABLE: &str = "CREATE TABLE IF NOT EXISTS app_tier1_refresh (
        app_did           TEXT PRIMARY KEY,
        last_refreshed_at INTEGER NOT NULL
     );";

// ADR-0022 §3/§6: the per-logical-service topology epoch a Tier-2 document
// carries and shard rebalancing will fence the data path on. Distinct from
// `binding_epochs`, which counts writes pushed to one *dependent* and moves
// for reasons unrelated to a member set changing. `fingerprint` is what
// decides whether a submit is a change at all. Rows are never deleted: a
// service removed from a plan and re-added later must not reuse a lower
// epoch.
const TOPOLOGY_EPOCHS_TABLE: &str = "CREATE TABLE IF NOT EXISTS topology_epochs (
        app_instance_id TEXT NOT NULL,
        service_name    TEXT NOT NULL,
        epoch           INTEGER NOT NULL,
        fingerprint     TEXT NOT NULL,
        PRIMARY KEY (app_instance_id, service_name)
     );";

// Placements whose instance key an operator has revoked. Read by *every*
// path that can mint a certificate -- the renewal work-list, `submit`, and
// `force-reconcile` alike -- because revoking a key and then silently
// re-minting one on the next ordinary redeploy is not a revocation at all.
const REVOKED_PLACEMENTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS revoked_placements (
        app_instance_id TEXT NOT NULL,
        logical_ref     TEXT NOT NULL,
        revoked_at      INTEGER NOT NULL,
        PRIMARY KEY (app_instance_id, logical_ref)
     );";

// A member whose renewal installed a fresh certificate but whose
// `restart-on-rotation` restart then failed. The certificate alone settles
// the health poll, so nothing else remembers the process still needs that
// restart -- this is the one thing that does, independent of the renewal
// alert's own lifecycle.
const PENDING_ROTATION_RESTARTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS \
                                               pending_rotation_restarts (
        app_instance_id TEXT NOT NULL,
        logical_ref     TEXT NOT NULL,
        marked_at       INTEGER NOT NULL,
        PRIMARY KEY (app_instance_id, logical_ref)
     );";

// A schedule belongs to the logical service, not to a member, and is never
// queued (ADR-0023 §3) -- this table, not the outbox, is its whole durable
// state. `evaluated_at` is the watermark; `last_run_at` is written before
// the call, not after, so a supervisor that dies mid-run skips the tick on
// restart rather than repeating it. `last_member_index` is NULL until a run
// has actually happened: 0 would read as member 0 having already run, and
// send a multi-member service's very first tick to member 1.
const SCHEDULED_RUNS_TABLE: &str = "CREATE TABLE IF NOT EXISTS scheduled_runs (
        app_instance_id   TEXT    NOT NULL,
        logical_ref       TEXT    NOT NULL,
        evaluated_at      INTEGER NOT NULL,
        last_run_at       INTEGER,
        last_member_index INTEGER,
        last_error        TEXT,
        PRIMARY KEY (app_instance_id, logical_ref)
     );";

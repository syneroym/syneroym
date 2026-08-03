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

use crate::models::{AppInstanceId, DeploymentPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeploymentState {
    Planned,
    Applying,
    Active,
    /// Some services applied and some did not. No rollback (ADR-0021 §5 /
    /// task.md non-goals): rolling back a stateful service is itself
    /// destructive, so the deployment stays here until a re-run completes
    /// the missing actions.
    Degraded,
    RollingBack,
    RolledBack,
}

impl fmt::Display for DeploymentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Planned => "PLANNED",
            Self::Applying => "APPLYING",
            Self::Active => "ACTIVE",
            Self::Degraded => "DEGRADED",
            Self::RollingBack => "ROLLING_BACK",
            Self::RolledBack => "ROLLED_BACK",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for DeploymentState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PLANNED" => Ok(Self::Planned),
            "APPLYING" => Ok(Self::Applying),
            "ACTIVE" => Ok(Self::Active),
            "DEGRADED" => Ok(Self::Degraded),
            "ROLLING_BACK" => Ok(Self::RollingBack),
            "ROLLED_BACK" => Ok(Self::RolledBack),
            _ => Err(anyhow!("Unknown deployment state: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentRecord {
    pub id: i64,
    pub instance_id: AppInstanceId,
    pub plan: DeploymentPlan,
    pub state: DeploymentState,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One (service, substrate) unit of work inside a deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRecord {
    pub action_type: String,
    pub logical_ref: String,
    pub substrate_alias: Option<String>,
    pub substrate_did: String,
}

/// No `Pending` variant (M05A A5c §24): nothing in this tree ever wrote one
/// -- `apply_plan` (`sdk/src/deploy.rs`) writes `InProgress` directly for
/// each action since nothing enqueues work ahead of applying it, and the
/// resident loop's own filtered plan (D-A5c-2) is applied the same way.
/// Removed rather than left as a variant no writer will ever populate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionState {
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for ActionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::InProgress => "IN_PROGRESS",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for ActionState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "IN_PROGRESS" => Ok(Self::InProgress),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            _ => Err(anyhow!("Unknown action state: {}", s)),
        }
    }
}

/// `conn: Arc<Mutex<Connection>>`, not a bare `Connection` (M05A A5b,
/// D-A5-8): `rusqlite::Connection` is `Send` but not `Sync`, so a reference
/// held across an `.await` point makes the enclosing future non-`Send` --
/// harmless while every caller blocks on it, fatal for a `tokio::spawn`ed
/// supervisor loop. `std::sync::Mutex`, not a tokio one: every statement is
/// a short synchronous call, and a tokio mutex would make `open_in_memory`
/// async for no benefit. The `Arc` also lets `SupervisorStore` back its
/// desired-state table, this journal, and `AlertStore` with one connection.
#[derive(Debug, Clone)]
pub struct DeploymentJournal {
    conn: Arc<Mutex<Connection>>,
}

// Lock-poisoning from a panicking holder is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here, matching `StaticInventory`'s.
#[allow(clippy::expect_used)]
impl DeploymentJournal {
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

    /// Wraps an already-open connection -- `SupervisorStore`'s constructor,
    /// so it can back this journal, `AlertStore`, and its own desired-state
    /// table with one shared `Arc<Mutex<Connection>>` rather than three
    /// separate database files.
    pub fn from_connection(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        Self::init_schema(&conn.lock().expect("journal connection lock poisoned"))?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Unconditional, not gated on `PRAGMA user_version`: pre-release, schema
        // changes are made in place with no version ladder, and `IF NOT EXISTS`
        // is already idempotent.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deployments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                instance_id TEXT NOT NULL,
                plan_json TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_deployments_instance_id ON deployments(instance_id);
             CREATE TABLE IF NOT EXISTS deployment_actions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                deployment_id INTEGER NOT NULL,
                action_type TEXT NOT NULL,
                logical_ref TEXT NOT NULL,
                substrate_alias TEXT,
                substrate_did TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY(deployment_id) REFERENCES deployments(id)
             );
             CREATE INDEX IF NOT EXISTS idx_deployment_actions_dep_id
                ON deployment_actions(deployment_id);",
        )?;

        Ok(())
    }

    pub fn append(&self, plan: &DeploymentPlan, state: DeploymentState) -> Result<i64> {
        let now = Utc::now().timestamp();
        let plan_json = plan.to_json()?;
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        conn.execute(
            "INSERT INTO deployments (instance_id, plan_json, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![plan.app_instance_id.as_str(), plan_json, state.to_string(), now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_state(&self, id: i64, state: DeploymentState) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        conn.execute(
            "UPDATE deployments SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![state.to_string(), now, id],
        )?;
        Ok(())
    }

    pub fn append_action(
        &self,
        deployment_id: i64,
        action_type: &str,
        logical_ref: &str,
        substrate_alias: Option<&str>,
        substrate_did: &str,
        state: ActionState,
    ) -> Result<i64> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        conn.execute(
            "INSERT INTO deployment_actions (deployment_id, action_type, logical_ref, \
             substrate_alias, substrate_did, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                deployment_id,
                action_type,
                logical_ref,
                substrate_alias,
                substrate_did,
                state.to_string(),
                now,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_action_state(&self, action_id: i64, state: ActionState) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        conn.execute(
            "UPDATE deployment_actions SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![state.to_string(), now, action_id],
        )?;
        Ok(())
    }

    pub fn get_completed_actions(&self, deployment_id: i64) -> Result<Vec<ActionRecord>> {
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT action_type, logical_ref, substrate_alias, substrate_did
             FROM deployment_actions
             WHERE deployment_id = ?1 AND state = 'COMPLETED'
             ORDER BY id ASC",
        )?;

        let mut rows = stmt.query(params![deployment_id])?;
        let mut completed = Vec::new();
        while let Some(row) = rows.next()? {
            completed.push(ActionRecord {
                action_type: row.get(0)?,
                logical_ref: row.get(1)?,
                substrate_alias: row.get(2)?,
                substrate_did: row.get(3)?,
            });
        }
        Ok(completed)
    }

    /// Every completed action for an app instance, across **all** its
    /// deployment records, oldest first.
    ///
    /// The per-record query above answers "what does this run still owe?"; this
    /// one answers "where has this service actually landed, ever?", which is a
    /// different question and spans records: a plan edit starts a new record,
    /// so the run that placed a service can be two records back. The
    /// placement-change refusal is its only caller.
    pub fn get_completed_actions_for_instance(
        &self,
        instance_id: &AppInstanceId,
    ) -> Result<Vec<ActionRecord>> {
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT a.action_type, a.logical_ref, a.substrate_alias, a.substrate_did
               FROM deployment_actions a
               JOIN deployments d ON d.id = a.deployment_id
              WHERE d.instance_id = ?1 AND a.state = 'COMPLETED'
              ORDER BY a.id ASC",
        )?;

        let mut rows = stmt.query(params![instance_id.as_str()])?;
        let mut completed = Vec::new();
        while let Some(row) = rows.next()? {
            completed.push(ActionRecord {
                action_type: row.get(0)?,
                logical_ref: row.get(1)?,
                substrate_alias: row.get(2)?,
                substrate_did: row.get(3)?,
            });
        }
        Ok(completed)
    }

    pub fn get_latest(&self, instance_id: &AppInstanceId) -> Result<Option<DeploymentRecord>> {
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, instance_id, plan_json, state, created_at, updated_at 
             FROM deployments 
             WHERE instance_id = ?1 
             ORDER BY id DESC LIMIT 1",
        )?;

        let mut rows = stmt.query(params![instance_id.as_str()])?;

        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let inst_str: String = row.get(1)?;
            let plan_json: String = row.get(2)?;
            let state_str: String = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            let updated_at: i64 = row.get(5)?;

            let state: DeploymentState = state_str.parse()?;
            let plan: DeploymentPlan = DeploymentPlan::from_json(&plan_json)?;

            Ok(Some(DeploymentRecord {
                id,
                instance_id: AppInstanceId::new(inst_str),
                plan,
                state,
                created_at,
                updated_at,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_last_state(
        &self,
        instance_id: &AppInstanceId,
        target_state: DeploymentState,
    ) -> Result<Option<DeploymentRecord>> {
        let conn = self.conn.lock().expect("journal connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, instance_id, plan_json, state, created_at, updated_at 
             FROM deployments 
             WHERE instance_id = ?1 AND state = ?2
             ORDER BY id DESC LIMIT 1",
        )?;

        let mut rows = stmt.query(params![instance_id.as_str(), target_state.to_string()])?;

        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let inst_str: String = row.get(1)?;
            let plan_json: String = row.get(2)?;
            let state_str: String = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            let updated_at: i64 = row.get(5)?;

            let state: DeploymentState = state_str.parse()?;
            let plan: DeploymentPlan = DeploymentPlan::from_json(&plan_json)?;

            Ok(Some(DeploymentRecord {
                id,
                instance_id: AppInstanceId::new(inst_str),
                plan,
                state,
                created_at,
                updated_at,
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;

    use super::*;
    use crate::models::{
        AppBlueprintId, LogicalServiceName, LogicalServiceRef, PlannedService, ServiceConfig,
        ServiceId, ServiceType, TopologyMode,
    };

    fn dummy_plan(instance_name: &str) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new(instance_name),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: Version::parse("1.0.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:z123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new(instance_name),
                    service_name: LogicalServiceName::new("echo"),
                },
                substrate: None,
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "test.wasm".to_string(),
                    hash: None,
                    interfaces: vec![],
                    env: BTreeMap::new(),
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: Default::default(),
                    fdae: None,
                    health_check: None,
                },
                resolved_dependencies: BTreeMap::new(),
                topology_mode: TopologyMode::Singleton,
                member_index: 0,
            }],
        }
    }

    #[test]
    fn test_journal_append_and_update() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_plan("inst-1");

        // Append
        let id = journal.append(&plan, DeploymentState::Planned).unwrap();

        // Retrieve
        let record = journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
        assert_eq!(record.id, id);
        assert_eq!(record.state, DeploymentState::Planned);
        assert_eq!(record.plan, plan);

        // Update state
        journal.update_state(id, DeploymentState::Applying).unwrap();

        // Retrieve again
        let record2 = journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
        assert_eq!(record2.state, DeploymentState::Applying);
    }

    #[test]
    fn an_action_row_round_trips_its_alias_and_substrate_did() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_plan("inst-1");
        let deployment_id = journal.append(&plan, DeploymentState::Applying).unwrap();

        journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/echo",
                Some("edge-1"),
                "did:key:zNodeA",
                ActionState::Completed,
            )
            .unwrap();

        let completed = journal.get_completed_actions(deployment_id).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].action_type, "ADD");
        assert_eq!(completed[0].logical_ref, "inst-1/echo");
        assert_eq!(completed[0].substrate_alias.as_deref(), Some("edge-1"));
        assert_eq!(completed[0].substrate_did, "did:key:zNodeA");
    }

    #[test]
    fn an_action_row_with_no_alias_round_trips_a_null_alias() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_plan("inst-1");
        let deployment_id = journal.append(&plan, DeploymentState::Applying).unwrap();

        journal
            .append_action(
                deployment_id,
                "ADD",
                "inst-1/echo",
                None,
                "did:key:zNodeA",
                ActionState::Completed,
            )
            .unwrap();

        let completed = journal.get_completed_actions(deployment_id).unwrap();
        assert_eq!(completed[0].substrate_alias, None);
    }

    #[test]
    fn the_degraded_state_round_trips_through_display_and_from_str() {
        assert_eq!(DeploymentState::Degraded.to_string(), "DEGRADED");
        assert_eq!("DEGRADED".parse::<DeploymentState>().unwrap(), DeploymentState::Degraded);
    }

    #[test]
    fn completed_actions_for_an_instance_span_every_record_oldest_first() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_plan("inst-1");

        // First deployment record: the service lands on edge-1.
        let first_id = journal.append(&plan, DeploymentState::Active).unwrap();
        journal
            .append_action(
                first_id,
                "ADD",
                "inst-1/echo",
                Some("edge-1"),
                "did:key:zNodeA",
                ActionState::Completed,
            )
            .unwrap();

        // A plan edit starts a second record: the service is now on edge-2.
        let second_id = journal.append(&plan, DeploymentState::Active).unwrap();
        journal
            .append_action(
                second_id,
                "ADD",
                "inst-1/echo",
                Some("edge-2"),
                "did:key:zNodeB",
                ActionState::Completed,
            )
            .unwrap();

        let completed =
            journal.get_completed_actions_for_instance(&AppInstanceId::new("inst-1")).unwrap();
        assert_eq!(completed.len(), 2);
        // Oldest first, so `rfind` on the caller's side finds the current home.
        assert_eq!(completed[0].substrate_did, "did:key:zNodeA");
        assert_eq!(completed[1].substrate_did, "did:key:zNodeB");
    }

    #[test]
    fn completed_actions_for_an_instance_ignores_another_instances_rows() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan_1 = dummy_plan("inst-1");
        let plan_2 = dummy_plan("inst-2");

        let id_1 = journal.append(&plan_1, DeploymentState::Active).unwrap();
        journal
            .append_action(
                id_1,
                "ADD",
                "inst-1/echo",
                Some("edge-1"),
                "did:key:zNodeA",
                ActionState::Completed,
            )
            .unwrap();

        let id_2 = journal.append(&plan_2, DeploymentState::Active).unwrap();
        journal
            .append_action(
                id_2,
                "ADD",
                "inst-2/echo",
                Some("edge-2"),
                "did:key:zNodeB",
                ActionState::Completed,
            )
            .unwrap();

        let completed =
            journal.get_completed_actions_for_instance(&AppInstanceId::new("inst-1")).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].logical_ref, "inst-1/echo");
    }

    /// D-A5-8: both stores must be `Send + Sync` as a whole, not merely
    /// individually lockable -- a supervisor holds them as fields of a type
    /// that itself must be `Send + Sync` (`NativeService`'s bound).
    #[test]
    fn journal_and_alert_store_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DeploymentJournal>();
        assert_send_sync::<crate::AlertStore>();
    }

    /// The whole point of the `Arc<Mutex<Connection>>` conversion: a journal
    /// held across an `.await` inside a spawned task must produce a `Send`
    /// future, or `tokio::spawn` cannot take it. Mirrors
    /// `apply_plan_returns_a_send_future` in `crates/sdk/src/deploy.rs`.
    #[test]
    fn a_journal_reference_held_across_an_await_is_send() {
        fn assert_send<T: Send>(_: T) {}
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let fut = async {
            let _ = &journal;
            tokio::task::yield_now().await;
            journal.get_latest(&AppInstanceId::new("inst-1")).unwrap()
        };
        assert_send(fut);
    }
}

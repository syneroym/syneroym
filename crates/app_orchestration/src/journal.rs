use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::models::{AppInstanceId, DeploymentPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeploymentState {
    Planned,
    Applying,
    Active,
    RollingBack,
    RolledBack,
}

impl std::fmt::Display for DeploymentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Planned => "PLANNED",
            Self::Applying => "APPLYING",
            Self::Active => "ACTIVE",
            Self::RollingBack => "ROLLING_BACK",
            Self::RolledBack => "ROLLED_BACK",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for DeploymentState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PLANNED" => Ok(Self::Planned),
            "APPLYING" => Ok(Self::Applying),
            "ACTIVE" => Ok(Self::Active),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionState {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl std::fmt::Display for ActionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "PENDING",
            Self::InProgress => "IN_PROGRESS",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for ActionState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PENDING" => Ok(Self::Pending),
            "IN_PROGRESS" => Ok(Self::InProgress),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            _ => Err(anyhow!("Unknown action state: {}", s)),
        }
    }
}

#[derive(Debug)]
pub struct DeploymentJournal {
    conn: Connection,
}

impl DeploymentJournal {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {}", db_name));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if version < 1 {
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS deployments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    instance_id TEXT NOT NULL,
                    plan_json TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_deployments_instance_id ON \
                 deployments(instance_id);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }

        if version < 2 {
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS deployment_actions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    deployment_id INTEGER NOT NULL,
                    action_type TEXT NOT NULL,
                    logical_ref TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    FOREIGN KEY(deployment_id) REFERENCES deployments(id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_deployment_actions_dep_id ON \
                 deployment_actions(deployment_id);
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }

        Ok(())
    }

    pub fn append(&self, plan: &DeploymentPlan, state: DeploymentState) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        let plan_json = plan.to_json()?;
        self.conn.execute(
            "INSERT INTO deployments (instance_id, plan_json, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![plan.app_instance_id.as_str(), plan_json, state.to_string(), now, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_state(&self, id: i64, state: DeploymentState) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
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
        state: ActionState,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO deployment_actions (deployment_id, action_type, logical_ref, state, \
             created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![deployment_id, action_type, logical_ref, state.to_string(), now, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_action_state(&self, action_id: i64, state: ActionState) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "UPDATE deployment_actions SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![state.to_string(), now, action_id],
        )?;
        Ok(())
    }

    pub fn get_completed_actions(&self, deployment_id: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT action_type, logical_ref 
             FROM deployment_actions 
             WHERE deployment_id = ?1 AND state = 'COMPLETED'",
        )?;

        let mut rows = stmt.query(params![deployment_id])?;
        let mut completed = Vec::new();
        while let Some(row) = rows.next()? {
            let action_type: String = row.get(0)?;
            let logical_ref: String = row.get(1)?;
            completed.push((action_type, logical_ref));
        }
        Ok(completed)
    }

    pub fn get_latest(&self, instance_id: &AppInstanceId) -> Result<Option<DeploymentRecord>> {
        let mut stmt = self.conn.prepare(
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
        let mut stmt = self.conn.prepare(
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
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "test.wasm".to_string(),
                    hash: None,
                    interfaces: vec![],
                    env: Default::default(),
                    args: vec![],
                    custom_config: None,
                    quota: None,
                },
                resolved_dependencies: vec![],
                topology_mode: TopologyMode::Singleton,
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
}

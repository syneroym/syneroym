use anyhow::Result;

use crate::{
    journal::{DeploymentJournal, DeploymentState},
    models::{AppInstanceId, DeploymentPlan, LogicalServiceRef, PlannedService},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    /// Add a new service
    Add(Box<PlannedService>),
    /// Remove an existing service
    Remove(LogicalServiceRef),
    /// Update an existing service (e.g., config or source changed)
    Update { old: Box<PlannedService>, new: Box<PlannedService> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilePlan {
    pub instance_id: AppInstanceId,
    pub target_plan: DeploymentPlan,
    pub actions: Vec<ReconcileAction>,
}

#[derive(Debug)]
pub struct Reconciler<'a> {
    journal: &'a DeploymentJournal,
}

impl<'a> Reconciler<'a> {
    pub fn new(journal: &'a DeploymentJournal) -> Self {
        Self { journal }
    }

    /// Computes the diff between the currently ACTIVE deployment and the
    /// provided desired plan.
    pub fn compute_diff(&self, desired: &DeploymentPlan) -> Result<ReconcilePlan> {
        let last_active =
            self.journal.get_last_state(&desired.app_instance_id, DeploymentState::Active)?;

        let actions = Self::diff_plans(last_active.map(|r| r.plan).as_ref(), desired);

        Ok(ReconcilePlan {
            instance_id: desired.app_instance_id.clone(),
            target_plan: desired.clone(),
            actions,
        })
    }

    pub fn recover_applying(&self, instance_id: &AppInstanceId) -> Result<Option<ReconcilePlan>> {
        let latest = self.journal.get_latest(instance_id)?;
        if let Some(record) = latest
            && record.state == DeploymentState::Applying
        {
            let last_active = self.journal.get_last_state(instance_id, DeploymentState::Active)?;

            let mut actions = Self::diff_plans(last_active.map(|r| r.plan).as_ref(), &record.plan);

            // Filter out actions that were already completed in this APPLYING deployment.
            let completed = self.journal.get_completed_actions(record.id)?;
            actions.retain(|a| {
                let (a_type, l_ref) = match a {
                    ReconcileAction::Add(svc) => ("ADD", &svc.logical_ref),
                    ReconcileAction::Remove(r) => ("REMOVE", r),
                    ReconcileAction::Update { new, .. } => ("UPDATE", &new.logical_ref),
                };
                let l_str = l_ref.to_string();
                !completed.iter().any(|(ct, cr)| ct == a_type && cr == &l_str)
            });

            return Ok(Some(ReconcilePlan {
                instance_id: instance_id.clone(),
                target_plan: record.plan,
                actions,
            }));
        }
        Ok(None)
    }

    fn diff_plans(
        active: Option<&DeploymentPlan>,
        desired: &DeploymentPlan,
    ) -> Vec<ReconcileAction> {
        let mut actions = Vec::new();

        if let Some(active_plan) = active {
            let mut active_map = std::collections::HashMap::new();
            for s in &active_plan.services {
                active_map.insert(s.logical_ref.clone(), s);
            }

            for desired_svc in &desired.services {
                if let Some(active_svc) = active_map.remove(&desired_svc.logical_ref) {
                    if active_svc != desired_svc {
                        actions.push(ReconcileAction::Update {
                            old: Box::new(active_svc.clone()),
                            new: Box::new(desired_svc.clone()),
                        });
                    }
                } else {
                    actions.push(ReconcileAction::Add(Box::new(desired_svc.clone())));
                }
            }

            // Removals must happen in reverse topological order.
            // `active_plan.services` is already topologically sorted, so iterating in
            // reverse ensures dependents are removed before their dependencies.
            for active_svc in active_plan.services.iter().rev() {
                if active_map.contains_key(&active_svc.logical_ref) {
                    actions.push(ReconcileAction::Remove(active_svc.logical_ref.clone()));
                }
            }
        } else {
            // No active plan, add all desired in topological order
            for desired_svc in &desired.services {
                actions.push(ReconcileAction::Add(Box::new(desired_svc.clone())));
            }
        }

        actions
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::*;
    use crate::models::{
        AppBlueprintId, LogicalServiceName, ServiceConfig, ServiceId, ServiceType, TopologyMode,
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
                    env: std::collections::BTreeMap::new(),
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema_path: None,
                    rotation_policy: Default::default(),
                },
                resolved_dependencies: vec![],
                topology_mode: TopologyMode::Singleton,
            }],
        }
    }

    #[test]
    fn test_reconcile_diff() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let active_plan = dummy_plan("inst-1");

        let _ = journal.append(&active_plan, DeploymentState::Active).unwrap();

        let mut desired_plan = active_plan.clone();
        desired_plan.services[0].config.source = "test2.wasm".to_string(); // Change something to trigger an update

        let reconciler = Reconciler::new(&journal);
        let plan = reconciler.compute_diff(&desired_plan).unwrap();

        assert_eq!(plan.actions.len(), 1);
        match &plan.actions[0] {
            ReconcileAction::Update { old, new } => {
                assert_eq!(old.config.source, "test.wasm");
                assert_eq!(new.config.source, "test2.wasm");
            }
            _ => panic!("Expected Update action"),
        }
    }

    #[test]
    fn test_recover_applying() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_plan("inst-1");

        // Mock an APPLYING state
        let _ = journal.append(&plan, DeploymentState::Applying).unwrap();

        let reconciler = Reconciler::new(&journal);
        let recovery_plan = reconciler
            .recover_applying(&AppInstanceId::new("inst-1"))
            .unwrap()
            .expect("Should recover");

        assert_eq!(recovery_plan.actions.len(), 1);
        match &recovery_plan.actions[0] {
            ReconcileAction::Add(svc) => {
                assert_eq!(svc.logical_ref.service_name.as_str(), "echo");
            }
            _ => panic!("Expected Add action since there was no active plan before"),
        }
    }

    #[test]
    fn test_diff_topological_sorting() {
        let mut active_plan = dummy_plan("inst-1");
        let mut desired_plan = active_plan.clone();

        // Add an extra service to active plan to test Remove order
        // Service B depends on A, so B should be removed before A
        let mut svc_a = active_plan.services[0].clone();
        svc_a.logical_ref.service_name = LogicalServiceName::new("A");
        let mut svc_b = active_plan.services[0].clone();
        svc_b.logical_ref.service_name = LogicalServiceName::new("B");
        active_plan.services = vec![svc_a.clone(), svc_b.clone()];

        // Add extra services to desired plan to test Add order
        // Service Y depends on X, so X should be added before Y
        let mut svc_x = desired_plan.services[0].clone();
        svc_x.logical_ref.service_name = LogicalServiceName::new("X");
        let mut svc_y = desired_plan.services[0].clone();
        svc_y.logical_ref.service_name = LogicalServiceName::new("Y");
        desired_plan.services = vec![svc_x.clone(), svc_y.clone()];

        let actions = Reconciler::diff_plans(Some(&active_plan), &desired_plan);

        // We expect Add(X), Add(Y), Remove(B), Remove(A)
        assert_eq!(actions.len(), 4);

        match &actions[0] {
            ReconcileAction::Add(svc) => assert_eq!(svc.logical_ref.service_name.as_str(), "X"),
            _ => panic!("Expected Add(X)"),
        }
        match &actions[1] {
            ReconcileAction::Add(svc) => assert_eq!(svc.logical_ref.service_name.as_str(), "Y"),
            _ => panic!("Expected Add(Y)"),
        }
        match &actions[2] {
            ReconcileAction::Remove(r) => assert_eq!(r.service_name.as_str(), "B"),
            _ => panic!("Expected Remove(B)"),
        }
        match &actions[3] {
            ReconcileAction::Remove(r) => assert_eq!(r.service_name.as_str(), "A"),
            _ => panic!("Expected Remove(A)"),
        }
    }
}

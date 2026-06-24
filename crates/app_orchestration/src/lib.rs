#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Domain models, catalog, compiler, and logical resolver for local app model
//! and lifecycle.

pub mod catalog;
pub mod compiler;
pub mod journal;
pub mod models;
pub mod reconcile;
pub mod resolver;

pub use catalog::{LocalFilesystemCatalog, ManifestCatalog};
pub use compiler::{CompiledDeployment, compile};
pub use journal::{DeploymentJournal, DeploymentRecord, DeploymentState};
pub use models::{
    AppBlueprintId, AppDependencySpec, AppInstanceId, DependencyName, DeploymentPlan,
    InterfaceName, LogicalServiceName, LogicalServiceRef, ParseError, PlannedService,
    ServiceConfig, ServiceId, ServiceSpec, ServiceType, SynAppManifest, TopologyMode,
};
pub use reconcile::{ReconcileAction, ReconcilePlan, Reconciler};
pub use resolver::{
    AllMembers, AppRegistry, LogicalResolver, ResolvedTopology, ShardingStrategy, StaticInventory,
    TopologyEntry, TopologyEpoch, rendezvous_select,
};

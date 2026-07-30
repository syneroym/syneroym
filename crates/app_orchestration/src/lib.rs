#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Domain models, catalog, compiler, and logical resolver for local app model
//! and lifecycle.

pub mod catalog;
pub mod compiler;
pub mod journal;
pub mod models;
pub mod reconcile;
pub mod resolver;
pub mod substrate_inventory;

pub use catalog::{LocalFilesystemCatalog, ManifestCatalog};
pub use compiler::{CompiledDeployment, compile};
pub use journal::{
    ActionRecord, ActionState, DeploymentJournal, DeploymentRecord, DeploymentState,
};
pub use models::{
    AppBlueprintId, AppDependencySpec, AppInstanceId, DependencyName, DeploymentPlan,
    InterfaceName, LogicalServiceName, LogicalServiceRef, ParseError, PlacementSelector,
    PlannedService, ServiceConfig, ServiceId, ServiceSpec, ServiceType, SubstrateAlias,
    SynAppManifest, TopologyMode,
};
pub use reconcile::{ReconcileAction, ReconcilePlan, Reconciler};
pub use resolver::{
    AllMembers, AppRegistry, DEFAULT_BINDING_CACHE_TTL_MS, LogicalResolver, ResolvedTopology,
    ShardingStrategy, StaticInventory, TopologyEntry, TopologyEpoch, empty_resolver,
    rendezvous_select,
};
pub use substrate_inventory::{
    SubstrateEntry, SubstrateInventory, check_placement, placement_demand,
};

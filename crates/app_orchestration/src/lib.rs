#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Domain models, catalog, compiler, and logical resolver for local app model
//! and lifecycle.

pub mod alerts;
pub mod catalog;
pub mod compiler;
pub mod journal;
pub mod models;
pub mod reconcile;
pub mod resolver;
pub mod saga;
pub mod schedule;
pub mod substrate_inventory;

pub use alerts::{AlertKind, AlertRecord, AlertStore};
pub use catalog::{LocalFilesystemCatalog, ManifestCatalog};
pub use compiler::{CompiledDeployment, compile};
pub use journal::{
    ActionRecord, ActionState, DeploymentJournal, DeploymentRecord, DeploymentState,
};
pub use models::{
    AppBlueprintId, AppDependencySpec, AppInstanceId, DependencyName, DeploymentPlan, HealthCheck,
    HttpProbe, InterfaceName, LogicalServiceName, LogicalServiceRef, ParseError, PlacementSelector,
    PlannedService, RpcProbe, ScheduleSpec, ServiceConfig, ServiceId, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest, TcpProbe, TopologyMode,
};
pub use reconcile::{ReconcileAction, ReconcilePlan, Reconciler};
pub use resolver::{
    AllMembers, AppRegistry, BindingWriteOutcome, DEFAULT_BINDING_CACHE_TTL_MS, LogicalResolver,
    ResolvedTopology, ShardingStrategy, StaticInventory, TopologyEntry, TopologyEpoch,
    classify_binding_write, empty_resolver, rendezvous_select,
};
pub use saga::{SAGA_UNDO_PREFIX, compensated_operation, saga_undo_name};
pub use schedule::{
    DEFAULT_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES, has_occurrence_in,
};
pub use substrate_inventory::{
    SubstrateEntry, SubstrateInventory, check_placement, placement_demand,
};

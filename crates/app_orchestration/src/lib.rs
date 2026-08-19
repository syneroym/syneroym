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
pub mod topology_document;

pub use alerts::{AlertKind, AlertRecord, AlertStore};
pub use catalog::{LocalFilesystemCatalog, ManifestCatalog};
pub use compiler::{CompiledDeployment, compile, validate_plan_visibility};
pub use journal::{
    ActionRecord, ActionState, DeploymentJournal, DeploymentRecord, DeploymentState,
};
pub use models::{
    AppBlueprintId, AppDependencySpec, AppDid, AppInstanceId, DependencyName, DeploymentPlan,
    HealthCheck, HttpProbe, InterfaceName, LogicalServiceName, LogicalServiceRef, ParseError,
    PlacementSelector, PlannedService, RpcProbe, ScheduleSpec, ServiceConfig, ServiceId,
    ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest, TcpProbe, TopologyMode,
    TopologyVisibility, Visibility,
};
pub use reconcile::{ReconcileAction, ReconcilePlan, Reconciler};
pub use resolver::{
    AllMembers, AppRegistry, AppScope, BindingWriteOutcome, DEFAULT_BINDING_CACHE_TTL_MS,
    LogicalResolver, ResolvedTopology, ShardingStrategy, StaticInventory, TopologyEntry,
    TopologyEpoch, TopologyKey, classify_binding_write, empty_resolver, is_retryable_resolve_error,
    rendezvous_select,
};
pub use saga::{SAGA_UNDO_PREFIX, compensated_operation, saga_undo_name};
pub use schedule::{
    DEFAULT_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES, has_occurrence_in,
};
pub use substrate_inventory::{
    SubstrateEntry, SubstrateInventory, check_placement, placement_demand,
};
pub use topology_document::{
    SignedTopologyDocument, TopologyDocument, TopologyFetcher, register_verified,
    topology_fingerprint,
};

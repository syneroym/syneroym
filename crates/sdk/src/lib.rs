#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym App SDK
//!
//! High-level APIs and traits to help third-party developers build apps
//! that integrate seamlessly with the Syneroym runtime and services.

pub mod client;
pub mod deploy;
pub mod health;
pub mod mapper;
pub mod topology;
pub mod types;

pub use client::{DEFAULT_CONNECT_TIMEOUT, MessageStream, SyneroymClient, TransportConnection};
pub use deploy::{
    ApplyReport, ApplyRequest, DeployTarget, ServiceFailure, SubstrateActor, apply_plan,
    resolve_targets,
};
pub use syneroym_rpc::{DeadLetterInfo, QueuedCallInfo, SagaInfo, SagaState};
pub use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, AssetBundle, BindingWrite, ContainerManifest, ContainerPortMapping,
    ContainerVolumeMapping, DependencyBinding, DeployManifest, DeploymentPlan, HealthCheck,
    HttpProbe, InstanceIdentity, NetworkEndpoint, PlannedService, RpcProbe, ServiceConfig,
    ServiceType, TcpManifest, TcpProbe, TopologyMode, Visibility, WasmManifest,
};
pub use topology::{RegistryTopologyFetcher, fetch_and_register};
pub use types::*;

#[cfg(test)]
mod tests;

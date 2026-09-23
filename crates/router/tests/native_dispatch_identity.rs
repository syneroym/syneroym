#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Native-dispatch identity threading tests.

#[path = "native_dispatch_identity/helpers.rs"]
mod helpers;

#[path = "native_dispatch_identity/access_control.rs"]
mod access_control;
#[path = "native_dispatch_identity/cross_service_fetch.rs"]
mod cross_service_fetch;
#[path = "native_dispatch_identity/fdae_enforcement.rs"]
mod fdae_enforcement;
#[path = "native_dispatch_identity/queries.rs"]
mod queries;
#[path = "native_dispatch_identity/resolve_relation.rs"]
mod resolve_relation;

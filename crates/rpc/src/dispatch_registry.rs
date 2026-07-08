//! Shared registry of native (non-WASM) services, keyed by `service_id`.
//!
//! A plain type alias, not a wrapper struct: `DashMap`'s own
//! `.insert()`/`.remove()`/`.get()` API is already exactly what every call
//! site needs (see `crates/router/src/route_handler.rs` and
//! `crates/control_plane/src/service.rs`), so a forwarding wrapper would add
//! a type with no behavior of its own. The only real fix this represents is
//! `Arc`-wrapping: `DashMap` itself is not cheaply `Clone`, so it must be
//! shared via `Arc` to hand the same registry to both `RouteHandler` (which
//! constructs it) and `ControlPlaneService` (which registers/deregisters
//! per-deployment native services into it).

use std::sync::Arc;

use dashmap::DashMap;

use crate::NativeService;

pub type NativeDispatchRegistry = Arc<DashMap<String, Arc<dyn NativeService>>>;

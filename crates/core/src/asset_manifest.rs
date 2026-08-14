//! The shared static-asset manifest and per-service registry (M06A A1):
//! request path -> blob hash, served straight from blob storage without
//! instantiating the component.
//!
//! Lives in `core`, next to `http_routes`, for the identical reason:
//! `syneroym-control-plane` builds these at deploy/undeploy
//! (`crates/control_plane/src/assets.rs`) and `syneroym-router` reads them
//! per HTTP request (`crates/router/src/route_handler/http.rs`).

use std::{collections::BTreeMap, sync::Arc};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// One static asset. `hash` is also the `ETag` value and the blob's key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetEntry {
    /// Blob content hash (hex sha256 of the plaintext).
    pub hash: String,
    pub len: u64,
    /// Resolved at unpack time from the path's extension.
    pub content_type: String,
}

/// A deployed service's static asset table: request path (normalised to a
/// single leading `/`) -> entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetManifest {
    pub entries: BTreeMap<String, AssetEntry>,
}

/// One service's asset bundle, as cached in the [`AssetRegistry`].
#[derive(Debug, Clone)]
pub struct ServiceAssets {
    pub manifest: Arc<AssetManifest>,
    /// The declared bundle's `visibility == public` (M06A D-A1-1). `internal`
    /// and `private` both collapse to `false` here -- A1 implements no
    /// middle tier (D-A1-8), so the router only ever needs "serve" or
    /// "don't", never the third value. The full three-way `visibility` enum
    /// lives on the deploy-facing types (`syneroym_app_orchestration::
    /// models::Visibility`, and the WIT `visibility` enum) where the
    /// distinction matters for future consumers; `core` has no dependency on
    /// `app_orchestration` and gains none purely for this cache.
    pub public: bool,
    /// Retained so the next `deploy()` generation and `undeploy()` can
    /// delete the manifest blob itself, not only the entries it lists
    /// (M06A D-A1-9).
    pub manifest_hash: String,
}

/// Shared, keyed-by-`service_id` static asset table. **A cache, not
/// persistence** (M06A D-A1-2): created empty each boot, populated by
/// `deploy()`, cleared by `undeploy()`. Same shape and lifecycle as
/// `HttpRouteRegistry`.
pub type AssetRegistry = Arc<DashMap<String, ServiceAssets>>;

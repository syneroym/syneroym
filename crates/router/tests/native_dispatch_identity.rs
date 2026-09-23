#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Native-dispatch identity threading -- one of the most
//! important behaviours in the router. Drives
//! `RouteHandler::dispatch_json_rpc_once`/`handle_http_stream` directly
//! (the wire handshake itself is `crates/router/src/handshake.rs`'s own
//! test responsibility) to prove:
//!
//! 1. An anonymous (`caller: None`) request to every native-capability
//!    interface (`data-layer`/`vault`/`app-config`/`blob-store`/ `messaging`)
//!    is rejected *before* the native service is invoked.
//! 2. The HTTP bridge rejects the same anonymous request, mapped to 401.
//! 3. An authenticated caller's identity reaches `SynSvcNativeService`'s
//!    `dispatch_data_layer` and becomes the stored `creator_id` -- not the
//!    service being called.

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

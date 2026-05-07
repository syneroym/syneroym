//! Common constants used across the Syneroym project.

/// Separator used in the routing preamble between the interface name and service ID.
/// Example: `json-rpc://syneroym-test:greeter/greet@0.1.0|did:key:z...`
pub const PREAMBLE_SEPARATOR: &str = "|";

/// Separator used in the HTTP Host header for routing.
/// Example: `interface--service_id.localhost`
pub const HOST_ROUTING_SEPARATOR: &str = "--";

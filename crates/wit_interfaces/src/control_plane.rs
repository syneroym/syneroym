//! Control plane WIT bindings
//!
//! Contains generated bindings and marshalling traits allowing guest
//! applications to invoke substrate control plane features.

wit_bindgen::generate!({
    world: "control-plane-service",
    path: "wit/control-plane/control-plane.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});

use exports::syneroym::control_plane::orchestrator::Visibility;

impl Visibility {
    /// The wire/display form (ADR-0018), matching how `roymctl` parses and
    /// prints it -- one definition instead of a hand-written match repeated
    /// at every call site that needs to print this value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Private => "private",
        }
    }
}

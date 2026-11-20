//! The fixed registry of Roym extensions that ship with this build.
//!
//! An extension adds typed `details` to a commitment and work steps to a deal.
//! It never changes the deal's states. Each extension is one JSON manifest
//! under `extensions/`. The Hub imports the same files, so the node and the
//! Hub always agree on which extensions are known.

use std::sync::LazyLock;

use serde::Deserialize;

use crate::{
    deal::{ExtensionRef, MAX_EXTENSION_ID_LEN, ResourceKind},
    listing,
};

pub mod check;
pub mod ids;
mod load;
pub mod schema;

#[cfg(test)]
mod tests;

pub use check::{
    ExtensionCheck, PartialCheck, check_commitment_extension, check_details_shape,
    check_extension_kind, check_partial_details,
};
pub use load::ManifestError;
pub use schema::{Check, Field, FieldType, Schema, SchemaError};

pub const SCHEDULING_EXTENSION: &str = "scheduling";
pub const SCHEDULING_SLOT_SCHEMA: &str = "scheduling/slot@1";

/// The manifests that ship with this build. The Hub's registry module lists
/// the same files.
pub const MANIFESTS: &[&str] =
    &[include_str!("../extensions/scheduling.json"), include_str!("../extensions/one-shot.json")];

/// Whether `id` has the shape of an extension id. A directory uses this to
/// check a search filter without loading the registry.
pub fn is_extension_id(id: &str) -> bool {
    listing::valid_token(id, MAX_EXTENSION_ID_LEN)
}

/// Longest step type, including the extension id prefix.
pub const MAX_STEP_TYPE_LEN: usize = 64;

/// What an extension is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestKind {
    /// Adds `details` and steps to a deal.
    Workflow,
    /// Only reads. It names what it reads and has no other part.
    Insight,
}

/// A call that the deal core makes on an extension that holds a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Hook {
    Reserve,
    Confirm,
    Release,
}

/// Which party of a commitment may record a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepBy {
    Giver,
    Receiver,
    Either,
}

/// One kind of work step that an extension declares.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepType {
    /// `<extension id>/<name>`.
    #[serde(rename = "type")]
    pub step_type: String,
    pub by: StepBy,
    pub fields: Vec<Field>,
}

impl StepType {
    /// Checks a step's `data` against the step's field list.
    pub fn check(&self, data: &serde_json::Value) -> Result<(), SchemaError> {
        schema::check_fields(&self.fields, data)
    }
}

/// One extension's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: u32,
    pub kind: ManifestKind,
    /// The commitment kinds that may name this extension.
    #[serde(default)]
    pub resource_kinds: Vec<ResourceKind>,
    /// All of `reserve`, `confirm` and `release`, or none.
    #[serde(default)]
    pub hooks: Vec<Hook>,
    #[serde(default)]
    pub details: Vec<Schema>,
    #[serde(default)]
    pub steps: Vec<StepType>,
    /// What an `insight` extension may read. Empty for a `workflow`.
    #[serde(default)]
    pub reads: Vec<String>,
}

impl Manifest {
    pub fn allows(&self, kind: ResourceKind) -> bool {
        self.resource_kinds.contains(&kind)
    }

    pub fn has_reserve_hook(&self) -> bool {
        self.hooks.contains(&Hook::Reserve)
    }
}

/// A set of loaded manifests.
#[derive(Debug, Clone)]
pub struct Registry {
    manifests: Vec<Manifest>,
}

impl Registry {
    /// Loads manifests and checks every rule of the format.
    pub fn parse(sources: &[&str]) -> Result<Self, ManifestError> {
        let mut manifests: Vec<Manifest> = Vec::with_capacity(sources.len());
        for source in sources {
            let manifest: Manifest =
                serde_json::from_str(source).map_err(|e| ManifestError::Format(e.to_string()))?;
            load::check_manifest(&manifest)?;
            if manifests.iter().any(|m| m.id == manifest.id && m.version == manifest.version) {
                return Err(ManifestError::rule(&manifest.id, "this version is listed twice"));
            }
            manifests.push(manifest);
        }
        load::check_message_ids(&manifests)?;
        Ok(Self { manifests })
    }

    pub fn manifests(&self) -> &[Manifest] {
        &self.manifests
    }

    /// Any manifest with this id and version, `insight` ones too.
    pub fn get(&self, ext: &ExtensionRef) -> Option<&Manifest> {
        self.manifests.iter().find(|m| m.id == ext.id && m.version == ext.version)
    }

    /// A `workflow` manifest only. The deal and listing paths use this, so an
    /// `insight` id is unknown to them.
    pub fn workflow(&self, ext: &ExtensionRef) -> Option<&Manifest> {
        self.get(ext).filter(|m| m.kind == ManifestKind::Workflow)
    }

    pub fn details_schema(&self, ext: &ExtensionRef, schema: &str) -> Option<&Schema> {
        self.workflow(ext)?.details.iter().find(|s| s.schema == schema)
    }

    pub fn step_type(&self, ext: &ExtensionRef, step_type: &str) -> Option<&StepType> {
        self.workflow(ext)?.steps.iter().find(|s| s.step_type == step_type)
    }
}

/// The manifests that ship with this build, parsed once.
#[expect(
    clippy::expect_used,
    reason = "the shipped manifests are compiled in; every_shipped_manifest_loads proves they \
              parse"
)]
pub fn registry() -> &'static Registry {
    static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
        Registry::parse(MANIFESTS).expect("the manifests that ship with this build must load")
    });
    &REGISTRY
}

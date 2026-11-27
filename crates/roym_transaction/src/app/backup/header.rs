//! Bundle header verification.

use syneroym_roym_core::{
    backup::{self, Bundle},
    envelope::Response,
};

use crate::app::SCHEMA_VERSION;

pub(super) fn validate_bundle_header(
    bundle: &Bundle,
    owner: &str,
    now: u64,
) -> Result<(), Response> {
    if let Err(e) = backup::check_signed_bundle(bundle, owner, now) {
        return Err(Response::invalid_params(e.to_string()));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Err(Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            )));
        }
    }
    Ok(())
}

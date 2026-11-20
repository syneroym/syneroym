//! The one set of checks that decides whether a commitment's extension and
//! `details` are valid. Every place that reads a commitment calls these:
//! the deal terms, the selective-disclosure shape and the presentation
//! checker.

use super::{Check, Manifest, Registry};
use crate::{
    deal::{
        Commitment, CommitmentKind, DealError, Details, ExtensionRef, MAX_DETAILS_EXTRA,
        MAX_DETAILS_EXTRA_LABEL_LEN, MAX_DETAILS_EXTRA_VALUE_LEN, MAX_EXTENSION_ID_LEN,
        ResourceKind,
    },
    listing,
    transaction::TimeWindow,
};

/// What the registry says about one commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionCheck {
    /// No extension on this commitment.
    None,
    /// Known extension; shape, kind, schema and named checks all pass.
    Checked,
    /// The extension is not a workflow extension in this node's registry.
    /// Only the shape was checked.
    NotChecked,
}

/// What the registry says about the visible part of a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialCheck {
    /// Everything this extension checks was visible and passed.
    Checked,
    /// A part was hidden: `details_matches_extension` is not checked.
    Partial,
    /// Unknown extension: `details_matches_extension` is not checked.
    NotChecked,
}

/// The resource kind and commitment kind must be a pair that exists.
fn check_kind_pair(resource_kind: ResourceKind, kind: CommitmentKind) -> Result<(), DealError> {
    match (resource_kind, kind) {
        (ResourceKind::ServiceInSlot, CommitmentKind::Fulfilment)
        | (ResourceKind::GoodsOrService, CommitmentKind::Fulfilment)
        | (ResourceKind::Currency, CommitmentKind::Currency) => Ok(()),
        (ResourceKind::ServiceInSlot, CommitmentKind::Currency)
        | (ResourceKind::GoodsOrService, CommitmentKind::Currency)
        | (ResourceKind::Currency, CommitmentKind::Fulfilment) => {
            Err(DealError::KindMismatch(resource_kind, kind))
        }
    }
}

/// A commitment with no extension. A slot needs an extension that holds it.
fn check_no_extension(resource_kind: ResourceKind) -> Result<ExtensionCheck, DealError> {
    if resource_kind == ResourceKind::ServiceInSlot {
        return Err(DealError::CommitmentKindNotSupported(
            "service-in-slot requires an extension with a reserve hook".to_string(),
        ));
    }
    Ok(ExtensionCheck::None)
}

fn check_extension_ref(ext: &ExtensionRef) -> Result<(), DealError> {
    if !listing::valid_token(&ext.id, MAX_EXTENSION_ID_LEN) {
        return Err(DealError::ExtensionShape(format!("invalid extension id: {}", ext.id)));
    }
    if ext.version == 0 {
        return Err(DealError::ExtensionShape("extension version must be at least 1".to_string()));
    }
    Ok(())
}

/// The form `<extension id>/<name>@<version>`.
fn check_schema_name(ext: &ExtensionRef, schema: &str) -> Result<(), DealError> {
    let form_ok = schema
        .strip_prefix(ext.id.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|rest| rest.rsplit_once('@'))
        .is_some_and(|(name, version)| !name.is_empty() && version.parse::<u32>().is_ok());
    if form_ok {
        Ok(())
    } else {
        Err(DealError::ExtensionShape(format!(
            "schema '{schema}' must be '{}/<name>@<version>'",
            ext.id
        )))
    }
}

fn check_extra(details: &Details) -> Result<(), DealError> {
    if details.extra.len() > MAX_DETAILS_EXTRA {
        return Err(DealError::ExtensionShape(format!(
            "details.extra length {} exceeds {}",
            details.extra.len(),
            MAX_DETAILS_EXTRA
        )));
    }
    for extra in &details.extra {
        if extra.label.len() > MAX_DETAILS_EXTRA_LABEL_LEN {
            return Err(DealError::ExtensionShape("extra label too long".to_string()));
        }
        if extra.value.len() > MAX_DETAILS_EXTRA_VALUE_LEN {
            return Err(DealError::ExtensionShape("extra value too long".to_string()));
        }
    }
    Ok(())
}

/// The shape rules that hold for every extension, known or not: `details`
/// exists exactly when the extension does, they name the same extension, the
/// extension and schema names have their form, and `extra` is within its
/// limits.
pub fn check_details_shape(c: &Commitment) -> Result<(), DealError> {
    if c.extension.is_some() != c.details.is_some() {
        return Err(DealError::DetailsMismatch);
    }
    let (Some(ext), Some(details)) = (&c.extension, &c.details) else {
        return Ok(());
    };
    check_ext_and_details(ext, details)
}

fn check_ext_and_details(ext: &ExtensionRef, details: &Details) -> Result<(), DealError> {
    if details.extension != ext.id {
        return Err(DealError::DetailsMismatch);
    }
    check_extension_ref(ext)?;
    check_schema_name(ext, &details.schema)?;
    check_extra(details)
}

/// The manifest that governs a commitment of this kind, if the extension is
/// known. A known extension must list the resource kind, and a slot needs a
/// reserve hook.
fn manifest_for_kind<'a>(
    registry: &'a Registry,
    resource_kind: ResourceKind,
    ext: &ExtensionRef,
) -> Result<Option<&'a Manifest>, DealError> {
    if resource_kind == ResourceKind::Currency {
        return Err(DealError::CommitmentKindNotSupported(format!(
            "currency does not support extension '{}'",
            ext.id
        )));
    }
    let Some(manifest) = registry.workflow(ext) else {
        return Ok(None);
    };
    if !manifest.allows(resource_kind) {
        return Err(DealError::CommitmentKindNotSupported(format!(
            "extension '{}' does not support {resource_kind:?}",
            ext.id
        )));
    }
    if resource_kind == ResourceKind::ServiceInSlot && !manifest.has_reserve_hook() {
        return Err(DealError::CommitmentKindNotSupported(format!(
            "extension '{}' has no reserve hook",
            ext.id
        )));
    }
    Ok(Some(manifest))
}

/// A full commitment: its `details` and `window` are plain.
pub fn check_commitment_extension(
    registry: &Registry,
    c: &Commitment,
) -> Result<ExtensionCheck, DealError> {
    check_kind_pair(c.resource_kind, c.kind)?;
    let Some(ext) = &c.extension else {
        let verdict = check_no_extension(c.resource_kind)?;
        if c.details.is_some() {
            return Err(DealError::DetailsMismatch);
        }
        return Ok(verdict);
    };
    check_details_shape(c)?;
    let Some(manifest) = manifest_for_kind(registry, c.resource_kind, ext)? else {
        return Ok(ExtensionCheck::NotChecked);
    };
    let Some(details) = &c.details else {
        return Err(DealError::DetailsMismatch);
    };
    let schema = manifest
        .details
        .iter()
        .find(|s| s.schema == details.schema)
        .ok_or(DealError::DetailsMismatch)?;
    schema.check(&details.data).map_err(|e| DealError::DetailsInvalid(e.0))?;
    for check in &schema.checks {
        run_named_check(*check, c.window.as_ref(), details)?;
    }
    Ok(ExtensionCheck::Checked)
}

/// Only the kind rules, for `SignedTerms::validate_visible`, which sees a
/// commitment whose `details` and `window` are still hidden digests.
pub fn check_extension_kind(
    registry: &Registry,
    resource_kind: ResourceKind,
    kind: CommitmentKind,
    ext: Option<&ExtensionRef>,
) -> Result<ExtensionCheck, DealError> {
    check_kind_pair(resource_kind, kind)?;
    let Some(ext) = ext else {
        return check_no_extension(resource_kind);
    };
    check_extension_ref(ext)?;
    Ok(match manifest_for_kind(registry, resource_kind, ext)? {
        Some(_) => ExtensionCheck::Checked,
        None => ExtensionCheck::NotChecked,
    })
}

/// For the presentation checker. `details` is `None` when hidden; `window` is
/// `None` when hidden and `Some(None)` when the commitment has none.
///
/// The commitment already passed `check_extension_kind`, so only the rules
/// that need `details` run here.
pub fn check_partial_details(
    registry: &Registry,
    ext: &ExtensionRef,
    details: Option<&Details>,
    window: Option<Option<&TimeWindow>>,
) -> Result<PartialCheck, DealError> {
    let Some(details) = details else {
        return Ok(PartialCheck::Partial);
    };
    check_ext_and_details(ext, details)?;
    let Some(manifest) = registry.workflow(ext) else {
        return Ok(PartialCheck::NotChecked);
    };
    let schema = manifest
        .details
        .iter()
        .find(|s| s.schema == details.schema)
        .ok_or(DealError::DetailsMismatch)?;
    schema.check(&details.data).map_err(|e| DealError::DetailsInvalid(e.0))?;
    let mut result = PartialCheck::Checked;
    for check in &schema.checks {
        match window {
            Some(Some(w)) => run_named_check(*check, Some(w), details)?,
            Some(None) | None => result = PartialCheck::Partial,
        }
    }
    Ok(result)
}

fn run_named_check(
    check: Check,
    window: Option<&TimeWindow>,
    details: &Details,
) -> Result<(), DealError> {
    match check {
        Check::SlotMatchesWindow => slot_matches_window(window, details),
    }
}

/// `slot_id` is the id derived from `listing_id` and the commitment's window.
fn slot_matches_window(window: Option<&TimeWindow>, details: &Details) -> Result<(), DealError> {
    let window = window.ok_or_else(|| {
        DealError::SlotDetails("scheduling commitment requires window".to_string())
    })?;
    let text = |key: &str| {
        details
            .data
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| DealError::SlotDetails(format!("{key} must be a string")))
    };
    let listing_id = text("listing_id")?;
    let slot_id = text("slot_id")?;
    if window.latest_secs <= window.earliest_secs {
        return Err(DealError::SlotDetails(
            "window latest must be greater than earliest".to_string(),
        ));
    }
    let expected = listing::derive_slot_id(listing_id, window.earliest_secs, window.latest_secs)
        .map_err(|e| DealError::SlotDetails(e.to_string()))?;
    if slot_id != expected {
        return Err(DealError::SlotDetails(format!(
            "slot_id {slot_id} does not match derived {expected}"
        )));
    }
    Ok(())
}

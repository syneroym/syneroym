//! Shared helpers for member master identities (ADR-0020 §1-§5): naming,
//! resolve-or-mint, and certifying a substrate-derived instance key.
//!
//! A member master is an ordinary `roymctl`-managed identity file; the only
//! thing this module adds is a deterministic name so `svc deploy --master`
//! and `app deploy --mint-masters` resolve the same file for the same
//! member without an operator having to track it by hand.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use syneroym_app_orchestration::models::{DeploymentPlan, LogicalServiceRef, ServiceId};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::SyneroymClient;

/// The attended posture's default certificate lifetime for a plan-deploy-time
/// certification -- `identity certify-instance` is the dedicated renewal
/// command for a longer- or shorter-lived one.
const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

/// `<dir>/identities/member-<app_instance_id>-<service_name>-<index>.key`'s
/// stem. `index` is the member ordinal (`0` for a `Singleton`).
///
/// `LogicalServiceName` already forbids `/`, but `AppInstanceId` has no
/// validator -- checked here so a name containing a path separator is
/// rejected before it can escape the identities directory.
pub fn member_master_name(logical_ref: &LogicalServiceRef, index: u32) -> Result<String> {
    let app_instance_id = logical_ref.app_instance_id.to_string();
    if app_instance_id.contains('/') {
        anyhow::bail!(
            "app instance id '{app_instance_id}' must not contain '/': it becomes part of a \
             member master's file name"
        );
    }
    Ok(format!("member-{app_instance_id}-{}-{index}", logical_ref.service_name))
}

fn member_master_path(dir: &Path, name: &str) -> PathBuf {
    dir.join("identities").join(format!("{name}.key"))
}

/// Loads the named member master, or fails naming the missing identity.
/// Minting is never silent (ADR-0020 §4): only
/// [`resolve_or_mint_member_master`] mints, and only when the operator asked
/// for it.
pub fn resolve_member_master(dir: &Path, name: &str) -> Result<Identity> {
    let path = member_master_path(dir, name);
    if !path.exists() {
        anyhow::bail!(
            "member master '{name}' not found at {}. Run `roymctl identity create --name {name}` \
             first, or pass --mint-masters to mint it",
            path.display()
        );
    }
    Identity::load_from_path(&path)
        .with_context(|| format!("failed to load member master at {}", path.display()))
}

/// Loads the named member master, minting and persisting a new one if it
/// doesn't exist. Prints ADR-0020 §4's backup warning at mint time -- losing
/// this key is unrecoverable and orphans the member's stored data, so the
/// operator must see that at the moment it is created, not discover it
/// later.
pub fn resolve_or_mint_member_master(dir: &Path, name: &str) -> Result<Identity> {
    let path = member_master_path(dir, name);
    if path.exists() {
        return Identity::load_from_path(&path)
            .with_context(|| format!("failed to load member master at {}", path.display()));
    }

    let identities_dir = dir.join("identities");
    std::fs::create_dir_all(&identities_dir)?;
    let identity = Identity::generate()?;
    identity.save_to_path(&path)?;
    let did = substrate::derive_did_key(&identity.public_key());
    eprintln!(
        "Minted new member master '{name}' ({did}) at {}.\nBack this key up -- losing it is \
         unrecoverable and orphans every row this member has written.",
        path.display()
    );
    Ok(identity)
}

/// Queries the substrate for the instance key it would derive for
/// `service_id` (the member master's own DID) under the connecting caller's
/// identity, and issues a `service-instance`-scoped certificate over it from
/// `master`. This is the full round trip: one read-only RPC, one local
/// signature, no install call -- installation happens at `deploy`.
pub async fn certify_instance(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let master_did = substrate::derive_did_key(&master.public_key());
    if master_did != service_id {
        anyhow::bail!(
            "master identity resolves to {master_did}, which does not match service_id \
             {service_id} -- a certificate for this pair would be rejected at install time"
        );
    }

    let identity = client
        .instance_identity(service_id)
        .await
        .context("failed to query the substrate for its derived instance identity")?;
    let pubkey_bytes = hex::decode(&identity.pubkey_hex)
        .context("substrate returned an invalid hex-encoded instance pubkey")?;
    let pubkey_array: [u8; 32] = pubkey_bytes.try_into().map_err(|_| {
        anyhow::anyhow!("substrate returned an instance pubkey of the wrong length")
    })?;
    let pubkey = ed25519_dalek::VerifyingKey::from_bytes(&pubkey_array)
        .context("substrate returned an invalid ed25519 instance pubkey")?;

    DelegationCertificate::issue(
        master,
        pubkey,
        expires_hours * 3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
}

/// The `app deploy --mint-masters` path: resolves or mints one member master
/// per service in the plan (index `0` -- nothing in today's manifest format
/// can express more than one member per `PlannedService`), then returns a
/// **new** plan with every `service_id` and `resolved_dependencies` entry
/// substituted from the compiler's fabricated id to the resolved master DID,
/// plus a certified instance certificate per resolved master.
///
/// Takes the already-compiled, already-journaled plan by reference and
/// returns a copy rather than mutating it in place: the deployment journal
/// records the plan *before* this substitution runs, so it never holds
/// master-DID-bearing data, only the compiler's fabricated ids.
pub async fn substitute_and_certify_members(
    client: &SyneroymClient,
    dir: &Path,
    plan: &DeploymentPlan,
) -> Result<(DeploymentPlan, BTreeMap<ServiceId, String>)> {
    let mut substitution: BTreeMap<ServiceId, ServiceId> = BTreeMap::new();
    let mut masters: BTreeMap<ServiceId, Identity> = BTreeMap::new();
    for svc in &plan.services {
        let name = member_master_name(&svc.logical_ref, 0)?;
        let master = resolve_or_mint_member_master(dir, &name)?;
        let master_did = ServiceId::try_new(substrate::derive_did_key(&master.public_key()))
            .context("substrate-derived master DID failed ServiceId validation")?;
        substitution.insert(svc.service_id.clone(), master_did.clone());
        masters.insert(master_did, master);
    }

    let mut new_plan = plan.clone();
    for svc in &mut new_plan.services {
        let old_id = svc.service_id.clone();
        svc.service_id = substitution
            .get(&old_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no resolved member master for service {old_id}"))?;
        svc.resolved_dependencies = svc
            .resolved_dependencies
            .iter()
            .map(|dep| {
                substitution.get(dep).cloned().ok_or_else(|| {
                    anyhow::anyhow!("no resolved member master for dependency {dep}")
                })
            })
            .collect::<Result<Vec<_>>>()?;
    }

    let mut instance_certs = BTreeMap::new();
    for (master_did, master) in &masters {
        let cert = certify_instance(
            client,
            master,
            master_did.as_str(),
            DEFAULT_INSTANCE_CERT_EXPIRES_HOURS,
        )
        .await?;
        instance_certs.insert(master_did.clone(), cert.to_json()?);
    }

    Ok((new_plan, instance_certs))
}

#[cfg(test)]
mod tests {
    use syneroym_app_orchestration::models::{AppInstanceId, LogicalServiceName};

    use super::*;

    fn logical_ref(app_instance_id: &str, service_name: &str) -> LogicalServiceRef {
        LogicalServiceRef {
            app_instance_id: AppInstanceId::new(app_instance_id),
            service_name: LogicalServiceName::new(service_name),
        }
    }

    #[test]
    fn member_master_name_is_deterministic() {
        let name = member_master_name(&logical_ref("inst-1", "backend"), 0).unwrap();
        assert_eq!(name, "member-inst-1-backend-0");
    }

    #[test]
    fn member_master_name_carries_the_index() {
        let name = member_master_name(&logical_ref("inst-1", "backend"), 2).unwrap();
        assert_eq!(name, "member-inst-1-backend-2");
    }

    #[test]
    fn member_master_name_rejects_an_app_instance_id_containing_a_path_separator() {
        let err = member_master_name(&logical_ref("inst/../escape", "backend"), 0).unwrap_err();
        assert!(err.to_string().contains('/'));
    }

    #[test]
    fn resolve_member_master_names_the_missing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_member_master(dir.path(), "member-inst-1-backend-0").unwrap_err();
        assert!(err.to_string().contains("member-inst-1-backend-0"));
    }

    #[test]
    fn resolve_or_mint_member_master_mints_once_and_reuses_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = resolve_or_mint_member_master(dir.path(), "member-inst-1-backend-0").unwrap();
        let second = resolve_or_mint_member_master(dir.path(), "member-inst-1-backend-0").unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }
}

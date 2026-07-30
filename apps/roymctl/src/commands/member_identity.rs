//! Shared helpers for member master identities (ADR-0020 §1-§5): naming and
//! resolve-or-mint.
//!
//! A member master is an ordinary `roymctl`-managed identity file; the only
//! thing this module adds is a deterministic name so `svc deploy --master`
//! and `app deploy --mint-masters` resolve the same file for the same
//! member without an operator having to track it by hand.
//!
//! Per-substrate certificate and endpoint-record minting lives in
//! `syneroym_sdk::deploy` (M05A Slice A3 §6), not here: it needs the
//! per-alias client map A3's multi-substrate placement introduces, and
//! living in the SDK lets the two-substrate e2e harness exercise it directly.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use syneroym_app_orchestration::models::{
    DeploymentPlan, LogicalServiceRef, ServiceId, SubstrateAlias,
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::{SyneroymClient, deploy};

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

/// Publishes or refreshes `master`'s anchor at `registry_url` (D-A1-7), or
/// warns naming the consequence when none was supplied: a certificate just
/// minted is unusable on the wire until its master's anchor is resolvable --
/// `HandshakeVerifier::verify_preamble` resolves it on every delegated
/// connection and fails closed when it is missing. Builds the client with
/// the DHT enabled, matching `identity publish-anchor`: unlike an endpoint
/// record, an anchor is self-signed by the master and has a valid DHT home.
pub async fn refresh_anchor_or_warn(registry_url: Option<&str>, master: &Identity) -> Result<()> {
    let master_did = substrate::derive_did_key(&master.public_key());
    match registry_url {
        Some(url) => {
            RegistryClient::new(true, Some(url.to_string()))
                .refresh_master_anchor(master)
                .await
                .with_context(|| format!("failed to publish master anchor for {master_did}"))?;
        }
        None => {
            eprintln!(
                "No --registry-url given, so no master anchor was published for \
                 {master_did}.\nAny connection presenting this certificate will be rejected until \
                 one is (`roymctl identity publish-anchor`). In a multi-substrate deploy, the \
                 anchor must live in the registry EVERY substrate in the inventory resolves \
                 through."
            );
        }
    }
    Ok(())
}

/// The `app deploy --mint-masters` path: resolves or mints one member master
/// per service in the plan (index `0` -- nothing in today's manifest format
/// can express more than one member per `PlannedService`), then returns a
/// **new** plan with every `service_id` and `resolved_dependencies` entry
/// substituted from the compiler's fabricated id to the resolved master DID,
/// plus a certified instance certificate and a master-signed endpoint record
/// per resolved master, minted against **each member's own placed
/// substrate** (M05A Slice A3 §0.1) -- a certificate minted through one
/// substrate's client is rejected at deploy by any other, since the
/// derivation includes both the node and the calling DID.
///
/// Takes the already-compiled, already-journaled plan by reference and
/// returns a copy rather than mutating it in place: the deployment journal
/// records the plan *before* this substitution runs, so it never holds
/// master-DID-bearing data, only the compiler's fabricated ids.
pub async fn substitute_and_certify_members(
    dir: &Path,
    plan: &DeploymentPlan,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    // `None` when every service is placed by alias.
    fallback: Option<&Arc<SyneroymClient>>,
    registry_url: Option<&str>,
) -> Result<(DeploymentPlan, BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>)> {
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
            .map(|(name, members)| {
                let substituted = members
                    .iter()
                    .map(|dep| {
                        substitution.get(dep).cloned().ok_or_else(|| {
                            anyhow::anyhow!("no resolved member master for dependency {dep}")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok((name.clone(), substituted))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
    }

    let (instance_certs, registry_certs) = deploy::certify_placed_members(
        &new_plan,
        &masters,
        clients,
        fallback,
        DEFAULT_INSTANCE_CERT_EXPIRES_HOURS,
    )
    .await?;

    // Once per master (D-A1-7): `masters` is already deduplicated by master
    // DID, unlike `plan.services`, which can name the same master more than
    // once for a redundant member.
    for master in masters.values() {
        refresh_anchor_or_warn(registry_url, master).await?;
    }

    Ok((new_plan, instance_certs, registry_certs))
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

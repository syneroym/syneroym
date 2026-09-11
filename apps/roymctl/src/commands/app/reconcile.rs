//! Reconcile, forget, and resolve subcommands for SynApps.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use syneroym_app_orchestration::{
    ActionState, AppInstanceId, DeploymentJournal, DeploymentState, LocalFilesystemCatalog,
    Reconciler, SynAppManifest, TopologyFetcher, compile,
    models::{AppDid, LogicalServiceName, LogicalServiceRef, MemberRef},
};
use syneroym_identity::Identity;
use syneroym_sdk::{RegistryTopologyFetcher, deploy};
use syneroym_ucan::CapabilityToken;

pub(super) async fn handle_reconcile(
    instance_id: String,
    manifest_path: Option<PathBuf>,
    journal_path: PathBuf,
) -> anyhow::Result<()> {
    let instance_id = AppInstanceId::try_new(instance_id.clone())?;

    let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
    let db_name = journal_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;

    let journal = DeploymentJournal::open(parent_dir, db_name)?;
    let reconciler = Reconciler::new(&journal);

    if let Some(recovery_plan) = reconciler.recover_applying(&instance_id)? {
        println!("Found APPLYING or DEGRADED state for {instance_id}. Computed recovery plan:");
        for action in recovery_plan.actions {
            println!(" - {action:?}");
        }
    } else {
        let active = journal.get_last_state(&instance_id, DeploymentState::Active)?;
        if active.is_some() {
            if let Some(manifest_path) = manifest_path {
                println!(
                    "App {instance_id} is ACTIVE. Diffing active deployment against manifest at \
                     {manifest_path:?}"
                );

                let toml_str = fs::read_to_string(&manifest_path)?;
                let manifest = SynAppManifest::from_toml(&toml_str)?;
                let catalog = LocalFilesystemCatalog::new(
                    manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                );

                let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;

                if let Some(target_plan) = compiled.plans.last() {
                    let diff = reconciler.compute_diff(target_plan)?;
                    println!("Computed diff:");
                    if diff.actions.is_empty() {
                        println!(" (No changes)");
                    } else {
                        for action in diff.actions {
                            println!(" - {action:?}");
                        }
                    }
                } else {
                    println!("Compiled deployment contains no plans.");
                }
            } else {
                println!(
                    "App {instance_id} is ACTIVE. Provide a --manifest-path to compute a diff."
                );
            }
        } else {
            println!("No ACTIVE, APPLYING or DEGRADED state found for {instance_id}");
        }
    }

    Ok(())
}

pub(super) fn handle_forget(
    instance_id: String,
    service: String,
    journal_path: PathBuf,
) -> anyhow::Result<()> {
    let instance_id = AppInstanceId::try_new(instance_id.clone())?;
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new(service.as_str()),
    };
    // The journal keys every action row on a `MemberRef`, not a
    // bare `LogicalServiceRef`. `--service` names
    // only the logical service, with no way to name one member of a
    // scaled one -- forgets member 0, the only member an unscaled
    // deploy ever has. Forgetting one member of a `replicas > 1`
    // service is not supported by this command yet (deferred-
    // backlog: "roymctl app forget has no per-member verb"), so a
    // scaled service is refused below rather than silently acting
    // on member 0 alone while its siblings stay tracked.
    let l_ref = (MemberRef { logical_ref: logical_ref.clone(), index: 0 }).to_string();

    let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
    let db_name = journal_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;
    let journal = DeploymentJournal::open(parent_dir, db_name)?;

    let landed = journal.get_completed_actions_for_instance(&instance_id)?;

    let member_prefix = format!("{logical_ref}#");
    let landed_member_indices: BTreeSet<u32> = landed
        .iter()
        .filter(|r| r.logical_ref.starts_with(&member_prefix))
        .filter_map(|r| r.logical_ref[member_prefix.len()..].parse::<u32>().ok())
        .filter(|idx| {
            deploy::current_placement(&landed, &format!("{member_prefix}{idx}")).is_some()
        })
        .collect();
    if landed_member_indices.len() > 1 {
        anyhow::bail!(
            "'{service}' in {instance_id} has {} landed members ({landed_member_indices:?}); `app \
             forget` does not yet support naming one member of a scaled service. Forgetting only \
             member 0 would leave the others tracked as if this command had never run.",
            landed_member_indices.len()
        );
    }

    match landed.iter().rev().find(|r| r.logical_ref == l_ref) {
        None => anyhow::bail!(
            "no completed deploy is recorded for '{service}' in {instance_id}; nothing to forget"
        ),
        Some(prev) if prev.action_type == "REMOVE" => {
            println!("'{service}' in {instance_id} is already forgotten.");
        }
        Some(prev) => {
            let record_id = journal
                .get_latest(&instance_id)?
                .ok_or_else(|| anyhow::anyhow!("no deployment record found for {instance_id}"))?
                .id;
            journal.append_action(
                record_id,
                "REMOVE",
                &l_ref,
                prev.substrate_alias.as_deref(),
                &prev.substrate_did,
                ActionState::Completed,
            )?;
            println!(
                "Forgot '{service}' (was on {}) for {instance_id}. This only clears roymctl's \
                 placement bookkeeping -- if the service instance is still running there, \
                 undeploy it first with `svc remove --svc-id <id>` against that substrate.",
                prev.substrate_alias.as_deref().unwrap_or(prev.substrate_did.as_str())
            );
        }
    }

    Ok(())
}

pub(super) async fn handle_resolve(
    app_did: String,
    service_name: String,
    api_url: &str,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let app_did = AppDid::try_new(app_did.clone())?;
    let service_name = LogicalServiceName::try_new(service_name.clone())?;

    let mut fetcher = RegistryTopologyFetcher::new(api_url.to_string());
    if let Some(name) = run_as {
        let path = dir.join("identities").join(format!("{name}.key"));
        let id = Identity::load_from_path(&path)
            .with_context(|| format!("no local identity '{name}' at {}", path.display()))?;
        fetcher = fetcher.with_identity(&id);
    }
    if let Some(path) = ucan_path {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
        let token: CapabilityToken = serde_json::from_str(&raw)
            .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))?;
        fetcher = fetcher.with_ucan(token);
    }

    let signed =
        fetcher.fetch(&app_did, &service_name).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    signed
        .verify(&app_did)
        .context("the fetched document did not verify against the resolved app DID")?;

    println!("app: {app_did}  service: {service_name}");
    println!("mode: {:?}  epoch: {}", signed.document.mode, signed.document.epoch.0);
    println!("members:");
    for member in &signed.document.members {
        println!("  {member}");
    }

    Ok(())
}

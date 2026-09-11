//! SynApp manifest deployment handler and verification.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use semver::Version;
use syneroym_app_orchestration::{
    ActionRecord, AppInstanceId, DeploymentJournal, DeploymentPlan, DeploymentState,
    LocalFilesystemCatalog, SynAppManifest, compile,
    models::{
        AppBlueprintId, LogicalServiceName, PlannedService, ServiceConfig, ServiceSpec,
        ServiceType, SubstrateAlias,
    },
    substrate_inventory::{SubstrateInventory, check_placement, placement_demand},
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_sdk::{
    SyneroymClient,
    deploy::{self, ApplyRequest, DeployTarget},
};

use super::{PREFLIGHT_TIMEOUT, resolve_credentials};
use crate::commands::member_identity;

/// Retries `f` until it succeeds or `budget` elapses, returning the last
/// error. Used only for the post-apply registry probe, which tolerates a
/// registry write still propagating rather than reporting a topology fault
/// for a slow one.
async fn retry_for<T, E, F, Fut>(budget: Duration, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if start.elapsed() >= budget {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
}

/// The post-apply fallback, kept now that the `status` call answers the
/// registry-namespace question at preflight whenever the credential can
/// read node facts -- the deploy loop above bails before any artifact work
/// when it can see a split namespace outright. This
/// probe stays as the propagation check and the fallback for a credential
/// that cannot: a substrate publishes through its **own** configured
/// registry, which nothing on the wire reports to a caller who cannot read
/// node facts, so a split-registry fleet can still deploy cleanly and then
/// fail to resolve. This is a heuristic over the URLs `roymctl` was given --
/// it proves "the registry at this URL cannot resolve member M", never
/// "substrate X cannot" -- and it only warns, never fails the deploy: the
/// services genuinely landed, and marking the deployment `Degraded` would
/// send the next run redeploying them for nothing.
async fn probe_registry_reachability(
    placed: &[(&PlannedService, &DeployTarget)],
    urls: &BTreeSet<String>,
) {
    for url in urls {
        // DHT enabled: the warning below names "enable the DHT"
        // as one of the two ways to satisfy the shared-namespace precondition,
        // so the probe must actually be able to see it -- with it disabled, a
        // fleet that took that advice got a false warning on every deploy.
        let reg = RegistryClient::new(true, Some(url.clone()));
        for (svc, target) in placed {
            match retry_for(Duration::from_secs(3), || reg.lookup(svc.service_id.as_str(), false))
                .await
            {
                Err(_) => eprintln!(
                    "warning: the registry at {url} cannot resolve member '{}' ({}). If that is \
                     the registry a substrate hosting one of this app's services uses, its \
                     dependency calls to '{}' will fail at call time. Every substrate in the \
                     inventory must publish into and resolve through one registry namespace (or \
                     enable the DHT).",
                    svc.logical_ref, svc.service_id, svc.logical_ref
                ),
                Ok(rec) if rec.info.substrate_id != target.substrate_did => eprintln!(
                    "warning: the registry at {url} resolves member '{}' to substrate {}, not {} \
                     -- a stale record from an earlier placement is still winning there.",
                    svc.logical_ref, rec.info.substrate_id, target.substrate_did
                ),
                Ok(_) => {}
            }
            if retry_for(Duration::from_secs(3), || {
                reg.resolve_master_anchor(svc.service_id.as_str(), None)
            })
            .await
            .is_err()
            {
                eprintln!(
                    "warning: the registry at {url} holds no master anchor for '{}' ({}). A \
                     substrate resolving through it will reject this member's calls at the \
                     handshake. Publish it with `roymctl identity publish-anchor --master <name> \
                     --registry-url {url}`.",
                    svc.logical_ref, svc.service_id
                );
            }
        }
    }
}

/// The placement-change refusal: a redeploy
/// that would move a service to a different substrate than it already
/// landed on is a hard error, not a silent relocation -- the old instance
/// would keep running and keep republishing its endpoint record, exactly
/// the two-publisher state a compare-and-swap admission rule bounds but
/// does not stop.
///
/// Sourced from `COMPLETED` action rows across **every** record for the
/// instance, not the last `ACTIVE` plan: a partially-failed deploy leaves
/// the record `Degraded` (or leaves no `ACTIVE` record at all, on a first
/// deploy), while the services that did land are still running -- an
/// `ACTIVE`-only source misses exactly that sequence.
///
/// Uses `deploy::current_placement` -- the **most recent** row for the
/// logical ref, of either action type, not the most recent `ADD`: `app
/// forget` (below) closes this refusal by appending a `REMOVE` row, and a
/// lookup scoped to `ADD` alone would keep finding the stale `ADD`
/// underneath it forever. A most-recent `REMOVE` means the operator has
/// already cleared the bookkeeping for this service, so any placement --
/// the same substrate or a different one -- is fine. Shared with
/// `apply_plan`'s resume-skip so the two cannot read the journal two
/// different ways again.
///
/// Pulled out of `handle` so it is unit-testable against a plain journal,
/// with no live substrate needed.
pub(crate) fn check_no_placement_change(
    dir: &Path,
    placed: &[(&PlannedService, &DeployTarget)],
    landed: &[ActionRecord],
) -> anyhow::Result<()> {
    for (svc, target) in placed {
        let l_ref = svc.member_ref().to_string();
        if let Some(prev) = deploy::current_placement(landed, &l_ref)
            && prev.substrate_did != target.substrate_did
        {
            let real_id = member_identity::deployed_service_id(dir, svc)?;
            anyhow::bail!(
                "service '{}' is already deployed on substrate {} and this run would place it on \
                 {}. A3 does not relocate -- the old instance would keep running and keep \
                 republishing its endpoint record.\nUndeploy it first:\n  roymctl --substrate {} \
                 --as <that substrate's identity> svc remove --svc-id {real_id}\nthen clear the \
                 placement record so this refusal does not fire again:\n  roymctl app forget {} \
                 --service {}\nthen redeploy.",
                svc.logical_ref,
                prev.substrate_alias.as_deref().unwrap_or(prev.substrate_did.as_str()),
                target
                    .alias
                    .as_ref()
                    .map(SubstrateAlias::as_str)
                    .unwrap_or(target.substrate_did.as_str()),
                prev.substrate_did,
                svc.logical_ref.app_instance_id,
                svc.logical_ref.service_name,
            );
        }
    }
    Ok(())
}

/// A manifest declaring dependencies should have no unmastered deploy path
/// at all. A warning at deploy time and a runtime failure at the guest's
/// first `dependency(...)` call was the worst split available: the operator
/// sees the consequence far from the cause. A manifest with no dependencies
/// is unaffected -- an unmastered deploy of an independent service stays
/// valid, which `svc deploy` and every dependency-free manifest rely on.
///
/// Pulled out of `handle` so it is unit-testable with no live substrate,
/// the same reason `check_no_placement_change` is its own function.
pub(crate) fn refuse_unmastered_dependencies(
    plan: &DeploymentPlan,
    mint_masters: bool,
) -> anyhow::Result<()> {
    if mint_masters || !plan.services.iter().any(|s| !s.resolved_dependencies.is_empty()) {
        return Ok(());
    }
    let names: BTreeSet<&str> = plan
        .services
        .iter()
        .flat_map(|s| s.resolved_dependencies.keys())
        .map(LogicalServiceName::as_str)
        .collect();
    let names = names.into_iter().collect::<Vec<_>>().join(", ");
    anyhow::bail!(
        "this manifest declares dependencies ({names}), and without --mint-masters they cannot be \
         bound: the plan carries the compiler's fabricated ids, not real member masters, so a \
         guest calling one by name gets `dependency-not-bound` at runtime. Re-run with \
         --mint-masters."
    );
}

/// Whether `plan` declares a schedule on any service -- `app deploy`'s own
/// check, since it has no supervisor behind it to ever run one
/// (ADR-0023 §6).
pub(crate) fn plan_declares_a_schedule(plan: &DeploymentPlan) -> bool {
    plan.services.iter().any(|s| s.schedule.is_some())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_deploy(
    instance_id: String,
    manifest_path: PathBuf,
    journal_path: PathBuf,
    mint_masters: bool,
    registry_url: Option<String>,
    inventory: Option<PathBuf>,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let instance_id = AppInstanceId::try_new(instance_id.clone())?;

    let manifest = if manifest_path.extension().and_then(|s| s.to_str()) == Some("wasm") {
        let mut services = BTreeMap::new();
        services.insert(
            LogicalServiceName::new("main"),
            ServiceSpec {
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: manifest_path.to_string_lossy().to_string(),
                    hash: None,
                    interfaces: vec![],
                    env: BTreeMap::new(),
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: Default::default(),
                    fdae: None,
                    health_check: None,
                    assets: None,
                    visibility: Default::default(),
                },
                depends_on: vec![],
                placement: None,
                replicas: 1,
                sharding_strategy: None,
                schedule: None,
                topology_visibility: Default::default(),
            },
        );
        SynAppManifest {
            id: AppBlueprintId::new("legacy-wasm-app"),
            version: Version::new(0, 1, 0),
            description: Some("Auto-generated legacy wrapper".to_string()),
            placement: None,
            services,
            dependencies: BTreeMap::new(),
        }
    } else {
        let toml_str = fs::read_to_string(&manifest_path)?;
        SynAppManifest::from_toml(&toml_str)?
    };

    let catalog =
        LocalFilesystemCatalog::new(manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf());

    let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;
    let target_plan = compiled
        .plans
        .last()
        .ok_or_else(|| anyhow::anyhow!("Compiled deployment contains no plans"))?;

    // `app deploy` has no supervisor behind it, and
    // the supervisor is the scheduler (ADR-0023 §6) -- a schedule
    // deployed this way validates and deploys, and then nothing
    // ever runs it. A warning rather than a refusal, matching the
    // posture already taken for a registry that does not resolve
    // every member: the deploy is valid, one declared behaviour
    // just will not happen.
    if plan_declares_a_schedule(target_plan) {
        eprintln!(
            "warning: this plan declares a schedule, but `app deploy` has no supervisor behind it \
             to run one. Use `roymctl supervisor submit` if the schedule should actually fire."
        );
    }

    let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
    let db_name = journal_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;
    let journal = DeploymentJournal::open(parent_dir, db_name)?;

    // ================================================================
    // Everything that can bail runs BEFORE the journal is written.
    // A record created ahead of a refusal becomes the next run's
    // resume target and a fake recovery plan for `app reconcile`.
    // ================================================================

    // --- inventory + preflight -------------------------------------
    let demand = placement_demand(target_plan);
    let mut clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> = BTreeMap::new();
    let mut client_urls: BTreeMap<SubstrateAlias, String> = BTreeMap::new();
    // `(registry_url, dht_enabled)` per alias whose credential could
    // read node facts; the registry-namespace check below fires only
    // when every placed alias is covered.
    let mut registry_facts: BTreeMap<SubstrateAlias, (Option<String>, bool)> = BTreeMap::new();
    if !demand.is_empty() {
        let inv_path = inventory.clone().unwrap_or_else(|| dir.join("substrates.toml"));
        let inv = SubstrateInventory::load(&inv_path)?;
        check_placement(&inv, &demand, &inv_path)?;
        for alias in demand.keys() {
            let entry = inv.get(alias, &inv_path)?;
            let entry_api_url = entry.api_url.as_deref().unwrap_or(api_url);
            let (entry_identity, entry_ucan) =
                resolve_credentials(alias, entry, &inv_path, dir, run_as, ucan_path)?;
            let mut c = crate::commands::client_for(
                entry.did.clone(),
                entry_api_url,
                dir,
                entry_identity,
                entry_ucan.as_deref(),
            )?;
            c.wait_for_ready(PREFLIGHT_TIMEOUT)
                .await
                .with_context(|| format!("substrate '{alias}' ({}) is not reachable", entry.did))?;

            // `node_facts()` alone, not `status(vec![])` --
            // an empty `service_ids` means "every service this
            // caller may see", so for the node-wide owner credential
            // that call would derive a phase and run a probe for
            // every service the node hosts, just to read these four
            // fields.
            match c.node_facts().await.ok().flatten() {
                None => {
                    // Node facts need node-wide
                    // orchestrator/status. A deploy-only or
                    // app-scoped credential legitimately cannot
                    // read them, and this must say so rather than
                    // pass silently.
                    eprintln!(
                        "note: cannot verify substrate '{alias}''s capabilities or registry \
                         configuration with this credential (needs node-wide \
                         orchestrator/status); falling back to the post-apply probe."
                    );
                }
                Some(facts) => {
                    if let Some(declared) = &entry.capabilities {
                        let reported: BTreeSet<String> =
                            facts.service_types.iter().cloned().collect();
                        for t in declared {
                            let name = match t {
                                ServiceType::Wasm => "wasm",
                                ServiceType::Container => "container",
                                ServiceType::Tcp => "tcp",
                                ServiceType::NativeHost => "nativehost",
                            };
                            if !reported.contains(name) {
                                eprintln!(
                                    "warning: substrate '{alias}' declares '{t:?}' in {} but \
                                     reports it cannot run it",
                                    inv_path.display()
                                );
                            }
                        }
                    }
                    registry_facts.insert(alias.clone(), (facts.registry_url, facts.dht_enabled));
                }
            }

            client_urls.insert(alias.clone(), entry_api_url.to_string());
            clients.insert(alias.clone(), Arc::new(c));
        }
    }

    if target_plan
        .services
        .iter()
        .filter_map(|s| s.substrate.as_ref())
        .collect::<BTreeSet<_>>()
        .len()
        > 1
        && demand.keys().all(|a| registry_facts.contains_key(a))
    {
        let urls: BTreeSet<Option<String>> =
            registry_facts.values().map(|(url, _)| url.clone()).collect();
        let all_dht = registry_facts.values().all(|(_, dht)| *dht);
        if urls.len() > 1 && !all_dht {
            let described: Vec<String> = registry_facts
                .iter()
                .map(|(a, (url, _))| format!("{a}: {}", url.as_deref().unwrap_or("(none)")))
                .collect();
            anyhow::bail!(
                "substrates publish endpoint records into different registries ({}) and not every \
                 substrate has the DHT enabled. Cross-substrate dependency calls cannot resolve. \
                 Point them at one registry, or enable BEP0044 on all of them.",
                described.join(", ")
            );
        }
    }

    // --- the fallback target, built lazily ------------------------
    // Only a service with no placement needs it: a fully-placed app
    // must not require a default substrate it never touches.
    let needs_fallback = target_plan.services.iter().any(|s| s.substrate.is_none());
    let fallback_client: Option<Arc<SyneroymClient>> = if needs_fallback {
        let did = crate::commands::get_substrate_did(substrate_opt.clone(), dir)?;
        let mut fb = crate::commands::client_for(did, api_url, dir, run_as, ucan_path)?;
        fb.wait_for_ready(PREFLIGHT_TIMEOUT).await?;
        Some(Arc::new(fb))
    } else {
        None
    };
    let fallback_target = fallback_client.as_ref().map(|fb| DeployTarget {
        alias: None,
        substrate_did: fb.service_id().to_string(),
        // `roymctl` deliberately keeps the undurable actor:
        // a CLI process exits when the command finishes, so a
        // durable queue behind it would be written and never
        // drained.
        actor: deploy::build_actor(fb.clone()),
    });

    let targets: BTreeMap<SubstrateAlias, DeployTarget> = clients
        .iter()
        .map(|(alias, c)| {
            (
                alias.clone(),
                DeployTarget {
                    alias: Some(alias.clone()),
                    substrate_did: c.service_id().to_string(),
                    actor: deploy::build_actor(c.clone()),
                },
            )
        })
        .collect();

    // --- placement change refusal --------------------------------
    let placed = deploy::resolve_targets(target_plan, &targets, fallback_target.as_ref())?;
    let landed = journal.get_completed_actions_for_instance(&instance_id)?;
    check_no_placement_change(dir, &placed, &landed)?;

    // --- masters -------------------------------------------------
    // Still before the journal record is created: certification can
    // bail on its own (an unreachable instance-identity call, a
    // master-DID mismatch, a missing master file). Running it *after*
    // the record existed used to leave an `Applying` record with zero
    // action rows on exactly that bail -- a phantom record that
    // `recover_applying` would then hand `app reconcile` as a recovery
    // plan for a deploy that never started.
    let (deploy_plan, instance_certs, registry_certs) = if mint_masters {
        member_identity::substitute_and_certify_members(
            dir,
            target_plan,
            &clients,
            fallback_client.as_ref(),
            registry_url.as_deref(),
        )
        .await?
    } else {
        (target_plan.clone(), BTreeMap::new(), BTreeMap::new())
    };

    refuse_unmastered_dependencies(&deploy_plan, mint_masters)?;

    // ================================================================
    // Past this point nothing bails before the journal is consistent.
    // ================================================================

    // --- resume --------------------------------------------------
    let record_id = match journal.get_latest(&instance_id)? {
        Some(rec)
            if matches!(rec.state, DeploymentState::Applying | DeploymentState::Degraded)
                && &rec.plan == target_plan =>
        {
            rec.id
        }
        _ => {
            let id = journal.append(target_plan, DeploymentState::Planned)?;
            journal.update_state(id, DeploymentState::Applying)?;
            id
        }
    };

    // --- apply -------------------------------------------------------
    let report = deploy::apply_plan(
        ApplyRequest {
            plan: &deploy_plan,
            targets: &targets,
            fallback: fallback_target.as_ref(),
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: mint_masters,
            // Unmanaged: `roymctl app deploy` is the operator path;
            // a supervisor's own `submit` presents whatever `adopt`
            // minted, and does not go through this command.
            generation: 0,
            // Unmanaged for the same reason: the epoch is the
            // resident loop's counter, and an absent
            // entry here means the same "no supervisor has written
            // here" that `generation: 0` above already means.
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        record_id,
    )
    .await?;

    // --- post-apply registry verification ------------------------
    let distinct_dids: BTreeSet<&str> =
        placed.iter().map(|(_, t)| t.substrate_did.as_str()).collect();
    if mint_masters && distinct_dids.len() > 1 {
        let mut urls: BTreeSet<String> = client_urls.values().cloned().collect();
        if fallback_client.is_some() {
            urls.insert(api_url.to_string());
        }
        if let Ok(deployed_placed) =
            deploy::resolve_targets(&deploy_plan, &targets, fallback_target.as_ref())
        {
            // Only the members that actually landed this run. A
            // failed service was never deployed at all -- the
            // registry cannot resolve it for that reason, not a
            // topology fault, and probing it anyway spends two full
            // retry budgets per failure to report a warning that
            // blames the wrong thing.
            let deployed: BTreeSet<String> =
                report.deployed.iter().map(ToString::to_string).collect();
            let succeeded: Vec<_> = deployed_placed
                .into_iter()
                .filter(|(svc, _)| deployed.contains(&svc.member_ref().to_string()))
                .collect();
            probe_registry_reachability(&succeeded, &urls).await;
        }
    }

    if report.is_complete() {
        journal.update_state(record_id, DeploymentState::Active)?;
        println!(
            "Successfully deployed {} service(s) for {} ({} already applied, skipped)",
            report.deployed.len(),
            instance_id,
            report.skipped.len()
        );
    } else {
        journal.update_state(record_id, DeploymentState::Degraded)?;
        for failure in &report.failures {
            eprintln!(
                "  {} on {} ({}): {}",
                failure.member_ref,
                failure.alias.as_ref().map(SubstrateAlias::as_str).unwrap_or("--substrate"),
                failure.substrate_did,
                failure.error
            );
        }
        anyhow::bail!(
            "{} of {} services failed to deploy; the app instance is DEGRADED. Nothing was rolled \
             back. Re-run the same command to retry only the failed services.",
            report.failures.len(),
            report.deployed.len() + report.failures.len() + report.skipped.len()
        );
    }

    Ok(())
}

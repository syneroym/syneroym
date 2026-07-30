use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use clap::Subcommand;
use semver::Version;
use syneroym_app_orchestration::{
    ActionRecord, AppInstanceId, DeploymentJournal, DeploymentState, LocalFilesystemCatalog,
    Reconciler, SynAppManifest, compile,
    models::{
        AppBlueprintId, LogicalServiceName, PlannedService, ServiceConfig, ServiceSpec,
        ServiceType, SubstrateAlias,
    },
    substrate_inventory::{SubstrateInventory, check_placement, placement_demand},
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::substrate;
use syneroym_sdk::{
    SyneroymClient,
    deploy::{self, ApplyRequest, DeployTarget, PlanApplier},
};

use super::member_identity;

/// One-shot readiness check per substrate before any deploy call is made
/// (D-A3-8): an unreachable substrate is a clean up-front error rather than a
/// partial application.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    /// Deploy a `SynApp` manifest (Dual Versioning)
    Deploy {
        /// The AppInstanceId to deploy
        instance_id: String,
        /// Path to the SynApp manifest TOML file or legacy .wasm file
        manifest_path: PathBuf,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
        /// Resolve or mint one member master identity per service in the
        /// plan (ADR-0020 §1), substitute each service's fabricated id with
        /// its resolved master DID, and install a certified instance
        /// certificate at deploy. Absent leaves every fabricated id and
        /// certificate untouched -- exactly today's behavior. Minting is
        /// never silent: a new master's backup warning prints at mint time.
        #[arg(long)]
        mint_masters: bool,
        /// Community registry URL to publish/refresh each minted master's
        /// anchor at (D-A1-7). Ignored when `--mint-masters` is absent.
        /// Without it, a minted certificate is unusable on the wire until an
        /// anchor exists some other way (`roymctl identity publish-anchor`).
        #[arg(long)]
        registry_url: Option<String>,
        /// Substrate inventory mapping the aliases a manifest's `placement`
        /// selectors name to DIDs, addresses, credentials, and declared
        /// capabilities (M05A Slice A3). Defaults to
        /// `<dir>/substrates.toml`. Only read when the plan actually places
        /// a service by alias.
        #[arg(long)]
        inventory: Option<PathBuf>,
    },
    /// Reconcile a deployment to recover or compute updates
    Reconcile {
        /// The AppInstanceId to reconcile
        instance_id: String,
        /// Optional path to a new SynApp manifest to diff against
        #[arg(long)]
        manifest_path: Option<PathBuf>,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
    },
}

/// Resolves a possibly-relative path against `dir` (`<roymctl --dir>`),
/// matching how `client_for` already resolves `identities/<name>.key` --
/// an inventory entry's `ucan` path should behave the same way.
fn resolve_under(dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { dir.join(path) }
}

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

/// The one failure this slice cannot catch any earlier (D-A3-17, §0.12): a
/// substrate publishes through its **own** configured registry, which
/// nothing on the wire reports, so a split-registry fleet deploys cleanly
/// and then cannot resolve. This is a heuristic over the URLs `roymctl` was
/// given -- it proves "the registry at this URL cannot resolve member M",
/// never "substrate X cannot" -- and it only warns, never fails the deploy:
/// the services genuinely landed, and marking the deployment `Degraded`
/// would send the next run redeploying them for nothing.
async fn probe_registry_reachability(
    placed: &[(&PlannedService, &DeployTarget)],
    urls: &BTreeSet<String>,
) {
    for url in urls {
        let reg = RegistryClient::new(false, Some(url.clone()));
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

/// The placement-change refusal (D-A3-12, sourced per D-A3-22): a redeploy
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
/// `ACTIVE`-only source misses exactly the sequence A3 introduces.
///
/// Pulled out of `handle` so it is unit-testable against a plain journal,
/// with no live substrate needed.
fn check_no_placement_change(
    dir: &Path,
    placed: &[(&PlannedService, &DeployTarget)],
    landed: &[ActionRecord],
) -> anyhow::Result<()> {
    for (svc, target) in placed {
        let l_ref = svc.logical_ref.to_string();
        if let Some(prev) =
            landed.iter().rfind(|r| r.action_type == "ADD" && r.logical_ref == l_ref)
            && prev.substrate_did != target.substrate_did
        {
            let name = member_identity::member_master_name(&svc.logical_ref, 0)?;
            let real_id = match member_identity::resolve_member_master(dir, &name) {
                Ok(id) => substrate::derive_did_key(&id.public_key()),
                Err(_) => svc.service_id.to_string(),
            };
            anyhow::bail!(
                "service '{}' is already deployed on substrate {} and this run would place it on \
                 {}. A3 does not relocate -- the old instance would keep running and keep \
                 republishing its endpoint record.\nUndeploy it first:\n  roymctl --substrate {} \
                 --as <that substrate's identity> svc remove --svc-id {real_id}\nthen redeploy.",
                svc.logical_ref,
                prev.substrate_alias.as_deref().unwrap_or(prev.substrate_did.as_str()),
                target
                    .alias
                    .as_ref()
                    .map(SubstrateAlias::as_str)
                    .unwrap_or(target.substrate_did.as_str()),
                prev.substrate_did,
            );
        }
    }
    Ok(())
}

pub async fn handle(
    command: &AppCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    match command {
        AppCommands::Deploy {
            instance_id,
            manifest_path,
            journal_path,
            mint_masters,
            registry_url,
            inventory,
        } => {
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
                        },
                        depends_on: vec![],
                        placement: None,
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
                let toml_str = fs::read_to_string(manifest_path)?;
                SynAppManifest::from_toml(&toml_str)?
            };

            let catalog = LocalFilesystemCatalog::new(
                manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            );

            let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;
            let target_plan = compiled
                .plans
                .last()
                .ok_or_else(|| anyhow::anyhow!("Compiled deployment contains no plans"))?;

            let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
            let db_name = journal_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;
            let journal = DeploymentJournal::open(parent_dir, db_name)?;

            // ================================================================
            // Everything that can bail runs BEFORE the journal is written
            // (D-A3-19). A record created ahead of a refusal becomes the next
            // run's resume target and a fake recovery plan for `app reconcile`.
            // ================================================================

            // --- inventory + preflight (D-A3-5/6/7/8) -----------------------
            let demand = placement_demand(target_plan);
            let mut clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> = BTreeMap::new();
            let mut client_urls: BTreeMap<SubstrateAlias, String> = BTreeMap::new();
            if !demand.is_empty() {
                let inv_path = inventory.clone().unwrap_or_else(|| dir.join("substrates.toml"));
                let inv = SubstrateInventory::load(&inv_path)?;
                check_placement(&inv, &demand, &inv_path)?;
                for alias in demand.keys() {
                    let entry = inv.get(alias, &inv_path)?;
                    let entry_api_url = entry.api_url.as_deref().unwrap_or(api_url);
                    let entry_ucan = entry
                        .ucan
                        .as_deref()
                        .map(|p| resolve_under(dir, p))
                        .or_else(|| ucan_path.map(Path::to_path_buf));
                    let mut c = super::client_for(
                        entry.did.clone(),
                        entry_api_url,
                        dir,
                        entry.identity.as_deref().or(run_as),
                        entry_ucan.as_deref(),
                    )?;
                    c.wait_for_ready(PREFLIGHT_TIMEOUT).await.with_context(|| {
                        format!("substrate '{alias}' ({}) is not reachable", entry.did)
                    })?;
                    client_urls.insert(alias.clone(), entry_api_url.to_string());
                    clients.insert(alias.clone(), Arc::new(c));
                }
            }

            // --- the fallback target, built lazily (D-A3-20) ----------------
            // Only a service with no placement needs it: a fully-placed app
            // must not require a default substrate it never touches.
            let needs_fallback = target_plan.services.iter().any(|s| s.substrate.is_none());
            let fallback_client: Option<Arc<SyneroymClient>> = if needs_fallback {
                let did = super::get_substrate_did(substrate_opt.clone(), dir)?;
                let mut fb = super::client_for(did, api_url, dir, run_as, ucan_path)?;
                fb.wait_for_ready(PREFLIGHT_TIMEOUT).await?;
                Some(Arc::new(fb))
            } else {
                None
            };
            let fallback_target = fallback_client.as_ref().map(|fb| DeployTarget {
                alias: None,
                substrate_did: fb.service_id().to_string(),
                applier: fb.clone() as Arc<dyn PlanApplier>,
            });

            let targets: BTreeMap<SubstrateAlias, DeployTarget> = clients
                .iter()
                .map(|(alias, c)| {
                    (
                        alias.clone(),
                        DeployTarget {
                            alias: Some(alias.clone()),
                            substrate_did: c.service_id().to_string(),
                            applier: c.clone() as Arc<dyn PlanApplier>,
                        },
                    )
                })
                .collect();

            // --- placement change refusal (D-A3-12, sourced per D-A3-22) ----
            let placed = deploy::resolve_targets(target_plan, &targets, fallback_target.as_ref())?;
            let landed = journal.get_completed_actions_for_instance(&instance_id)?;
            check_no_placement_change(dir, &placed, &landed)?;

            // ================================================================
            // Past this point nothing bails before the journal is consistent.
            // ================================================================

            // --- resume (D-A3-10) -------------------------------------------
            let record_id = match journal.get_latest(&instance_id)? {
                Some(rec)
                    if matches!(
                        rec.state,
                        DeploymentState::Applying | DeploymentState::Degraded
                    ) && &rec.plan == target_plan =>
                {
                    rec.id
                }
                _ => {
                    let id = journal.append(target_plan, DeploymentState::Planned)?;
                    journal.update_state(id, DeploymentState::Applying)?;
                    id
                }
            };

            // --- masters (§6) ------------------------------------------------
            let (deploy_plan, instance_certs, registry_certs) = if *mint_masters {
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

            if !*mint_masters
                && deploy_plan.services.iter().any(|s| !s.resolved_dependencies.is_empty())
            {
                eprintln!(
                    "warning: deploying without --mint-masters, so declared dependencies are not \
                     bound. A guest calling one by name gets `dependency-not-bound` until the app \
                     is redeployed with --mint-masters."
                );
            }

            // --- apply -------------------------------------------------------
            let report = deploy::apply_plan(
                ApplyRequest {
                    plan: &deploy_plan,
                    targets: &targets,
                    fallback: fallback_target.as_ref(),
                    instance_certificates: &instance_certs,
                    registry_certificates: &registry_certs,
                    emit_bindings: *mint_masters,
                },
                &journal,
                record_id,
            )
            .await?;

            // --- post-apply registry verification (D-A3-17, §0.12) ---------
            let distinct_dids: BTreeSet<&str> =
                placed.iter().map(|(_, t)| t.substrate_did.as_str()).collect();
            if *mint_masters && distinct_dids.len() > 1 {
                let mut urls: BTreeSet<String> = client_urls.values().cloned().collect();
                if fallback_client.is_some() {
                    urls.insert(api_url.to_string());
                }
                if let Ok(deployed_placed) =
                    deploy::resolve_targets(&deploy_plan, &targets, fallback_target.as_ref())
                {
                    probe_registry_reachability(&deployed_placed, &urls).await;
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
                        failure.logical_ref,
                        failure.alias.as_ref().map(SubstrateAlias::as_str).unwrap_or("--substrate"),
                        failure.substrate_did,
                        failure.error
                    );
                }
                anyhow::bail!(
                    "{} of {} services failed to deploy; the app instance is DEGRADED. Nothing \
                     was rolled back. Re-run the same command to retry only the failed services.",
                    report.failures.len(),
                    report.deployed.len() + report.failures.len() + report.skipped.len()
                );
            }
        }
        AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
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
                println!(
                    "Found APPLYING or DEGRADED state for {}. Computed recovery plan:",
                    instance_id
                );
                for action in recovery_plan.actions {
                    println!(" - {:?}", action);
                }
            } else {
                let active = journal.get_last_state(&instance_id, DeploymentState::Active)?;
                if active.is_some() {
                    if let Some(manifest_path) = manifest_path {
                        println!(
                            "App {} is ACTIVE. Diffing active deployment against manifest at {:?}",
                            instance_id, manifest_path
                        );

                        let toml_str = fs::read_to_string(manifest_path)?;
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
                                    println!(" - {:?}", action);
                                }
                            }
                        } else {
                            println!("Compiled deployment contains no plans.");
                        }
                    } else {
                        println!(
                            "App {} is ACTIVE. Provide a --manifest-path to compute a diff.",
                            instance_id
                        );
                    }
                } else {
                    println!("No ACTIVE, APPLYING or DEGRADED state found for {}", instance_id);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use syneroym_app_orchestration::models::{LogicalServiceRef, ServiceId, TopologyMode};
    use syneroym_sdk::DeploymentPlan as WitDeploymentPlan;

    use super::*;

    #[derive(Parser)]
    struct DummyCli {
        #[command(subcommand)]
        command: AppCommands,
    }

    #[test]
    fn test_app_reconcile_command_parsing() {
        let cli = DummyCli::try_parse_from([
            "dummy",
            "reconcile",
            "inst-1",
            "--manifest-path",
            "test.toml",
            "--journal-path",
            "test.db",
        ])
        .unwrap();

        match cli.command {
            AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
                assert_eq!(instance_id, "inst-1");
                assert_eq!(manifest_path, Some(PathBuf::from("test.toml")));
                assert_eq!(journal_path, PathBuf::from("test.db"));
            }
            _ => panic!("Expected Reconcile command"),
        }
    }

    #[test]
    fn deploy_help_lists_inventory() {
        let mut cmd = DummyCli::command();
        let help = cmd
            .get_subcommands_mut()
            .find(|c| c.get_name() == "deploy")
            .expect("deploy subcommand")
            .render_help()
            .to_string();
        assert!(help.contains("--inventory"), "{help}");
    }

    #[test]
    fn resolve_under_leaves_an_absolute_path_untouched() {
        let dir = Path::new("/roymctl/dir");
        let abs = Path::new("/etc/grants/edge-1.json");
        assert_eq!(resolve_under(dir, abs), abs);
    }

    #[test]
    fn resolve_under_joins_a_relative_path_under_dir() {
        let dir = Path::new("/roymctl/dir");
        let rel = Path::new("grants/edge-1.json");
        assert_eq!(resolve_under(dir, rel), Path::new("/roymctl/dir/grants/edge-1.json"));
    }

    // The placement-change refusal never calls `.apply()` -- it only reads
    // `DeployTarget`'s own fields -- so a fake that panics if ever invoked is
    // enough to keep these tests free of any live substrate.
    #[derive(Debug)]
    struct NoopApplier;

    #[async_trait::async_trait]
    impl PlanApplier for NoopApplier {
        async fn apply(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
            unimplemented!("check_no_placement_change must never call apply()")
        }
    }

    fn dummy_config() -> ServiceConfig {
        ServiceConfig {
            service_type: ServiceType::Tcp,
            source: "127.0.0.1:9000".to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: Default::default(),
            fdae: None,
        }
    }

    fn planned_service(
        logical_ref: LogicalServiceRef,
        service_id: &str,
        alias: &str,
    ) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(service_id),
            logical_ref,
            substrate: Some(SubstrateAlias::new(alias)),
            config: dummy_config(),
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
        }
    }

    fn deploy_target(did: &str, alias: &str) -> DeployTarget {
        DeployTarget {
            alias: Some(SubstrateAlias::new(alias)),
            substrate_did: did.to_string(),
            applier: Arc::new(NoopApplier),
        }
    }

    /// The message must resolve the real, deployed member-master DID -- the
    /// journal's plan JSON stores only the compiler's fabricated id, which
    /// the operator cannot act on.
    #[test]
    fn a_placement_change_is_refused_naming_the_deployed_service_id() {
        let dir = tempfile::tempdir().unwrap();
        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        };
        let name = member_identity::member_master_name(&logical_ref, 0).unwrap();
        let master = member_identity::resolve_or_mint_member_master(dir.path(), &name).unwrap();
        let real_did = substrate::derive_did_key(&master.public_key());

        let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];
        let landed = vec![ActionRecord {
            action_type: "ADD".to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zOldNode".to_string(),
        }];

        let err = check_no_placement_change(dir.path(), &placed, &landed).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&real_did), "{msg}");
        assert!(msg.contains("edge-1"), "{msg}");
        assert!(msg.contains("edge-2"), "{msg}");
    }

    /// D-A3-22: the exact sequence round 1 was blind to -- a first partial
    /// deploy leaves the record `Degraded` with one `COMPLETED` row and no
    /// `ACTIVE` record at all. An `ACTIVE`-sourced refusal would pass this
    /// run silently and leave the service running on two nodes.
    #[test]
    fn a_placement_change_is_refused_after_a_degraded_run_not_only_an_active_one() {
        let dir = tempfile::tempdir().unwrap();
        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        };

        let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];
        // No ACTIVE record exists at all -- only a COMPLETED action row from
        // a deployment record left DEGRADED.
        let landed = vec![ActionRecord {
            action_type: "ADD".to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zOldNode".to_string(),
        }];

        let err = check_no_placement_change(dir.path(), &placed, &landed).unwrap_err();
        assert!(err.to_string().contains("already deployed"));
    }

    #[test]
    fn no_placement_change_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        };

        let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1");
        let target = deploy_target("did:key:zSameNode", "edge-1");
        let placed = vec![(&svc, &target)];
        let landed = vec![ActionRecord {
            action_type: "ADD".to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zSameNode".to_string(),
        }];

        check_no_placement_change(dir.path(), &placed, &landed).unwrap();
    }
}

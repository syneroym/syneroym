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
    ActionRecord, ActionState, AlertStore, AppInstanceId, DeploymentJournal, DeploymentPlan,
    DeploymentState, LocalFilesystemCatalog, Reconciler, SynAppManifest, compile,
    models::{
        AppBlueprintId, LogicalServiceName, LogicalServiceRef, MemberRef, PlannedService,
        ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias,
    },
    substrate_inventory::{SubstrateEntry, SubstrateInventory, check_placement, placement_demand},
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_sdk::{
    SubstrateStatus, SyneroymClient,
    deploy::{self, ApplyRequest, DeployTarget, SubstrateActor},
    health,
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
    /// Clear a service's placement bookkeeping so a redeploy to a different
    /// substrate is no longer refused (D-A3-12's escape hatch).
    ///
    /// `svc remove --svc-id <id>` undeploys the running instance but has no
    /// concept of an app instance or a journal, so it cannot itself clear
    /// the `COMPLETED` `ADD` row `check_no_placement_change` refuses on --
    /// nothing else in the tree can. This appends a `REMOVE` row for the
    /// service's most recent placement, at whichever substrate it names,
    /// without contacting any substrate itself. Run `svc remove` against the
    /// old substrate first; this command only clears roymctl's own record of
    /// where the service used to be.
    Forget {
        /// The AppInstanceId the service belongs to
        instance_id: String,
        /// The service's logical name, as written in the manifest
        #[arg(long)]
        service: String,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
    },
    /// Poll every substrate this app instance's services are placed on and
    /// report per-service health (M05A A4). Read-only: nothing is restarted,
    /// retried, or redeployed. Alerts are recorded unless `--no-record` is
    /// passed. Exits non-zero when any service reports a fault; a service
    /// the substrate could not decide about is reported but not fatal
    /// unless `--strict`.
    Health {
        /// The AppInstanceId to poll
        instance_id: String,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
        /// Alert store. Defaults to `alerts.db` beside the journal.
        #[arg(long)]
        alerts_path: Option<PathBuf>,
        #[arg(long)]
        inventory: Option<PathBuf>,
        /// Repeat every N seconds instead of polling once and exiting.
        #[arg(long, value_name = "SECS")]
        watch: Option<u64>,
        /// Poll and print without writing alert rows.
        #[arg(long)]
        no_record: bool,
        /// Treat an undetermined service as a failure too.
        #[arg(long)]
        strict: bool,
    },
    /// Show alerts recorded for an app instance by `app health`.
    Alerts {
        /// The AppInstanceId to read alerts for
        instance_id: String,
        #[arg(long)]
        alerts_path: Option<PathBuf>,
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
        /// Include alerts that have since cleared.
        #[arg(long)]
        all: bool,
    },
}

/// Resolves a possibly-relative path against `dir` (`<roymctl --dir>`),
/// matching how `client_for` already resolves `identities/<name>.key` --
/// an inventory entry's `ucan` path should behave the same way.
fn resolve_under(dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { dir.join(path) }
}

/// Resolves the `identity`/`ucan` pair an alias's client presents (D-A3-6).
///
/// The pair is inherited from **one** source, entry or global, never mixed
/// field-by-field: an entry that sets `identity` but not `ucan` would
/// otherwise fall back to the *global* `--ucan`, connecting as the entry's
/// identity while presenting a token whose `audience_did` is the global
/// one. `client_for`'s own guard only rejects "ucan without as", not this,
/// and the mismatch then fails silently server-side (a `warn!`-logged chain
/// drop), surfacing downstream as a confusing "holds no grant" instead of
/// the real cause -- the exact failure that guard was written to prevent.
fn resolve_credentials<'a>(
    alias: &SubstrateAlias,
    entry: &'a SubstrateEntry,
    inv_path: &Path,
    dir: &Path,
    run_as: Option<&'a str>,
    ucan_path: Option<&'a Path>,
) -> anyhow::Result<(Option<&'a str>, Option<PathBuf>)> {
    if entry.identity.is_some() != entry.ucan.is_some() {
        anyhow::bail!(
            "substrate '{alias}' in {} sets only one of `identity`/`ucan`. A partial override \
             would pair this entry's value with the *global* --as/--ucan for the other field, \
             which is almost never the intended credential -- set both in the entry, or neither \
             to inherit the global pair as-is.",
            inv_path.display()
        );
    }
    if entry.identity.is_some() {
        Ok((entry.identity.as_deref(), entry.ucan.as_deref().map(|p| resolve_under(dir, p))))
    } else {
        Ok((run_as, ucan_path.map(Path::to_path_buf)))
    }
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

/// A3's own post-apply fallback, now that A4's `status` call answers the
/// registry-namespace question at preflight (D-A4-15) whenever the
/// credential can read node facts (D-A4-18) -- the deploy loop above bails
/// before any artifact work when it can see a split namespace outright. This
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
        // DHT enabled (finding 06): the warning below names "enable the DHT"
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
/// Uses `deploy::current_placement` -- the **most recent** row for the
/// logical ref, of either action type, not the most recent `ADD`: `app
/// forget` (below) closes this refusal by appending a `REMOVE` row, and a
/// lookup scoped to `ADD` alone would keep finding the stale `ADD`
/// underneath it forever. A most-recent `REMOVE` means the operator has
/// already cleared the bookkeeping for this service, so any placement --
/// the same substrate or a different one -- is fine. Shared with
/// `apply_plan`'s resume-skip so the two cannot read the journal two
/// different ways again (post-review: they briefly did).
///
/// Pulled out of `handle` so it is unit-testable against a plain journal,
/// with no live substrate needed.
fn check_no_placement_change(
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

/// M05A A5a §4.5: the standing backlog row "`app deploy` without
/// --mint-masters binds nothing" names its own fix -- a manifest declaring
/// dependencies should have no unmastered deploy path at all. A warning at
/// deploy time and a runtime failure at the guest's first `dependency(...)`
/// call was the worst split available: the operator sees the consequence
/// far from the cause. A manifest with no dependencies is unaffected -- an
/// unmastered deploy of an independent service stays valid, which `svc
/// deploy` and every pre-A0 manifest rely on.
///
/// Pulled out of `handle` so it is unit-testable with no live substrate,
/// the same reason `check_no_placement_change` is its own function.
fn refuse_unmastered_dependencies(plan: &DeploymentPlan, mint_masters: bool) -> anyhow::Result<()> {
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
                            health_check: None,
                        },
                        depends_on: vec![],
                        placement: None,
                        replicas: 1,
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
            // D-A4-15: closes two A3 backlog rows once `status` exists.
            // `(registry_url, dht_enabled)` per alias whose credential could
            // read node facts; the registry-namespace check below fires only
            // when every placed alias is covered.
            let mut registry_facts: BTreeMap<SubstrateAlias, (Option<String>, bool)> =
                BTreeMap::new();
            if !demand.is_empty() {
                let inv_path = inventory.clone().unwrap_or_else(|| dir.join("substrates.toml"));
                let inv = SubstrateInventory::load(&inv_path)?;
                check_placement(&inv, &demand, &inv_path)?;
                for alias in demand.keys() {
                    let entry = inv.get(alias, &inv_path)?;
                    let entry_api_url = entry.api_url.as_deref().unwrap_or(api_url);
                    let (entry_identity, entry_ucan) =
                        resolve_credentials(alias, entry, &inv_path, dir, run_as, ucan_path)?;
                    let mut c = super::client_for(
                        entry.did.clone(),
                        entry_api_url,
                        dir,
                        entry_identity,
                        entry_ucan.as_deref(),
                    )?;
                    c.wait_for_ready(PREFLIGHT_TIMEOUT).await.with_context(|| {
                        format!("substrate '{alias}' ({}) is not reachable", entry.did)
                    })?;

                    // A4-06: `node_facts()` alone, not `status(vec![])` --
                    // an empty `service_ids` means "every service this
                    // caller may see", so for the node-wide owner credential
                    // that call would derive a phase and run a probe for
                    // every service the node hosts, just to read these four
                    // fields.
                    match c.node_facts().await.ok().flatten() {
                        None => {
                            // D-A4-18: node facts need node-wide
                            // orchestrator/status. A deploy-only or
                            // app-scoped credential legitimately cannot
                            // read them, and this must say so rather than
                            // pass silently.
                            eprintln!(
                                "note: cannot verify substrate '{alias}''s capabilities or \
                                 registry configuration with this credential (needs node-wide \
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
                                            "warning: substrate '{alias}' declares '{t:?}' in {} \
                                             but reports it cannot run it",
                                            inv_path.display()
                                        );
                                    }
                                }
                            }
                            registry_facts
                                .insert(alias.clone(), (facts.registry_url, facts.dht_enabled));
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
                        "substrates publish endpoint records into different registries ({}) and \
                         not every substrate has the DHT enabled. Cross-substrate dependency \
                         calls cannot resolve. Point them at one registry, or enable BEP0044 on \
                         all of them.",
                        described.join(", ")
                    );
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
                actor: fb.clone() as Arc<dyn SubstrateActor>,
            });

            let targets: BTreeMap<SubstrateAlias, DeployTarget> = clients
                .iter()
                .map(|(alias, c)| {
                    (
                        alias.clone(),
                        DeployTarget {
                            alias: Some(alias.clone()),
                            substrate_did: c.service_id().to_string(),
                            actor: c.clone() as Arc<dyn SubstrateActor>,
                        },
                    )
                })
                .collect();

            // --- placement change refusal (D-A3-12, sourced per D-A3-22) ----
            let placed = deploy::resolve_targets(target_plan, &targets, fallback_target.as_ref())?;
            let landed = journal.get_completed_actions_for_instance(&instance_id)?;
            check_no_placement_change(dir, &placed, &landed)?;

            // --- masters (§6) -------------------------------------------------
            // Still before the journal record is created (D-A3-19, finding 07):
            // certification can bail on its own (an unreachable
            // instance-identity call, a master-DID mismatch, a missing master
            // file). Running it *after* the record existed used to leave an
            // `Applying` record with zero action rows on exactly that bail --
            // the phantom D-A3-19 was written to prevent, which
            // `recover_applying` would then hand `app reconcile` as a recovery
            // plan for a deploy that never started.
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

            refuse_unmastered_dependencies(&deploy_plan, *mint_masters)?;

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

            // --- apply -------------------------------------------------------
            let report = deploy::apply_plan(
                ApplyRequest {
                    plan: &deploy_plan,
                    targets: &targets,
                    fallback: fallback_target.as_ref(),
                    instance_certificates: &instance_certs,
                    registry_certificates: &registry_certs,
                    emit_bindings: *mint_masters,
                    // Unmanaged (M05A A5a): `roymctl app deploy` is the
                    // operator path; a supervisor's own `submit` presents
                    // whatever `adopt` minted, and does not go through
                    // this command.
                    generation: 0,
                    // Unmanaged for the same reason: the epoch is the
                    // resident loop's counter (M05A A5c), and an absent
                    // entry here means the same "no supervisor has written
                    // here" that `generation: 0` above already means.
                    binding_epochs: &BTreeMap::new(),
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
                    // Finding 03: only the members that actually landed this
                    // run. A failed service was never deployed at all -- the
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
        AppCommands::Forget { instance_id, service, journal_path } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;
            let logical_ref = LogicalServiceRef {
                app_instance_id: instance_id.clone(),
                service_name: LogicalServiceName::new(service.as_str()),
            };
            // M05A A5e: the journal now keys every action row on a
            // `MemberRef`, not a bare `LogicalServiceRef`. `--service` names
            // only the logical service, with no way to name one member of a
            // scaled one -- forgets member 0, the only member an unscaled
            // deploy ever has. Forgetting one member of a `replicas > 1`
            // service is not supported by this command yet.
            let l_ref = (MemberRef { logical_ref: logical_ref.clone(), index: 0 }).to_string();

            let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
            let db_name = journal_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;
            let journal = DeploymentJournal::open(parent_dir, db_name)?;

            let landed = journal.get_completed_actions_for_instance(&instance_id)?;
            match landed.iter().rev().find(|r| r.logical_ref == l_ref) {
                None => anyhow::bail!(
                    "no completed deploy is recorded for '{service}' in {instance_id}; nothing to \
                     forget"
                ),
                Some(prev) if prev.action_type == "REMOVE" => {
                    println!("'{service}' in {instance_id} is already forgotten.");
                }
                Some(prev) => {
                    let record_id = journal
                        .get_latest(&instance_id)?
                        .ok_or_else(|| {
                            anyhow::anyhow!("no deployment record found for {instance_id}")
                        })?
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
                        "Forgot '{service}' (was on {}) for {instance_id}. This only clears \
                         roymctl's placement bookkeeping -- if the service instance is still \
                         running there, undeploy it first with `svc remove --svc-id <id>` against \
                         that substrate.",
                        prev.substrate_alias.as_deref().unwrap_or(prev.substrate_did.as_str())
                    );
                }
            }
        }
        AppCommands::Health {
            instance_id,
            journal_path,
            alerts_path,
            inventory,
            watch,
            no_record,
            strict,
        } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;

            let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
            let db_name = journal_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;
            let journal = DeploymentJournal::open(parent_dir, db_name)?;

            let (alerts_dir, alerts_name) = match alerts_path {
                Some(p) => (
                    p.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid alerts path"))?
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Invalid alerts path characters"))?
                        .to_string(),
                ),
                None => (parent_dir.to_path_buf(), "alerts.db".to_string()),
            };
            let alerts = AlertStore::open(&alerts_dir, &alerts_name)?;

            let record = journal
                .get_latest(&instance_id)?
                .ok_or_else(|| anyhow::anyhow!("no deployment record for {instance_id}"))?;
            let landed = journal.get_completed_actions_for_instance(&instance_id)?;

            let mut expected = Vec::new();
            let mut aliases: BTreeMap<String, Option<SubstrateAlias>> = BTreeMap::new();
            for svc in &record.plan.services {
                match deploy::current_placement(&landed, &svc.logical_ref.to_string()) {
                    None => expected.push(health::ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                        member_index: svc.member_index,
                    }),
                    Some(row) => {
                        // The plan's `service_id` is the compiler's
                        // fabricated id whenever the deploy minted masters
                        // (§0.3), so re-derive.
                        let id = member_identity::deployed_service_id(dir, svc)?;
                        expected.push(health::ExpectedService {
                            logical_ref: svc.logical_ref.clone(),
                            service_id: id,
                            substrate_did: row.substrate_did.clone(),
                            member_index: svc.member_index,
                        });
                        aliases.insert(
                            row.substrate_did.clone(),
                            row.substrate_alias.as_deref().map(SubstrateAlias::new),
                        );
                    }
                }
            }

            // Aliased substrates resolve through the inventory exactly as
            // `app deploy` does, including `resolve_credentials`' both-or-
            // neither rule.
            let inv_path = inventory.clone().unwrap_or_else(|| dir.join("substrates.toml"));
            let inv = if aliases.values().any(Option::is_some) {
                Some(SubstrateInventory::load(&inv_path)?)
            } else {
                None
            };

            let mut targets: BTreeMap<String, health::HealthTarget> = BTreeMap::new();
            for (did, alias) in &aliases {
                let (entry_api_url, entry_identity, entry_ucan) = match (alias, &inv) {
                    (Some(a), Some(inv)) => {
                        let entry = inv.get(a, &inv_path)?;
                        let (id, ucan) =
                            resolve_credentials(a, entry, &inv_path, dir, run_as, ucan_path)?;
                        (entry.api_url.clone().unwrap_or_else(|| api_url.to_string()), id, ucan)
                    }
                    _ => (api_url.to_string(), run_as, ucan_path.map(Path::to_path_buf)),
                };
                let client_result = super::client_for(
                    did.clone(),
                    &entry_api_url,
                    dir,
                    entry_identity,
                    entry_ucan.as_deref(),
                );
                let query: Arc<dyn health::StatusQuery> = match client_result {
                    Ok(mut c) => {
                        // NOT fatal, unlike `app deploy`'s preflight: an
                        // unreachable substrate is the exact thing this
                        // command exists to report.
                        match c.wait_for_ready(PREFLIGHT_TIMEOUT).await {
                            Ok(()) => Arc::new(c),
                            Err(e) => Arc::new(UnreachableTarget(e.to_string())),
                        }
                    }
                    Err(e) => Arc::new(UnreachableTarget(e.to_string())),
                };
                targets.insert(
                    did.clone(),
                    health::HealthTarget {
                        alias: alias.clone(),
                        substrate_did: did.clone(),
                        query,
                    },
                );
            }

            let mut report;
            loop {
                report = health::poll_once(&targets, &expected).await;
                print_health_table(&report);
                for u in report.unknowns() {
                    eprintln!(
                        "undetermined: {} on {}: {:?}",
                        u.logical_ref, u.substrate_did, u.signal
                    );
                }
                if !*no_record {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    for (kind, subject) in health::record_report(
                        &alerts,
                        &instance_id,
                        &report,
                        now,
                        &[],
                        health::CertAlertPolicy::Reminder,
                    )? {
                        eprintln!("ALERT {kind:?}: {subject}");
                    }
                }
                match watch {
                    None => break,
                    Some(secs) => tokio::time::sleep(Duration::from_secs(*secs)).await,
                }
            }

            // D-A4-19: faults are fatal; "cannot tell" is not, unless
            // --strict. A `tcp` service that declared no probe is
            // permanently undetermined, and must not make every routine
            // sweep exit non-zero. Reuses the loop's own last sweep rather
            // than polling again, so the exit code always agrees with what
            // was just printed and recorded.
            if !report.faults().is_empty() || (*strict && !report.unknowns().is_empty()) {
                anyhow::bail!("{} service(s) unhealthy for {instance_id}", report.faults().len());
            }
        }
        AppCommands::Alerts { instance_id, alerts_path, journal_path, all } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;
            let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
            let (alerts_dir, alerts_name) = match alerts_path {
                Some(p) => (
                    p.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid alerts path"))?
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Invalid alerts path characters"))?
                        .to_string(),
                ),
                None => (parent_dir.to_path_buf(), "alerts.db".to_string()),
            };
            let alerts = AlertStore::open(&alerts_dir, &alerts_name)?;
            let rows =
                if *all { alerts.all(&instance_id)? } else { alerts.active(&instance_id)? };
            if rows.is_empty() {
                println!("no {}alerts for {instance_id}", if *all { "" } else { "active " });
            }
            for row in rows {
                println!(
                    "{:<24} {:<20} {:<28} {}{}",
                    row.logical_ref.as_deref().unwrap_or("(substrate)"),
                    row.substrate_alias.as_deref().unwrap_or(row.substrate_did.as_str()),
                    format!("{:?}", row.kind),
                    row.detail,
                    if row.cleared_at.is_some() { " [cleared]" } else { "" }
                );
            }
        }
    }
    Ok(())
}

/// A substrate that never came up: "the connection never came up" and "the
/// status call failed" take **one** path into `poll_once` instead of two, so
/// `SubstrateUnreachable` has a single producer.
#[derive(Debug)]
struct UnreachableTarget(String);

#[async_trait::async_trait]
impl health::StatusQuery for UnreachableTarget {
    async fn status(&self, _service_ids: Vec<String>) -> anyhow::Result<SubstrateStatus, String> {
        Err(self.0.clone())
    }
}

fn print_health_table(report: &health::HealthReport) {
    println!("{:<24} {:<12} {:<20} DETAIL", "SERVICE", "SUBSTRATE", "STATUS");
    for s in &report.services {
        let (status, detail) = match &s.signal {
            health::Signal::Healthy => ("HEALTHY".to_string(), "-".to_string()),
            health::Signal::SubstrateUnreachable(d) => {
                ("SUBSTRATE_UNREACHABLE".to_string(), d.clone())
            }
            health::Signal::InstanceNotRunning(d) => {
                ("INSTANCE_NOT_RUNNING".to_string(), d.clone())
            }
            health::Signal::ProbeFailing(d) => ("PROBE_FAILING".to_string(), d.clone()),
            health::Signal::Unknown(d) => ("UNDETERMINED".to_string(), d.clone()),
            health::Signal::NotDeployed => ("NOT_DEPLOYED".to_string(), "-".to_string()),
        };
        println!(
            "{:<24} {:<12} {:<20} {}",
            s.logical_ref,
            s.alias.as_ref().map(SubstrateAlias::as_str).unwrap_or(s.substrate_did.as_str()),
            status,
            detail
        );
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use syneroym_app_orchestration::models::{ServiceId, TopologyMode};
    use syneroym_identity::substrate;
    use syneroym_sdk::{BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan};

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
    fn test_app_forget_command_parsing() {
        let cli = DummyCli::try_parse_from([
            "dummy",
            "forget",
            "inst-1",
            "--service",
            "backend",
            "--journal-path",
            "test.db",
        ])
        .unwrap();

        match cli.command {
            AppCommands::Forget { instance_id, service, journal_path } => {
                assert_eq!(instance_id, "inst-1");
                assert_eq!(service, "backend");
                assert_eq!(journal_path, PathBuf::from("test.db"));
            }
            _ => panic!("Expected Forget command"),
        }
    }

    #[test]
    fn test_app_health_command_parsing() {
        let cli = DummyCli::try_parse_from([
            "dummy",
            "health",
            "inst-1",
            "--journal-path",
            "test.db",
            "--watch",
            "5",
            "--strict",
        ])
        .unwrap();

        match cli.command {
            AppCommands::Health { instance_id, journal_path, watch, strict, no_record, .. } => {
                assert_eq!(instance_id, "inst-1");
                assert_eq!(journal_path, PathBuf::from("test.db"));
                assert_eq!(watch, Some(5));
                assert!(strict);
                assert!(!no_record);
            }
            _ => panic!("Expected Health command"),
        }
    }

    #[test]
    fn test_app_alerts_command_parsing() {
        let cli = DummyCli::try_parse_from(["dummy", "alerts", "inst-1", "--all"]).unwrap();

        match cli.command {
            AppCommands::Alerts { instance_id, all, .. } => {
                assert_eq!(instance_id, "inst-1");
                assert!(all);
            }
            _ => panic!("Expected Alerts command"),
        }
    }

    #[test]
    fn health_help_lists_no_record_watch_and_strict() {
        let mut cmd = DummyCli::command();
        let help = cmd
            .get_subcommands_mut()
            .find(|c| c.get_name() == "health")
            .expect("health subcommand")
            .render_help()
            .to_string();
        assert!(help.contains("--no-record"), "{help}");
        assert!(help.contains("--watch"), "{help}");
        assert!(help.contains("--strict"), "{help}");
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

    fn entry(identity: Option<&str>, ucan: Option<&str>) -> SubstrateEntry {
        SubstrateEntry {
            did: "did:key:z6MkExampleNodeA".to_string(),
            api_url: None,
            identity: identity.map(str::to_string),
            ucan: ucan.map(PathBuf::from),
            capabilities: None,
        }
    }

    /// D-A3-6, finding 02: an entry overriding neither field inherits the
    /// global pair as-is -- today's pre-A3 behavior, unaffected.
    #[test]
    fn resolve_credentials_falls_back_to_the_global_pair_when_the_entry_sets_neither() {
        let alias = SubstrateAlias::new("edge-1");
        let e = entry(None, None);
        let (id, ucan) = resolve_credentials(
            &alias,
            &e,
            Path::new("substrates.toml"),
            Path::new("/dir"),
            Some("global-op"),
            Some(Path::new("grants/global.json")),
        )
        .unwrap();
        assert_eq!(id, Some("global-op"));
        assert_eq!(ucan.as_deref(), Some(Path::new("grants/global.json")));
    }

    /// An entry overriding both fields together is always consistent,
    /// regardless of what the globals are.
    #[test]
    fn resolve_credentials_uses_the_entrys_own_pair_when_it_sets_both() {
        let alias = SubstrateAlias::new("edge-1");
        let e = entry(Some("edge1-op"), Some("grants/edge-1.json"));
        let (id, ucan) = resolve_credentials(
            &alias,
            &e,
            Path::new("substrates.toml"),
            Path::new("/dir"),
            Some("global-op"),
            Some(Path::new("grants/global.json")),
        )
        .unwrap();
        assert_eq!(id, Some("edge1-op"));
        assert_eq!(ucan.as_deref(), Some(Path::new("/dir/grants/edge-1.json")));
    }

    /// Finding 02's exact hazard: `identity` overridden, `ucan` left to fall
    /// back to a *global* `--ucan` whose audience is the global identity,
    /// not this entry's. Must be rejected, not silently paired.
    #[test]
    fn resolve_credentials_rejects_identity_override_with_a_global_ucan_present() {
        let alias = SubstrateAlias::new("edge-1");
        let e = entry(Some("edge1-op"), None);
        let err = resolve_credentials(
            &alias,
            &e,
            Path::new("substrates.toml"),
            Path::new("/dir"),
            Some("global-op"),
            Some(Path::new("grants/global.json")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("edge-1"), "{err}");
    }

    /// The symmetric case: `ucan` overridden, `identity` left to fall back to
    /// a global `--as` the entry's token was never minted for.
    #[test]
    fn resolve_credentials_rejects_ucan_override_with_a_global_identity_present() {
        let alias = SubstrateAlias::new("edge-1");
        let e = entry(None, Some("grants/edge-1.json"));
        let err = resolve_credentials(
            &alias,
            &e,
            Path::new("substrates.toml"),
            Path::new("/dir"),
            Some("global-op"),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("edge-1"), "{err}");
    }

    // The placement-change refusal never calls the actor -- it only reads
    // `DeployTarget`'s own fields -- so a fake that panics if ever invoked is
    // enough to keep these tests free of any live substrate.
    #[derive(Debug)]
    struct NoopApplier;

    #[async_trait::async_trait]
    impl SubstrateActor for NoopApplier {
        async fn apply_plan(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
            unimplemented!("check_no_placement_change must never call apply_plan()")
        }

        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("check_no_placement_change must never call write_bindings()")
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("check_no_placement_change must never call restart()")
        }

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("check_no_placement_change must never call renew_cert()")
        }

        async fn instance_identity(
            &self,
            _service_id: &str,
        ) -> Result<syneroym_sdk::InstanceIdentity, String> {
            unimplemented!("check_no_placement_change must never call instance_identity()")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("check_no_placement_change must never call held_generation()")
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
            health_check: None,
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
            member_index: 0,
        }
    }

    fn deploy_target(did: &str, alias: &str) -> DeployTarget {
        DeployTarget {
            alias: Some(SubstrateAlias::new(alias)),
            substrate_did: did.to_string(),
            actor: Arc::new(NoopApplier),
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
        let name = member_identity::member_master_name(&logical_ref, 0);
        let master = member_identity::resolve_or_mint_member_master(dir.path(), &name).unwrap();
        let real_did = substrate::derive_did_key(&master.public_key());

        let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];
        let landed = vec![ActionRecord {
            action_type: "ADD".to_string(),
            logical_ref: format!("{logical_ref}#0"),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zOldNode".to_string(),
        }];

        let err = check_no_placement_change(dir.path(), &placed, &landed).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&real_did), "{msg}");
        assert!(msg.contains("edge-1"), "{msg}");
        assert!(msg.contains("edge-2"), "{msg}");
    }

    fn dummy_deployment_plan(instance_id: &AppInstanceId, svc: PlannedService) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: instance_id.clone(),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: Version::new(1, 0, 0),
            services: vec![svc],
        }
    }

    /// D-A3-22: the exact sequence round 1 was blind to -- a first partial
    /// deploy leaves the record `Degraded` with one `COMPLETED` row and no
    /// `ACTIVE` record at all. An `ACTIVE`-sourced refusal would pass this
    /// run silently and leave the service running on two nodes.
    ///
    /// Post-review (finding 04): `landed` used to be a hand-typed literal,
    /// identical in shape to the previous test's -- which proves nothing
    /// about D-A3-22's actual claim, that the rows come from `COMPLETED`
    /// actions across every record rather than the last `ACTIVE` plan.
    /// `check_no_placement_change` takes `landed` as a plain slice by
    /// design (so it needs no live substrate), so the only way to pin the
    /// *source* is to build the state through a real journal, the same way
    /// `handle` does, and read `landed` back out with the real query.
    #[test]
    fn a_placement_change_is_refused_after_a_degraded_run_not_only_an_active_one() {
        let dir = tempfile::tempdir().unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        let logical_ref = LogicalServiceRef {
            app_instance_id: instance_id.clone(),
            service_name: LogicalServiceName::new("backend"),
        };

        let journal = DeploymentJournal::open_in_memory().unwrap();
        let plan = dummy_deployment_plan(
            &instance_id,
            planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1"),
        );
        let deployment_id = journal.append(&plan, DeploymentState::Applying).unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &format!("{logical_ref}#0"),
                Some("edge-1"),
                "did:key:zOldNode",
                ActionState::Completed,
            )
            .unwrap();
        journal.update_state(deployment_id, DeploymentState::Degraded).unwrap();

        // No ACTIVE record exists for this instance at all -- an
        // `ACTIVE`-sourced refusal would find nothing and pass silently.
        assert!(journal.get_last_state(&instance_id, DeploymentState::Active).unwrap().is_none());

        let landed = journal.get_completed_actions_for_instance(&instance_id).unwrap();

        let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];

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

    /// Finding 01: a most-recent `REMOVE` row (what `app forget` appends)
    /// must clear the refusal, even though an older `ADD` row for the same
    /// logical ref still sits underneath it -- a `rfind` scoped to `ADD`
    /// alone would miss the `REMOVE` and refuse forever.
    #[test]
    fn a_remove_row_after_an_add_clears_the_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        };

        let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];
        let landed = vec![
            ActionRecord {
                action_type: "ADD".to_string(),
                logical_ref: logical_ref.to_string(),
                substrate_alias: Some("edge-1".to_string()),
                substrate_did: "did:key:zOldNode".to_string(),
            },
            ActionRecord {
                action_type: "REMOVE".to_string(),
                logical_ref: logical_ref.to_string(),
                substrate_alias: Some("edge-1".to_string()),
                substrate_did: "did:key:zOldNode".to_string(),
            },
        ];

        check_no_placement_change(dir.path(), &placed, &landed).unwrap();
    }

    /// `app forget` end to end: a real journal, on disk, exactly as `handle`
    /// itself opens it -- proving the whole escape from finding 01, not just
    /// `check_no_placement_change`'s half of it.
    #[tokio::test]
    async fn app_forget_appends_a_remove_row_that_clears_a_later_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let instance_id = AppInstanceId::new("inst-forget");
        let logical_ref = LogicalServiceRef {
            app_instance_id: instance_id.clone(),
            service_name: LogicalServiceName::new("backend"),
        };
        let journal_path = dir.path().join("deployments.db");

        {
            let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
            let plan = dummy_deployment_plan(
                &instance_id,
                planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1"),
            );
            let deployment_id = journal.append(&plan, DeploymentState::Active).unwrap();
            journal
                .append_action(
                    deployment_id,
                    "ADD",
                    &format!("{logical_ref}#0"),
                    Some("edge-1"),
                    "did:key:zOldNode",
                    ActionState::Completed,
                )
                .unwrap();
        }

        handle(
            &AppCommands::Forget {
                instance_id: instance_id.to_string(),
                service: "backend".to_string(),
                journal_path: journal_path.clone(),
            },
            "http://localhost:1",
            None,
            dir.path(),
            None,
            None,
        )
        .await
        .unwrap();

        let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
        let landed = journal.get_completed_actions_for_instance(&instance_id).unwrap();
        let last =
            landed.iter().rev().find(|r| r.logical_ref == format!("{logical_ref}#0")).unwrap();
        assert_eq!(last.action_type, "REMOVE");
        assert_eq!(last.substrate_did, "did:key:zOldNode");

        // A redeploy naming a different substrate is no longer refused.
        let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-2");
        let target = deploy_target("did:key:zNewNode", "edge-2");
        let placed = vec![(&svc, &target)];
        check_no_placement_change(dir.path(), &placed, &landed).unwrap();

        // Forgetting again is a no-op, not a second REMOVE row.
        handle(
            &AppCommands::Forget {
                instance_id: instance_id.to_string(),
                service: "backend".to_string(),
                journal_path,
            },
            "http://localhost:1",
            None,
            dir.path(),
            None,
            None,
        )
        .await
        .unwrap();
        let landed_again = journal.get_completed_actions_for_instance(&instance_id).unwrap();
        assert_eq!(landed_again.len(), landed.len(), "{landed_again:?}");
    }

    #[tokio::test]
    async fn app_forget_with_nothing_deployed_names_the_service_and_instance() {
        let dir = tempfile::tempdir().unwrap();
        let journal_path = dir.path().join("deployments.db");
        let err = handle(
            &AppCommands::Forget {
                instance_id: "inst-empty".to_string(),
                service: "backend".to_string(),
                journal_path,
            },
            "http://localhost:1",
            None,
            dir.path(),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("nothing to forget"), "{err}");
    }

    // ── M05A A5a §4.5: unmastered deploy refused when deps are declared ──

    /// A manifest declaring a dependency has no unmastered deploy path --
    /// the plan carries the compiler's fabricated ids, which resolve to no
    /// real key, so binding it would only push the failure from deploy
    /// time to the guest's first `dependency(...)` call.
    #[test]
    fn a_manifest_declaring_depends_on_is_refused_without_mint_masters() {
        let instance_id = AppInstanceId::new("inst-1");
        let logical_ref = LogicalServiceRef {
            app_instance_id: instance_id.clone(),
            service_name: LogicalServiceName::new("frontend"),
        };
        let mut svc = planned_service(logical_ref, "did:key:hFabricated", "edge-1");
        svc.resolved_dependencies.insert(
            LogicalServiceName::new("backend"),
            vec![ServiceId::new("did:key:hBackendFabricated")],
        );
        let plan = dummy_deployment_plan(&instance_id, svc);

        let err = refuse_unmastered_dependencies(&plan, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("backend"), "{msg}");
        assert!(msg.contains("--mint-masters"), "{msg}");
    }

    /// The boundary: an unmastered deploy of an independent service (no
    /// declared dependencies) stays valid -- `svc deploy` and every
    /// pre-A0 manifest rely on this.
    #[test]
    fn a_manifest_with_no_dependencies_still_deploys_without_mint_masters() {
        let instance_id = AppInstanceId::new("inst-1");
        let logical_ref = LogicalServiceRef {
            app_instance_id: instance_id.clone(),
            service_name: LogicalServiceName::new("frontend"),
        };
        let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-1");
        let plan = dummy_deployment_plan(&instance_id, svc);

        assert!(refuse_unmastered_dependencies(&plan, false).is_ok());
    }
}

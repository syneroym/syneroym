//! `roymctl supervisor …`: an operator's interface to a running App
//! Supervisor -- submit desired state, adopt/release/pause/resume/retire an
//! app instance, force a reconcile, read status/alerts, and back up or
//! adopt a master (M05A A5b) -- a member master, or, since M05A A7, the
//! app instance's own.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Subcommand;
use syneroym_app_orchestration::{
    AppInstanceId, LocalFilesystemCatalog, SynAppManifest, compile,
    models::{ServiceType, SubstrateAlias},
    substrate_inventory::{SubstrateInventory, placement_demand},
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_sdk::mapper::INLINE_ARTIFACT_PREFIX;
use syneroym_ucan::CapabilityToken;

/// Matches `app.rs`'s own preflight budget.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Subcommand, Debug, Clone)]
pub enum SupervisorCommands {
    /// Compile a manifest and hand it to a supervisor as desired state.
    /// The supervisor resolves-or-mints its own member masters and
    /// substitutes them -- this command sends the compiler's fabricated
    /// ids, never a master.
    Submit {
        instance_id: String,
        manifest_path: PathBuf,
        #[arg(long)]
        inventory: Option<PathBuf>,
        /// Submit at this generation. Omit to reuse the one `adopt` last
        /// minted for this instance (0 for a first, unmanaged submission).
        #[arg(long)]
        generation: Option<u64>,
    },
    /// Mint the next generation for an app instance and claim it: reads
    /// every managed substrate's held generation and writes `held + 1`.
    Adopt {
        instance_id: String,
    },
    /// Hand an instance back to manual operation. Does not undeploy it.
    Release {
        instance_id: String,
    },
    Pause {
        instance_id: String,
    },
    Resume {
        instance_id: String,
    },
    /// Stop managing and release. Deliberately not a teardown.
    Retire {
        instance_id: String,
    },
    /// Re-run the mint/certify/apply pipeline against the currently stored
    /// desired state, without waiting for a resident loop.
    Reconcile {
        instance_id: String,
    },
    Status {
        instance_id: String,
    },
    Alerts {
        instance_id: String,
        #[arg(long)]
        all: bool,
    },
    /// Ask the supervisor to write a master -- a member's, or the app
    /// instance's own (M05A A7, named `app-<instance-id>`) -- into its
    /// configured `master_backup_dir` and print the path it wrote (backup
    /// is mandatory: mint-in-place means the operator holds nothing until
    /// they ask). Takes a name, not a path -- the destination is operator
    /// config on the supervisor's side, so no caller can steer a private
    /// key anywhere.
    ExportMaster {
        name: String,
    },
    /// Ask the supervisor to adopt a master already placed in that same
    /// directory: either a member master (an A0-A4 deployment's
    /// `<dir>/identities/member-*.key`), or an app instance's own master
    /// carried from another supervisor by a prior `export-master`. For a
    /// handover of an app instance's master, run this *before* `adopt` on
    /// the new supervisor -- adopting first mints a second app identity
    /// under the same name (M05A A7).
    ImportMaster {
        name: String,
    },
    /// Revoke one placed member's derived instance key: publish it into the
    /// member master's anchor revoked-key list, and stop this supervisor
    /// re-certifying that placement on every write path -- the resident
    /// loop's renewal, `submit`, and `reconcile` alike.
    ///
    /// Scoped to one member, not the instance. The member's process keeps
    /// running: use `roymctl app remove` (or `supervisor retire`) if the
    /// process itself should stop.
    RevokeInstance {
        instance_id: String,
        /// The member's full `MemberRef`, as `status` prints it:
        /// `<instance-id>/<service-name>#<index>` (M05A A5e).
        logical_ref: String,
    },
    /// List this instance's outbox: binding writes still waiting for
    /// delivery, or currently claimed by the worker (M05B B1 review
    /// finding 13).
    Outbox {
        instance_id: String,
    },
    /// List this instance's dead letters: queued binding writes whose
    /// delivery attempt budget is exhausted (M05B B1).
    DeadLetters {
        instance_id: String,
    },
    /// Re-enqueue a dead letter -- it is delivered through the ordinary
    /// worker path, not executed inline. A second failure returns it here
    /// with its attempt history intact.
    Replay {
        instance_id: String,
        dead_letter_id: u64,
    },
    /// List this instance's declared schedules and what the supervisor has
    /// most recently done about each.
    Schedules {
        instance_id: String,
    },
}

fn resolve_under(dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { dir.join(path) }
}

/// `release`/`retire` now act on every placed substrate they *can* reach
/// rather than failing the whole call the moment one is down (S7, Slice
/// A5b review) -- this surfaces which ones, if any, still hold a stale
/// generation stamp and need a manual re-release once reachable again.
fn print_unreleased_substrates(result: &serde_json::Value) {
    let Some(unreleased) = result.get("unreleased_substrates").and_then(|v| v.as_array()) else {
        return;
    };
    if unreleased.is_empty() {
        return;
    }
    println!("  warning: could not release the generation stamp on:");
    for entry in unreleased {
        let alias = entry.get("alias").and_then(|v| v.as_str()).unwrap_or("?");
        let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
        println!("    {alias}: {reason}");
    }
}

/// Builds the `alias -> {did, api-url, ucan}` inventory a submitted plan's
/// placed aliases need, resolving each alias's local `ucan` file (a path
/// meaningless once it crosses the wire) into the token itself.
fn build_supervisor_inventory(
    demand: &BTreeMap<SubstrateAlias, BTreeSet<ServiceType>>,
    inv_path: &Path,
    inv: &SubstrateInventory,
) -> anyhow::Result<BTreeMap<String, SupervisorInventoryEntry>> {
    let mut out = BTreeMap::new();
    for alias in demand.keys() {
        let entry = inv.get(alias, inv_path)?;
        let ucan = match &entry.ucan {
            None => None,
            Some(path) => {
                let path = resolve_under(inv_path.parent().unwrap_or(Path::new(".")), path);
                let raw = fs::read_to_string(&path).map_err(|e| {
                    anyhow::anyhow!("failed to read UCAN token at {}: {e}", path.display())
                })?;
                let token: CapabilityToken = serde_json::from_str(&raw).map_err(|e| {
                    anyhow::anyhow!("invalid UCAN token JSON at {}: {e}", path.display())
                })?;
                Some(token)
            }
        };
        out.insert(
            alias.as_str().to_string(),
            SupervisorInventoryEntry {
                did: entry.did.clone(),
                api_url: entry.api_url.clone(),
                ucan,
            },
        );
    }
    Ok(out)
}

pub async fn handle(
    command: &SupervisorCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let substrate_did = super::get_substrate_did(substrate_opt, dir)?;
    let mut client = super::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(PREFLIGHT_TIMEOUT).await?;

    match command {
        SupervisorCommands::Submit { instance_id, manifest_path, inventory, generation } => {
            let instance_id_typed = AppInstanceId::try_new(instance_id.clone())?;
            let toml_str = fs::read_to_string(manifest_path)?;
            let manifest = SynAppManifest::from_toml(&toml_str)?;
            let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));
            let catalog = LocalFilesystemCatalog::new(manifest_dir.to_path_buf());

            let compiled = compile(instance_id_typed, &manifest, &catalog).await?;
            if compiled.plans.len() > 1 {
                anyhow::bail!(
                    "manifest '{}' declares a Spawn dependency, which compiles to more than one \
                     deployment plan; `supervisor submit` sends exactly one plan and would \
                     silently drop the others. Spawn dependencies are not yet supported here.",
                    manifest_path.display()
                );
            }
            let mut target_plan = compiled
                .plans
                .last()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("compiled deployment contains no plans"))?;

            // Inline every Wasm artifact (D-A5-7): the supervisor applies
            // this plan on a remote substrate with no access to this
            // machine's filesystem.
            for svc in &mut target_plan.services {
                if svc.config.service_type == ServiceType::Wasm
                    && !svc.config.source.starts_with("http://")
                    && !svc.config.source.starts_with("https://")
                    && !svc.config.source.starts_with(INLINE_ARTIFACT_PREFIX)
                {
                    let path = resolve_under(manifest_dir, Path::new(&svc.config.source));
                    let bytes = fs::read(&path).map_err(|e| {
                        anyhow::anyhow!("failed to read Wasm artifact at {}: {e}", path.display())
                    })?;
                    svc.config.source = format!("{INLINE_ARTIFACT_PREFIX}{}", hex::encode(bytes));
                }
            }

            let demand = placement_demand(&target_plan);
            let inv_path = inventory.clone().unwrap_or_else(|| dir.join("substrates.toml"));
            let supervisor_inventory = if demand.is_empty() {
                BTreeMap::new()
            } else {
                let inv = SubstrateInventory::load(&inv_path)?;
                build_supervisor_inventory(&demand, &inv_path, &inv)?
            };

            let plan_json = target_plan.to_json()?;
            let inventory_json = serde_json::to_string(&supervisor_inventory)?;

            // "Omit to reuse the one `adopt` last minted for this
            // instance" is this flag's own help text, but the generation
            // this supervisor already holds is server state `roymctl`
            // does not otherwise track -- reading it back from `status`
            // is what makes the documented default true rather than
            // silently presenting 0 against an adopted instance (S2,
            // Slice A5b review). A lookup failure just means "no prior
            // desired state", i.e. a genuine first submission at 0, not
            // an error worth surfacing here.
            let effective_generation = match generation {
                Some(g) => *g,
                None => client
                    .request("supervisor", "status", serde_json::to_value([instance_id.clone()])?)
                    .await
                    .ok()
                    .and_then(|res| res.result.get("generation").and_then(|v| v.as_u64()))
                    .unwrap_or(0),
            };

            let res = client
                .request(
                    "supervisor",
                    "submit",
                    serde_json::to_value([serde_json::json!({
                        "app_instance_id": instance_id,
                        "plan_json": plan_json,
                        "inventory_json": inventory_json,
                        "generation": effective_generation,
                    })])?,
                )
                .await?;

            println!("Submitted desired state for app instance '{instance_id}'.");
            if let Some(masters) = res.result.get("masters").and_then(|m| m.as_array()) {
                for m in masters {
                    let service_name =
                        m.get("service_name").and_then(|v| v.as_str()).unwrap_or("?");
                    let master_did = m.get("master_did").and_then(|v| v.as_str()).unwrap_or("?");
                    // The vault key `export-master` actually takes, not the
                    // bare logical name above -- printing `service_name`
                    // here produced a command that always failed with "no
                    // master named '<service_name>' in this vault" (S1,
                    // Slice A5b review).
                    let vault_name =
                        m.get("vault_name").and_then(|v| v.as_str()).unwrap_or(service_name);
                    println!(
                        "  {service_name}: {master_did} -- back it up with `roymctl supervisor \
                         export-master {vault_name}`"
                    );
                }
            }
        }
        SupervisorCommands::Adopt { instance_id } => {
            let res = client
                .request("supervisor", "adopt", serde_json::to_value([instance_id.clone()])?)
                .await?;
            let generation = res.result.get("generation").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("Adopted '{instance_id}' at generation {generation}.");
            // M05A A7 (D-A7-8): told at the moment the key exists, the same
            // rule `submit`'s own `minted-master` rows already encode --
            // not left to a follow-up `status` call.
            let app_master_did =
                res.result.get("app_master_did").and_then(|v| v.as_str()).unwrap_or("?");
            let vault_name = res.result.get("vault_name").and_then(|v| v.as_str()).unwrap_or("?");
            println!(
                "  app master: {app_master_did} -- back it up with `roymctl supervisor \
                 export-master {vault_name}`"
            );
        }
        SupervisorCommands::Release { instance_id } => {
            let res = client
                .request("supervisor", "release", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("Released '{instance_id}' back to manual operation.");
            print_unreleased_substrates(&res.result);
        }
        SupervisorCommands::Pause { instance_id } => {
            let res = client
                .request("supervisor", "pause", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("Paused '{instance_id}'.");
            // ADR-0022 §2: a paused instance gets zero write-phase work,
            // so its Tier-1 registry record stops refreshing along with
            // everything else -- surfaced here rather than left to a
            // follow-up `status` call, the same rule `adopt`'s own
            // printed line already follows for the app master DID.
            if let Some(expires_at) =
                res.result.get("app_record_expires_at").and_then(|v| v.as_u64())
            {
                println!(
                    "  its Tier-1 registry record stops refreshing while paused, and expires at \
                     Unix time {expires_at} unless resumed before then"
                );
            }
        }
        SupervisorCommands::Resume { instance_id } => {
            client
                .request("supervisor", "resume", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("Resumed '{instance_id}'.");
        }
        SupervisorCommands::Retire { instance_id } => {
            let res = client
                .request("supervisor", "retire", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("Retired '{instance_id}'.");
            print_unreleased_substrates(&res.result);
        }
        SupervisorCommands::Reconcile { instance_id } => {
            client
                .request(
                    "supervisor",
                    "force-reconcile",
                    serde_json::to_value([instance_id.clone()])?,
                )
                .await?;
            println!("Reconciled '{instance_id}'.");
        }
        SupervisorCommands::Status { instance_id } => {
            let res = client
                .request("supervisor", "status", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("{}", serde_json::to_string_pretty(&res.result)?);
        }
        SupervisorCommands::Alerts { instance_id, all } => {
            let res = client
                .request("supervisor", "alerts", serde_json::to_value((instance_id.clone(), *all))?)
                .await?;
            println!("{}", serde_json::to_string_pretty(&res.result)?);
        }
        SupervisorCommands::ExportMaster { name } => {
            let res = client
                .request("supervisor", "export-master", serde_json::to_value([name.clone()])?)
                .await?;
            println!("Wrote master '{name}' to {}", res.result);
        }
        SupervisorCommands::ImportMaster { name } => {
            client
                .request("supervisor", "import-master", serde_json::to_value([name.clone()])?)
                .await?;
            println!("Imported master '{name}' into the vault.");
        }
        SupervisorCommands::RevokeInstance { instance_id, logical_ref } => {
            let result = client
                .request(
                    "supervisor",
                    "revoke-instance",
                    serde_json::to_value((instance_id.clone(), logical_ref.clone()))?,
                )
                .await?;
            let instance_did =
                result.result.get("instance_did").and_then(|v| v.as_str()).unwrap_or("?");
            println!("Revoked the instance key for '{logical_ref}' ({instance_did}).");
            println!(
                "  Nothing will re-certify this member: not the resident loop, not `submit`, not \
                 `reconcile`."
            );
            println!("  Its process is still running -- remove it separately if that is intended.");
        }
        SupervisorCommands::Outbox { instance_id } => {
            let res = client
                .request("supervisor", "outbox", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("{}", serde_json::to_string_pretty(&res.result)?);
        }
        SupervisorCommands::DeadLetters { instance_id } => {
            let res = client
                .request("supervisor", "dead-letters", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("{}", serde_json::to_string_pretty(&res.result)?);
        }
        SupervisorCommands::Replay { instance_id, dead_letter_id } => {
            client
                .request(
                    "supervisor",
                    "replay",
                    serde_json::to_value((instance_id.clone(), *dead_letter_id))?,
                )
                .await?;
            println!("Replayed dead letter {dead_letter_id} for '{instance_id}'.");
        }
        SupervisorCommands::Schedules { instance_id } => {
            let res = client
                .request("supervisor", "schedules", serde_json::to_value([instance_id.clone()])?)
                .await?;
            println!("{}", serde_json::to_string_pretty(&res.result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use syneroym_app_orchestration::models::AppBlueprintId;

    use super::*;

    /// `Submit`'s handler refuses whenever `compile` returns more than one
    /// plan -- the same condition a Spawn dependency produces (`compiled.
    /// plans.last()` would silently drop every plan but the parent's).
    /// Exercises the real compiler, not a stub, so a future compiler change
    /// that stops producing multiple plans for Spawn would be caught here
    /// rather than only in `Submit`'s own dead refusal branch.
    #[tokio::test]
    async fn a_spawn_dependency_compiles_to_more_than_one_plan() {
        let root_toml = r#"
            id = "syneroym:root-app"
            version = "1.0.0"

            [services.web]
            service_type = "wasm"
            source = "web.wasm"

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
        "#;
        let db_toml = r#"
            id = "syneroym:db-app"
            version = "2.0.0"

            [services.postgres]
            service_type = "container"
            source = "postgres:latest"
        "#;
        let root_manifest = SynAppManifest::from_toml(root_toml).unwrap();
        let db_manifest = SynAppManifest::from_toml(db_toml).unwrap();
        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:db-app"), db_manifest);

        let compiled =
            compile(AppInstanceId::new("inst-1"), &root_manifest, &catalog).await.unwrap();

        assert!(
            compiled.plans.len() > 1,
            "a Spawn dependency must compile to more than one plan, which is what `Submit` \
             refuses on"
        );
    }
}

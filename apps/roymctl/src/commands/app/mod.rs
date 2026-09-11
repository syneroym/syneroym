//! SynApp management subcommands.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Subcommand;
use syneroym_app_orchestration::{models::SubstrateAlias, substrate_inventory::SubstrateEntry};

pub mod deploy;
pub mod health;
pub mod reconcile;
pub mod resolve;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use std::{collections::BTreeMap, sync::Arc};

#[cfg(test)]
pub(crate) use deploy::{
    check_no_placement_change, plan_declares_a_schedule, refuse_unmastered_dependencies,
};
#[cfg(test)]
pub(crate) use semver::Version;
#[cfg(test)]
pub(crate) use syneroym_app_orchestration::{
    ActionRecord, ActionState, AppInstanceId, DeploymentJournal, DeploymentPlan, DeploymentState,
    models::{
        AppBlueprintId, LogicalServiceName, LogicalServiceRef, PlannedService, ServiceConfig,
        ServiceType,
    },
};
#[cfg(test)]
pub(crate) use syneroym_sdk::deploy::DeployTarget;

#[cfg(test)]
pub(crate) use crate::commands::member_identity;

/// How long `deploy` and `health` each wait for a substrate to report ready
/// before giving up on it -- shared so the two commands agree on one budget
/// for the same kind of wait.
pub(super) const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolves a possibly-relative path against `dir` (`<roymctl --dir>`),
/// matching how `client_for` already resolves `identities/<name>.key` --
/// an inventory entry's `ucan` path should behave the same way.
pub(crate) fn resolve_under(dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { dir.join(path) }
}

/// Resolves the `identity`/`ucan` pair an alias's client presents.
///
/// The pair is inherited from **one** source, entry or global, never mixed
/// field-by-field: an entry that sets `identity` but not `ucan` would
/// otherwise fall back to the *global* `--ucan`, connecting as the entry's
/// identity while presenting a token whose `audience_did` is the global
/// one. `client_for`'s own guard only rejects "ucan without as", not this,
/// and the mismatch then fails silently server-side (a `warn!`-logged chain
/// drop), surfacing downstream as a confusing "holds no grant" instead of
/// the real cause -- the exact failure that guard was written to prevent.
///
/// Shared by `deploy` and `health`: both resolve a substrate alias's
/// credentials from the same inventory the same way.
pub(crate) fn resolve_credentials<'a>(
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
        /// anchor at. Ignored when `--mint-masters` is absent.
        /// Without it, a minted certificate is unusable on the wire until an
        /// anchor exists some other way (`roymctl identity publish-anchor`).
        #[arg(long)]
        registry_url: Option<String>,
        /// Substrate inventory mapping the aliases a manifest's `placement`
        /// selectors name to DIDs, addresses, credentials, and declared
        /// capabilities. Defaults to
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
    /// substrate is no longer refused -- the escape hatch for the
    /// placement-change refusal.
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
    /// report per-service health. Read-only: nothing is restarted,
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
    /// Resolve an app's logical service to its current member set
    /// (ADR-0022 §3): look the app DID up in the registry (Tier 1), fetch
    /// the signed topology document from the supervisor holding it (Tier 2),
    /// verify it against the app DID, and print the members. Prints the
    /// members as DIDs, not addresses -- turning one into an address is an
    /// ordinary registry lookup (Tier 3), unaffected by this command.
    Resolve {
        /// The app instance's own master DID, as Tier 1 answers with (not
        /// its human `AppInstanceId`).
        app_did: String,
        /// The logical service name within that app instance.
        service_name: String,
    },
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
            deploy::handle_deploy(
                instance_id.clone(),
                manifest_path.clone(),
                journal_path.clone(),
                *mint_masters,
                registry_url.clone(),
                inventory.clone(),
                api_url,
                substrate_opt,
                dir,
                run_as,
                ucan_path,
            )
            .await
        }
        AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
            reconcile::handle_reconcile(
                instance_id.clone(),
                manifest_path.clone(),
                journal_path.clone(),
            )
            .await
        }
        AppCommands::Forget { instance_id, service, journal_path } => {
            reconcile::handle_forget(instance_id.clone(), service.clone(), journal_path.clone())
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
            health::handle_health(
                instance_id.clone(),
                journal_path.clone(),
                alerts_path.clone(),
                inventory.clone(),
                *watch,
                *no_record,
                *strict,
                api_url,
                dir,
                run_as,
                ucan_path,
            )
            .await
        }
        AppCommands::Alerts { instance_id, alerts_path, journal_path, all } => {
            health::handle_alerts(
                instance_id.clone(),
                alerts_path.clone(),
                journal_path.clone(),
                *all,
            )
        }
        AppCommands::Resolve { app_did, service_name } => {
            resolve::handle_resolve(
                app_did.clone(),
                service_name.clone(),
                api_url,
                dir,
                run_as,
                ucan_path,
            )
            .await
        }
    }
}

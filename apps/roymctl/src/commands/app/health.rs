//! Health polling and alert inspection for deployed app instances.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use syneroym_app_orchestration::{
    AlertStore, AppInstanceId, DeploymentJournal, models::SubstrateAlias,
    substrate_inventory::SubstrateInventory,
};
use syneroym_sdk::{SubstrateStatus, deploy, health};

use super::{PREFLIGHT_TIMEOUT, resolve_credentials};
use crate::commands::member_identity;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_health(
    instance_id: String,
    journal_path: PathBuf,
    alerts_path: Option<PathBuf>,
    inventory: Option<PathBuf>,
    watch: Option<u64>,
    no_record: bool,
    strict: bool,
    api_url: &str,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
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
        match deploy::current_placement(&landed, &svc.member_ref().to_string()) {
            None => expected.push(health::ExpectedService {
                logical_ref: svc.logical_ref.clone(),
                service_id: String::new(),
                substrate_did: String::new(),
                member_index: svc.member_index,
            }),
            Some(row) => {
                // The plan's `service_id` is the compiler's
                // fabricated id whenever the deploy minted masters,
                // so re-derive.
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
                let (id, ucan) = resolve_credentials(a, entry, &inv_path, dir, run_as, ucan_path)?;
                (entry.api_url.clone().unwrap_or_else(|| api_url.to_string()), id, ucan)
            }
            _ => (api_url.to_string(), run_as, ucan_path.map(Path::to_path_buf)),
        };
        let client_result = crate::commands::client_for(
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
            health::HealthTarget { alias: alias.clone(), substrate_did: did.clone(), query },
        );
    }

    let mut report;
    loop {
        report = health::poll_once(&targets, &expected).await;
        print_health_table(&report);
        for u in report.unknowns() {
            eprintln!("undetermined: {} on {}: {:?}", u.logical_ref, u.substrate_did, u.signal);
        }
        if !no_record {
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
            Some(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
        }
    }

    // Faults are fatal; "cannot tell" is not, unless --strict.
    // A `tcp` service that declared no probe is
    // permanently undetermined, and must not make every routine
    // sweep exit non-zero. Reuses the loop's own last sweep rather
    // than polling again, so the exit code always agrees with what
    // was just printed and recorded.
    if !report.faults().is_empty() || (strict && !report.unknowns().is_empty()) {
        anyhow::bail!("{} service(s) unhealthy for {instance_id}", report.faults().len());
    }

    Ok(())
}

pub(super) fn handle_alerts(
    instance_id: String,
    alerts_path: Option<PathBuf>,
    journal_path: PathBuf,
    all: bool,
) -> anyhow::Result<()> {
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
    let rows = if all { alerts.all(&instance_id)? } else { alerts.active(&instance_id)? };
    if rows.is_empty() {
        println!("no {}alerts for {instance_id}", if all { "" } else { "active " });
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

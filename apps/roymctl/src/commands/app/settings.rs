//! The per-service mailbox and certificate lifetime in `app deploy`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use syneroym_app_orchestration::{
    AppInstanceId, DeploymentJournal, DeploymentPlan, MailboxChoice, SynAppManifest,
    models::{LogicalServiceRef, SubstrateAlias},
};
use syneroym_sdk::deploy::{self, MailboxDirectory, PinSource, PinSources};

use crate::commands::mailbox_auto::{self, HostRegistry};

/// Applies `app deploy --mailbox <service>=<mailbox>` to the manifest, before
/// it is compiled. The command line wins over the manifest. A name that is
/// not a service of the manifest is an error that lists the names.
pub(crate) fn apply_mailbox_overrides(
    manifest: &mut SynAppManifest,
    overrides: &[(String, MailboxChoice)],
) -> anyhow::Result<()> {
    for (name, choice) in overrides {
        let spec = manifest.services.iter_mut().find(|(n, _)| n.as_str() == name).map(|(_, s)| s);
        let Some(spec) = spec else {
            let known: Vec<&str> = manifest.services.keys().map(|n| n.as_str()).collect();
            anyhow::bail!(
                "--mailbox names service '{name}', which the manifest does not have (services: {})",
                known.join(", ")
            );
        };
        spec.mailbox = Some(choice.clone());
    }
    Ok(())
}

/// A mailbox needs a signed record and a certificate, and the certificate
/// lifetime is the lifetime of a certificate this command issues. Without
/// `--mint-masters` nothing signs or issues, so the value would be silently
/// unused. The check covers every compiled plan, not only the root manifest:
/// an app that the manifest spawns can set the values too.
pub(crate) fn refuse_settings_without_masters(
    plans: &[DeploymentPlan],
    mint_masters: bool,
) -> anyhow::Result<()> {
    if mint_masters {
        return Ok(());
    }
    for service in plans.iter().flat_map(|plan| &plan.services) {
        if service.mailbox.is_some() || service.instance_cert_expires_hours.is_some() {
            anyhow::bail!(
                "the manifest sets `mailbox` or `instance_cert_expires_hours` for '{}'; deploy \
                 with --mint-masters, or nothing would sign its record or issue its certificate",
                service.logical_ref
            );
        }
    }
    Ok(())
}

/// Refuses a `mailbox_methods` entry that names a node or platform
/// interface, in every compiled plan including spawned apps. The substrate
/// refuses it at deploy too; this fails before anything is placed.
pub(crate) fn refuse_reserved_mailbox_methods(plans: &[DeploymentPlan]) -> anyhow::Result<()> {
    for plan in plans {
        if let Err(errors) = syneroym_sdk::refuse_reserved_mailbox_methods(plan) {
            anyhow::bail!("{}", errors.join("; "));
        }
    }
    Ok(())
}

/// Pins every `auto` mailbox of `plan` before the journal records it, so a
/// re-run resumes the same record and keeps the same mailbox. A service that
/// had a mailbox in this instance's latest journal record keeps it. A new
/// choice must come from the registry its hosting node publishes to.
///
/// The plan still holds the compiler's made-up member ids here, so the
/// current records of the members cannot be read.
pub(crate) async fn pin_auto_mailboxes(
    plan: &mut DeploymentPlan,
    journal: &DeploymentJournal,
    directory: Option<&dyn MailboxDirectory>,
    list_registry: Option<&str>,
    host_registries: &BTreeMap<SubstrateAlias, (Option<String>, bool)>,
) -> anyhow::Result<()> {
    let previous = earlier_plan(journal, &plan.app_instance_id)?;
    let sources = PinSources { previous: previous.as_ref(), directory, members_are_masters: false };
    let pins = deploy::pin_auto_mailboxes(plan, sources)
        .await
        .context("app deploy reads the open mailboxes from --registry-url")?;
    for pin in &pins {
        if let (PinSource::List, Some(list_registry)) = (pin.source, list_registry) {
            check_every_host(plan, &pin.service, list_registry, host_registries)?;
        }
        println!("mailbox 'auto' for '{}': {} ({})", pin.service, pin.mailbox, pin.source.as_str());
    }
    Ok(())
}

/// The plan of the latest journal record of the instance, in any state. It is
/// the one place an earlier mailbox is read from, so `app deploy` and `app
/// reconcile` keep the same mailbox.
pub(crate) fn earlier_plan(
    journal: &DeploymentJournal,
    instance: &AppInstanceId,
) -> anyhow::Result<Option<DeploymentPlan>> {
    Ok(journal.get_latest(instance)?.map(|record| record.plan))
}

/// Checks the registry of every node that hosts a member of `service`, not
/// only the first: a member on another registry would lose its record for
/// the senders there. A member with no alias runs on the node the deploy is
/// aimed at, whose facts are not read here.
fn check_every_host(
    plan: &DeploymentPlan,
    service: &LogicalServiceRef,
    list_registry: &str,
    host_registries: &BTreeMap<SubstrateAlias, (Option<String>, bool)>,
) -> anyhow::Result<()> {
    let members = || plan.services.iter().filter(|s| &s.logical_ref == service);
    let aliases: BTreeSet<&SubstrateAlias> =
        members().filter_map(|s| s.substrate.as_ref()).collect();
    if members().any(|s| s.substrate.is_none()) {
        mailbox_auto::check_same_registry(
            &service.to_string(),
            list_registry,
            HostRegistry::Unknown,
        )?;
    }
    for alias in aliases {
        let registry = match host_registries.get(alias) {
            None => HostRegistry::Unknown,
            Some((None, _)) => HostRegistry::None,
            Some((Some(url), _)) => HostRegistry::Url(url),
        };
        mailbox_auto::check_same_registry(
            &format!("{service} on {alias}"),
            list_registry,
            registry,
        )?;
    }
    Ok(())
}

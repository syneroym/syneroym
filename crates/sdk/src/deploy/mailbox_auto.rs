//! `mailbox = "auto"` (ADR-0026 Decision 8): pick one of a registry's open
//! mailboxes, and keep that choice for every later deploy. A different
//! mailbox means a new signed record, and the messages that wait at the old
//! one are lost.

use std::{collections::BTreeSet, fmt};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures::future::join_all;
use reqwest::{Client, StatusCode};
use syneroym_app_orchestration::models::{
    DeploymentPlan, LogicalServiceRef, MailboxChoice, ServiceId,
};
use syneroym_core::{
    curated_list::{self, MailboxEntry, MailboxFilter},
    dht_registry::{EndpointMailbox, SignedEndpointInfo},
    registry_http,
};

/// The registry side of `auto`.
#[async_trait]
pub trait MailboxDirectory: fmt::Debug + Send + Sync {
    /// The registry's open mailboxes, unranked and unchecked.
    async fn open_mailboxes(&self) -> Result<Vec<MailboxEntry>>;

    /// The mailbox host the current record of `service_id` names, if that
    /// record verifies and is for `service_id`. `Ok(None)` when there is no
    /// record or it names no mailbox.
    async fn named_mailbox(&self, service_id: &str) -> Result<Option<ServiceId>>;
}

/// One registry, over HTTP.
#[derive(Debug)]
pub struct RegistryMailboxDirectory {
    client: Client,
    registry_url: String,
}

impl RegistryMailboxDirectory {
    #[must_use]
    pub fn new(registry_url: impl Into<String>) -> Self {
        Self { client: Client::new(), registry_url: registry_url.into() }
    }

    /// The registry this directory reads, as it was given.
    #[must_use]
    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }
}

#[async_trait]
impl MailboxDirectory for RegistryMailboxDirectory {
    async fn open_mailboxes(&self) -> Result<Vec<MailboxEntry>> {
        let list = curated_list::fetch::<MailboxEntry>(
            &self.client,
            &self.registry_url,
            &MailboxFilter {},
            0,
        )
        .await
        .with_context(|| format!("reading the open mailboxes of {}", self.registry_url))?;
        Ok(list.entries)
    }

    async fn named_mailbox(&self, service_id: &str) -> Result<Option<ServiceId>> {
        let base = curated_list::normalize_url(&self.registry_url)
            .with_context(|| format!("registry URL {:?}", self.registry_url))?;
        let response = self
            .client
            .get(format!("{base}/lookup/{service_id}"))
            .timeout(registry_http::HTTP_REQUEST_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("reading the current record of {service_id} at {base}"))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("registry {base} returned {status} for the current record of {service_id}");
        }
        let record = response.json::<SignedEndpointInfo>().await?;
        Ok(verified_mailbox(service_id, Some(&record))
            .and_then(|mailbox| ServiceId::try_new(mailbox.service_id.clone()).ok()))
    }
}

/// The mailbox `record` names, if it is a record of `service_id` with a good
/// signature. A wrong or hostile registry must not decide by lying. A record
/// past its `not_after` still counts: the mailbox is the choice of the
/// signer, and a deploy that keeps the earlier mailbox must not forget it
/// only because the record was not renewed.
#[must_use]
pub fn verified_mailbox<'a>(
    service_id: &str,
    record: Option<&'a SignedEndpointInfo>,
) -> Option<&'a EndpointMailbox> {
    record
        .filter(|r| r.verify_signature().is_ok() && r.info.service_id == service_id)
        .and_then(|r| r.info.mailbox.as_ref())
}

/// The first open mailbox for `chooser` by the rendezvous ranking, after bad
/// and duplicate entries are dropped. `None` when no entry is left.
#[must_use]
pub fn pick_open_mailbox(chooser: &str, entries: Vec<MailboxEntry>) -> Option<ServiceId> {
    curated_list::rank(chooser, entries)
        .into_iter()
        .find_map(|entry| ServiceId::try_new(entry.service_id).ok())
}

/// Where a pinned mailbox came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    /// The stored plan of an earlier deploy of this instance.
    StoredPlan,
    /// The current record of a member.
    CurrentRecord,
    /// The registry's list of open mailboxes.
    List,
}

impl PinSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StoredPlan => "stored plan",
            Self::CurrentRecord => "current record",
            Self::List => "list",
        }
    }
}

/// One `auto` that was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedMailbox {
    pub service: LogicalServiceRef,
    pub mailbox: ServiceId,
    pub source: PinSource,
}

/// What a deploy can read to pin `auto`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PinSources<'a> {
    /// The plan an earlier deploy of this instance stored.
    pub previous: Option<&'a DeploymentPlan>,
    /// The list registry. `None` when the deployer has none.
    pub directory: Option<&'a dyn MailboxDirectory>,
    /// Whether the plan's `service_id`s are the real member masters, so
    /// their current records can be read. `false` for a plan that still
    /// holds the compiler's made-up ids.
    pub members_are_masters: bool,
}

/// Replaces every `auto` in `plan` with the mailbox of an earlier deploy of
/// the same logical service in `previous`. No network. Returns what it
/// replaced; any other `auto` stays.
pub fn keep_earlier_mailboxes(
    plan: &mut DeploymentPlan,
    previous: Option<&DeploymentPlan>,
) -> Vec<PinnedMailbox> {
    let mut pins = Vec::new();
    for service in auto_services(plan) {
        if let Some(mailbox) = earlier_mailbox(previous, &service) {
            set_mailbox(plan, &service, &mailbox);
            pins.push(PinnedMailbox { service, mailbox, source: PinSource::StoredPlan });
        }
    }
    pins
}

/// Replaces every `auto` in `plan` with a node DID, the same one for every
/// member of a service: from the stored plan, else from a member's current
/// record, else from the list, which is read at most once.
///
/// # Errors
/// When a service needs the list and there is no directory, a registry
/// read fails, or the list has no usable entry.
pub async fn pin_auto_mailboxes(
    plan: &mut DeploymentPlan,
    sources: PinSources<'_>,
) -> Result<Vec<PinnedMailbox>> {
    let mut pins = keep_earlier_mailboxes(plan, sources.previous);
    let mut listed: Option<Vec<MailboxEntry>> = None;
    for service in auto_services(plan) {
        let directory = sources.directory.ok_or_else(|| {
            anyhow!(
                "'{service}' uses mailbox 'auto', but no registry is given to read the open \
                 mailboxes from"
            )
        })?;
        let pin = pin_from_registry(plan, &service, directory, &mut listed, sources).await?;
        set_mailbox(plan, &service, &pin.mailbox);
        pins.push(pin);
    }
    Ok(pins)
}

async fn pin_from_registry(
    plan: &DeploymentPlan,
    service: &LogicalServiceRef,
    directory: &dyn MailboxDirectory,
    listed: &mut Option<Vec<MailboxEntry>>,
    sources: PinSources<'_>,
) -> Result<PinnedMailbox> {
    if sources.members_are_masters
        && let Some(mailbox) = first_named_mailbox(plan, service, directory).await?
    {
        return Ok(PinnedMailbox {
            service: service.clone(),
            mailbox,
            source: PinSource::CurrentRecord,
        });
    }
    let entries = match listed {
        Some(entries) => entries.clone(),
        None => listed.insert(directory.open_mailboxes().await?).clone(),
    };
    let mailbox = pick_open_mailbox(&service.to_string(), entries).ok_or_else(|| {
        anyhow!(
            "'{service}' uses mailbox 'auto', but the registry lists no open mailbox; name the \
             mailbox host's node DID instead"
        )
    })?;
    Ok(PinnedMailbox { service: service.clone(), mailbox, source: PinSource::List })
}

/// The logical services whose mailbox is still `auto`.
fn auto_services(plan: &DeploymentPlan) -> BTreeSet<LogicalServiceRef> {
    plan.services
        .iter()
        .filter(|s| s.mailbox == Some(MailboxChoice::Auto))
        .map(|s| s.logical_ref.clone())
        .collect()
}

fn earlier_mailbox(
    previous: Option<&DeploymentPlan>,
    service: &LogicalServiceRef,
) -> Option<ServiceId> {
    previous?.services.iter().filter(|s| &s.logical_ref == service).find_map(|s| match &s.mailbox {
        Some(MailboxChoice::Service(host)) => Some(host.clone()),
        _ => None,
    })
}

async fn first_named_mailbox(
    plan: &DeploymentPlan,
    service: &LogicalServiceRef,
    directory: &dyn MailboxDirectory,
) -> Result<Option<ServiceId>> {
    let mut members: Vec<_> = plan.services.iter().filter(|s| &s.logical_ref == service).collect();
    members.sort_by_key(|s| s.member_index);
    // All members are asked at once, so a service with many members waits
    // for one lookup and not for the sum of them. The first member in order
    // that names a mailbox decides.
    let answers =
        join_all(members.iter().map(|member| directory.named_mailbox(member.service_id.as_str())))
            .await;
    for answer in answers {
        if let Some(host) = answer? {
            return Ok(Some(host));
        }
    }
    Ok(None)
}

fn set_mailbox(plan: &mut DeploymentPlan, service: &LogicalServiceRef, host: &ServiceId) {
    for member in plan.services.iter_mut().filter(|s| &s.logical_ref == service) {
        member.mailbox = Some(MailboxChoice::Service(host.clone()));
    }
}

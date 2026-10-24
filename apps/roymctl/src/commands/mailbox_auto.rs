//! `svc deploy --mailbox auto`: a single service has no stored plan, so the
//! mailbox its current record names is the earlier choice to keep. Also the
//! same-registry check that `app deploy` shares.

use anyhow::{Context, bail};
use syneroym_app_orchestration::{MailboxChoice, ServiceId};
use syneroym_core::curated_list;
use syneroym_sdk::deploy::{self, MailboxDirectory, RegistryMailboxDirectory};

#[cfg(test)]
mod tests;

/// What the deployer knows about the registry the hosting node publishes to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HostRegistry<'a> {
    /// Its node facts could not be read.
    Unknown,
    /// The node has no `substrate.registry_url`.
    None,
    Url(&'a str),
}

/// The mailbox host `choice` means for `svc_id`. For `auto`: the mailbox the
/// service's current record names, else the first open mailbox of
/// `registry_url` ranked for `svc_id`. `host_registry` is the hosting node's
/// `substrate.registry_url`, when its node facts could be read.
pub(crate) async fn resolve_svc_mailbox(
    choice: Option<&MailboxChoice>,
    svc_id: &str,
    registry_url: Option<&str>,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<Option<ServiceId>> {
    match choice {
        None => Ok(None),
        Some(MailboxChoice::Service(host)) => Ok(Some(host.clone())),
        Some(MailboxChoice::Auto) => {
            let url = registry_url.context(
                "--mailbox auto needs --registry-url: that registry lists the open mailboxes",
            )?;
            let directory = RegistryMailboxDirectory::new(url);
            pick_svc_mailbox(svc_id, &directory, url, host_registry).await.map(Some)
        }
    }
}

/// The testable half of [`resolve_svc_mailbox`]: no HTTP of its own.
pub(crate) async fn pick_svc_mailbox(
    svc_id: &str,
    directory: &dyn MailboxDirectory,
    list_registry: &str,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<ServiceId> {
    if let Some(host) = directory
        .named_mailbox(svc_id)
        .await
        .with_context(|| format!("reading the current record of {svc_id}"))?
    {
        println!("--mailbox auto keeps {host}, which the current record of {svc_id} names");
        return Ok(host);
    }
    check_same_registry(svc_id, list_registry, host_registry)?;
    let entries = directory.open_mailboxes().await?;
    let host = deploy::pick_open_mailbox(svc_id, entries).context(
        "--mailbox auto: the registry lists no open mailbox; give the mailbox host's node DID",
    )?;
    println!("--mailbox auto chose {host} from {list_registry}");
    Ok(host)
}

/// Refuses a new `auto` choice whose list registry is not the host registry:
/// the mailbox would keep the record alive in one registry while senders look
/// in the other.
pub(crate) fn check_same_registry(
    service: &str,
    list_registry: &str,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<()> {
    match host_registry {
        HostRegistry::Unknown => {
            eprintln!(
                "note: cannot check that '{service}' publishes its record to {list_registry} (the \
                 credential cannot read the node's facts); the mailbox works only if it does"
            );
            Ok(())
        }
        HostRegistry::None => bail!(
            "'{service}': the hosting node has no registry, so a mailbox from {list_registry} \
             cannot keep its record findable"
        ),
        HostRegistry::Url(host) => {
            if curated_list::normalize_url(host)? != curated_list::normalize_url(list_registry)? {
                bail!(
                    "'{service}' publishes its record to {host}, but mailbox 'auto' reads the \
                     list of {list_registry}; use one registry, or name the mailbox host's node \
                     DID"
                );
            }
            Ok(())
        }
    }
}

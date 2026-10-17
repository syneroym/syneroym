//! The per-service mailbox and certificate lifetime in `app deploy`.

use syneroym_app_orchestration::{MailboxChoice, SynAppManifest};

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
/// unused.
pub(crate) fn refuse_settings_without_masters(
    manifest: &SynAppManifest,
    mint_masters: bool,
) -> anyhow::Result<()> {
    if mint_masters {
        return Ok(());
    }
    for (name, spec) in &manifest.services {
        if spec.mailbox.is_some() || spec.instance_cert_expires_hours.is_some() {
            anyhow::bail!(
                "the manifest sets `mailbox` or `instance_cert_expires_hours` for '{name}'; \
                 deploy with --mint-masters, or nothing would sign its record or issue its \
                 certificate"
            );
        }
    }
    Ok(())
}

//! `--mailbox-passport` for `svc deploy` and `app deploy`: reading the
//! mailbox host's passport and signing the child tokens of one deploy.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use syneroym_app_orchestration::ServiceId;
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::deploy::MailboxPassport;

use super::identity;

/// The owner who signs the child tokens, and the passports they hold.
pub(crate) type OwnerPassports = (Identity, Vec<MailboxPassport>);

/// Reads each `--mailbox-passport` file and checks it for the `--as`
/// identity, which signs the child tokens and must be the passport's
/// audience. `None` when there are no files.
pub(crate) fn load_passports(
    dir: &Path,
    files: &[PathBuf],
    run_as: Option<&str>,
    now: u64,
) -> anyhow::Result<Option<OwnerPassports>> {
    if files.is_empty() {
        return Ok(None);
    }
    let Some(name) = run_as else {
        bail!("--mailbox-passport needs --as <owner>: the owner signs the child token");
    };
    let owner = Identity::load_from_path(identity::require_key_present(dir, name, "Identity")?)?;
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passports = files
        .iter()
        .map(|file| {
            let json = fs::read_to_string(file)
                .with_context(|| format!("could not read the passport {}", file.display()))?;
            MailboxPassport::parse(&json, &owner_did, now)
                .with_context(|| format!("--mailbox-passport {}", file.display()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some((owner, passports)))
}

/// The child token of `svc_id` for `svc deploy`: the one passport must be
/// for the mailbox the service uses.
pub(crate) fn child_token_for_service(
    passports: &OwnerPassports,
    mailbox: Option<&ServiceId>,
    svc_id: &str,
    now: u64,
) -> anyhow::Result<Option<String>> {
    let (owner, passports) = passports;
    let (Some(passport), Some(mailbox)) = (passports.first(), mailbox) else {
        return Ok(None);
    };
    ensure!(
        passport.host() == mailbox.as_str(),
        "the passport is for mailbox {}, but --mailbox is {mailbox}",
        passport.host()
    );
    passport.child_token_for(owner, svc_id, now).map(Some)
}

#[cfg(test)]
mod tests;

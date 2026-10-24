//! Mailbox passports at deploy: the owner signs one child token for each
//! member master whose service uses a mailbox that issued a passport.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use syneroym_app_orchestration::models::{DeploymentPlan, MailboxChoice, ServiceId};
use syneroym_identity::Identity;
use syneroym_passport::{CapabilityToken, MAILBOX_USE, child_token, inspect_child_token};

/// A mailbox passport read from a file, checked for one owner.
#[derive(Debug, Clone)]
pub struct MailboxPassport {
    token: CapabilityToken,
    host: String,
}

impl MailboxPassport {
    /// Parses `json` and checks that it is a passport (no proofs, one
    /// capability) for `mailbox/use` on `substrate:<did>/mailbox`, issued to
    /// `owner_did`, and not expired at `now`. The signature is checked when
    /// a child token is made from it.
    ///
    /// # Errors
    /// The passport does not parse, is for another use or owner, or has
    /// expired.
    pub fn parse(json: &str, owner_did: &str, now: u64) -> Result<Self> {
        let token: CapabilityToken =
            serde_json::from_str(json).context("the mailbox passport does not parse")?;
        ensure!(
            token.proofs.is_empty() && token.capabilities.len() == 1,
            "this file is not a mailbox passport: a passport has one capability and no proofs"
        );
        let capability = &token.capabilities[0];
        ensure!(
            capability.can.0 == MAILBOX_USE.ability,
            "this passport is for {}, not for {}",
            capability.can.0,
            MAILBOX_USE.ability
        );
        let host = MAILBOX_USE
            .host_of(&capability.with)
            .with_context(|| {
                format!(
                    "the passport resource {} is not substrate:<did>/mailbox",
                    capability.with.0
                )
            })?
            .to_string();
        ensure!(
            token.audience_did == owner_did,
            "this passport was issued to {}, not to the owner {owner_did}",
            token.audience_did
        );
        ensure!(now < token.expires_at_secs, "this mailbox passport has expired");
        Ok(Self { token, host })
    }

    /// The mailbox node DID the passport names.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The owner's child token for `master`, for the passport's whole life,
    /// as JSON. It is checked offline before it is returned, so a fault shows
    /// here and not at the node.
    ///
    /// # Errors
    /// `owner` is not the passport's audience, or the token does not verify.
    pub fn child_token_for(&self, owner: &Identity, master: &str, now: u64) -> Result<String> {
        let life = self.token.expires_at_secs.saturating_sub(now);
        let token = child_token(owner, &self.token, master, life, now)?;
        inspect_child_token(&token, &MAILBOX_USE, master, now)
            .map_err(|e| anyhow::anyhow!("the child token for {master} is refused: {e}"))?;
        Ok(serde_json::to_string(&token)?)
    }
}

/// One child token for each service of `plan` whose pinned mailbox has a
/// passport. Every passport must match at least one service, and no two
/// passports may name the same mailbox.
///
/// # Errors
/// Two passports for one mailbox, a passport that matches no service, a
/// service whose mailbox is still `auto`, or a child token that fails.
pub fn child_tokens_for(
    plan: &DeploymentPlan,
    owner: &Identity,
    passports: &[MailboxPassport],
    now: u64,
) -> Result<BTreeMap<ServiceId, String>> {
    let mut hosts = BTreeSet::new();
    for passport in passports {
        if !hosts.insert(passport.host()) {
            bail!("two --mailbox-passport files name the mailbox {}", passport.host());
        }
    }
    let mut out = BTreeMap::new();
    let mut used = BTreeSet::new();
    for svc in &plan.services {
        match &svc.mailbox {
            None => {}
            Some(MailboxChoice::Auto) => bail!(
                "service '{}' still has mailbox 'auto'; pin it (pin_auto_mailboxes) before \
                 signing its child token",
                svc.service_id
            ),
            Some(MailboxChoice::Service(host)) => {
                if let Some(p) = passports.iter().find(|p| p.host() == host.as_str()) {
                    out.insert(
                        svc.service_id.clone(),
                        p.child_token_for(owner, svc.service_id.as_str(), now)?,
                    );
                    used.insert(p.host());
                }
            }
        }
    }
    for passport in passports {
        ensure!(
            used.contains(passport.host()),
            "--mailbox-passport for {} matches no service that uses that mailbox",
            passport.host()
        );
    }
    Ok(out)
}

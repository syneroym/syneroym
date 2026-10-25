//! Mailbox passports for the end-to-end tests: the mailbox host's controller
//! issues them, a service owner holds them and signs one child token for
//! each member master.

#![allow(clippy::cognitive_complexity, dead_code)]

use serde_json::{Map, Value};
use syneroym_identity::Identity;
use syneroym_passport::{CapabilityToken, MAILBOX_USE, child_token, issue_passport, now_secs};

use super::mailbox_client::did_of;

/// One day: long enough for any test, short enough to read.
const LIFE_SECS: u64 = 24 * 3600;

pub struct PassportDesk {
    controller: Identity,
    owner: Identity,
    mailbox_did: String,
}

impl PassportDesk {
    /// `controller` is the verified controller of the mailbox host
    /// `mailbox_did`. The owner is a new identity.
    pub fn new(controller: &Identity, mailbox_did: &str) -> Self {
        Self {
            controller: Identity::from_bytes(&controller.to_bytes()),
            owner: Identity::generate().unwrap(),
            mailbox_did: mailbox_did.to_string(),
        }
    }

    /// As [`Self::new`], with `owner` as the service owner, for a test that
    /// deploys through `roymctl` as that identity.
    pub fn owned_by(controller: &Identity, owner: &Identity, mailbox_did: &str) -> Self {
        Self {
            controller: Identity::from_bytes(&controller.to_bytes()),
            owner: Identity::from_bytes(&owner.to_bytes()),
            mailbox_did: mailbox_did.to_string(),
        }
    }

    /// A passport for the owner, with these use limits.
    pub fn passport(&self, facts: &[(&str, u64)]) -> CapabilityToken {
        let limits: Map<String, Value> =
            facts.iter().map(|&(k, v)| (k.to_string(), Value::from(v))).collect();
        issue_passport(
            &self.controller,
            &self.mailbox_did,
            &MAILBOX_USE,
            &did_of(&self.owner),
            limits,
            LIFE_SECS,
            now_secs(),
        )
        .expect("the passport must be issued")
    }

    /// The owner's child token of `passport` for `master`, as JSON.
    pub fn child_token(&self, passport: &CapabilityToken, master: &str) -> String {
        let token = child_token(&self.owner, passport, master, LIFE_SECS, now_secs())
            .expect("the child token must be made");
        serde_json::to_string(&token).unwrap()
    }
}

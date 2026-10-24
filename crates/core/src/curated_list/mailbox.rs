//! The mailbox list: open mailboxes that a deployer picks from with `auto`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use syneroym_identity::substrate;

use crate::curated_list::ListEntry;

/// One entry of `[[roles.community_registry.mailboxes]]`, and of the
/// `GET /mailboxes` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEntry {
    /// The node DID of the mailbox host, which serves interface `mailbox`.
    /// The entry id.
    pub service_id: String,
    /// Who runs the mailbox. Shown to people; not checked.
    pub operator: String,
}

/// `GET /mailboxes` has no filter. A struct with no fields, not `()`: the
/// query always carries `hops`, and a unit type reads only an empty query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct MailboxFilter {}

impl ListEntry for MailboxEntry {
    const LIST_KEY: &'static str = "mailboxes";
    // A mailbox keeps a sleeping receiver's record alive only in its own
    // node's registry, and a registry lookup never asks the parent. A
    // mailbox from the parent's list would leave senders here without the
    // record once it expires.
    const FORWARDS_TO_PARENT: bool = false;
    type Filter = MailboxFilter;

    fn matches(&self, _filter: &MailboxFilter) -> bool {
        true
    }

    fn filter_pairs(_filter: &MailboxFilter) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    fn check(&mut self) -> anyhow::Result<String> {
        let key = substrate::resolve_did_key(&self.service_id)
            .with_context(|| format!("service_id {:?} is not a node DID", self.service_id))?;
        // The canonical spelling, so two spellings of one key are a duplicate.
        self.service_id = substrate::derive_did_key(&key);
        Ok(self.service_id.clone())
    }
}

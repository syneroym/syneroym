//! The coordinator list: open Iroh relays that a substrate may use as its
//! home relay.

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::curated_list::{self, ListEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoordinatorTransport {
    /// An Iroh relay.
    IrohRelay,
}

impl CoordinatorTransport {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IrohRelay => "iroh-relay",
        }
    }
}

/// One entry of `[[roles.community_registry.coordinators]]`, and of the
/// `GET /coordinators` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinatorEntry {
    pub transport: CoordinatorTransport,
    /// The relay URL. The entry id.
    pub url: String,
    /// Where `GET /v1/info` of the coordinator is served. It has its own port.
    pub info_url: String,
    /// Who runs the coordinator. Shown to people; not checked.
    pub operator: String,
}

/// `GET /coordinators?transport=iroh-relay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct CoordinatorFilter {
    pub transport: CoordinatorTransport,
}

impl ListEntry for CoordinatorEntry {
    const LIST_KEY: &'static str = "coordinators";
    type Filter = CoordinatorFilter;

    fn matches(&self, filter: &CoordinatorFilter) -> bool {
        self.transport == filter.transport
    }

    fn filter_pairs(filter: &CoordinatorFilter) -> Vec<(&'static str, String)> {
        vec![("transport", filter.transport.as_str().to_owned())]
    }

    fn check(&mut self) -> anyhow::Result<String> {
        self.url = curated_list::normalize_url(&self.url).context("url")?;
        self.info_url = curated_list::normalize_url(&self.info_url).context("info_url")?;
        Ok(self.url.clone())
    }
}

use serde::{Deserialize, Serialize};

use crate::{area::Area, directory::SearchHit, membership::MembershipEvidence};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishInput {
    pub envelope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishOutput {
    pub listing_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingInput {
    pub member_did: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOutput {
    #[serde(default)]
    pub issuer_did: Option<String>,
    pub member_did: String,
    #[serde(default)]
    pub evidence: MembershipEvidence,
    pub answered_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchOutput {
    #[serde(default)]
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub answered_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectoryInfoOutput {
    pub name: String,
    #[serde(default)]
    pub rules: String,
    #[serde(default)]
    pub area: Vec<Area>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub support_contact: String,
    #[serde(default)]
    pub dispute_path: String,
    pub retention_secs: u64,
    pub member_count: u64,
    #[serde(default)]
    pub issuer_did: Option<String>,
    #[serde(default)]
    pub signing_did: Option<String>,
}

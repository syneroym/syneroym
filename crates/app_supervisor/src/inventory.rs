//! The wire shape of `submission.inventory-json` (D-A5-7): alias -> `{did,
//! api-url, ucan}`. Distinct from `syneroym_app_orchestration::
//! SubstrateInventory`, whose `ucan` field is a **local file path** on the
//! operator's machine -- meaningless once the inventory crosses the wire to
//! a substrate that cannot read that path. `roymctl supervisor submit`
//! resolves each entry's credential file into the token itself before
//! sending.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use syneroym_rpc::CapabilityToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorInventoryEntry {
    pub did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// The supervisor's own credential against this substrate (§11.2):
    /// nothing issues it automatically, so `submit` refuses an alias that
    /// carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ucan: Option<CapabilityToken>,
}

pub type SupervisorInventory = BTreeMap<String, SupervisorInventoryEntry>;

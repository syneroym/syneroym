//! The settings of one passport use: home relay or dedicated mailbox.

use syneroym_ucan::{Ability, Capability, ResourceUri};

/// Reserved facts keys of ADR-0015 A10.
pub mod facts {
    /// `relay/home`: distinct node keys that may hold a lease.
    pub const MAX_NODES: &str = "max_nodes";
    /// `mailbox/use`: distinct member master DIDs that may hold a queue.
    pub const MAX_SERVICES: &str = "max_services";
    /// `mailbox/use`: bytes per queue.
    pub const MAX_BYTES: &str = "max_bytes";
    /// `mailbox/use`: messages per queue.
    pub const MAX_MESSAGES: &str = "max_messages";
    /// `mailbox/use`: longest message time-to-live, in seconds.
    pub const MAX_TTL_SECS: &str = "max_ttl_secs";
}

/// What one use of the passport fixes: its ability, its resource selector,
/// its signed use limits and how long one lease may last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassportUse {
    /// The exact ability a passport for this use must name.
    pub ability: &'static str,
    /// The selector after `substrate:<host node did>/`.
    pub selector: &'static str,
    /// The facts key that limits distinct subjects for one passport.
    pub count_key: &'static str,
    /// Every facts key a passport for this use may carry.
    pub limit_keys: &'static [&'static str],
    /// The subject limit when the passport has no `count_key`. `None`: no
    /// limit.
    pub open_count_limit: Option<u64>,
    /// The longest one lease may last.
    pub max_lease_secs: u64,
}

/// Use this relay as the home relay.
pub const RELAY_HOME: PassportUse = PassportUse {
    ability: Ability::RELAY_HOME,
    selector: "relay",
    count_key: facts::MAX_NODES,
    limit_keys: &[facts::MAX_NODES],
    open_count_limit: None,
    max_lease_secs: 24 * 60 * 60,
};

/// Keep a mailbox queue at this mailbox service.
pub const MAILBOX_USE: PassportUse = PassportUse {
    ability: Ability::MAILBOX_USE,
    selector: "mailbox",
    count_key: facts::MAX_SERVICES,
    limit_keys: &[facts::MAX_SERVICES, facts::MAX_BYTES, facts::MAX_MESSAGES, facts::MAX_TTL_SECS],
    open_count_limit: None,
    // The same as the mailbox queue lifetime, so a sleeping receiver keeps
    // its raised quota while its queue lives.
    max_lease_secs: 30 * 24 * 60 * 60,
};

impl PassportUse {
    /// `substrate:<host_node_did>/<selector>` (ADR-0015 A1 selector form).
    #[must_use]
    pub fn resource(&self, host_node_did: &str) -> ResourceUri {
        ResourceUri(format!("substrate:{host_node_did}/{}", self.selector))
    }

    /// The one capability a passport and its child token carry.
    #[must_use]
    pub fn capability(&self, host_node_did: &str) -> Capability {
        Capability {
            with: self.resource(host_node_did),
            can: Ability(self.ability.to_string()),
            caveats: None,
        }
    }

    /// Whether `cap` names exactly this use's ability and resource.
    /// Entailment is not accepted (ADR-0015 A10.1): `substrate/admin`, or a
    /// bare `substrate:<did>`, is never a passport. Caveats are not compared
    /// here; `verify_chain` already applies `can_delegate`.
    #[must_use]
    pub fn names_exactly(&self, cap: &Capability, host_node_did: &str) -> bool {
        cap.can.0 == self.ability && cap.with == self.resource(host_node_did)
    }

    /// The same use with another lease limit (a service may read it from
    /// its config).
    #[must_use]
    pub const fn with_max_lease_secs(mut self, secs: u64) -> Self {
        self.max_lease_secs = secs;
        self
    }
}

#[cfg(test)]
mod tests;

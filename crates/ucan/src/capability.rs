//! Capability types: resources, abilities, and grants (ADR-0015 §1).

use serde::{Deserialize, Serialize};

/// A resource a capability may authorize access to.
///
/// `synapp:<app_instance_id>:svc:<service_id>` for a service resource,
/// `substrate:<node_did>` for node-scoped authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUri(pub String);

impl ResourceUri {
    #[must_use]
    pub fn service(app_instance_id: &str, service_id: &str) -> Self {
        Self(format!("synapp:{app_instance_id}:svc:{service_id}"))
    }

    #[must_use]
    pub fn substrate(node_did: &str) -> Self {
        Self(format!("substrate:{node_did}"))
    }

    /// Whether this is a `substrate:<node_did>` node-scoped resource, as
    /// opposed to a `synapp:...:svc:...` service resource.
    ///
    /// Note: this checks the `substrate:` *prefix* only, not which node's
    /// DID follows it -- `covers`/`grants` therefore treat *any*
    /// `substrate:<node_did>` capability as a wildcard over all resources,
    /// including a `substrate:<other-node>` one. Inert at B1 (the only
    /// issuer of a substrate-scoped capability is this node's own admin
    /// root, naming this node's own DID -- see `covers`'s tests), but once
    /// multi-node/owner-rooted trust exists (Slice B7) this should also
    /// verify the named node DID matches the node actually evaluating the
    /// capability.
    #[must_use]
    pub fn is_substrate_scope(&self) -> bool {
        self.0.starts_with("substrate:")
    }
}

/// A `/`-delimited ability hierarchy string, e.g. `"data-layer/admin"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ability(pub String);

impl Ability {
    pub const APP_CONFIG_READ: &'static str = "app-config/read";
    pub const BLOB_READ: &'static str = "blob/read";
    pub const BLOB_SIGN_URL: &'static str = "blob/sign-url";
    pub const BLOB_WRITE: &'static str = "blob/write";
    pub const DATA_LAYER_ADMIN: &'static str = "data-layer/admin";
    pub const DATA_LAYER_READ: &'static str = "data-layer/read";
    pub const DATA_LAYER_WRITE: &'static str = "data-layer/write";
    pub const MESSAGING_PUBLISH: &'static str = "messaging/publish";
    pub const MESSAGING_SUBSCRIBE: &'static str = "messaging/subscribe";
    pub const SUBSTRATE_ADMIN: &'static str = "substrate/admin";
    pub const VAULT_REVEAL: &'static str = "vault/reveal";

    /// Whether `self` entails `other`: a parent ability grants everything a
    /// child ability grants. `substrate/admin` entails everything on the
    /// node. Within the `data-layer` namespace there is an explicit tiered
    /// hierarchy (`admin` ⊇ `write` ⊇ `read`); every other ability is flat
    /// (entails only itself) — escalation attempts (a lower tier, or an
    /// unrelated/sibling ability, claiming to entail a higher or different
    /// one) fail closed.
    #[must_use]
    pub fn entails(&self, other: &Self) -> bool {
        if self.0 == Self::SUBSTRATE_ADMIN {
            return true;
        }
        if self.0 == other.0 {
            return true;
        }
        match (Self::tier(&self.0), Self::tier(&other.0)) {
            (Some((ns1, rank1)), Some((ns2, rank2))) => ns1 == ns2 && rank1 > rank2,
            _ => false,
        }
    }

    /// Returns `(namespace, rank)` for abilities with an explicit tiered
    /// hierarchy; higher rank is more privileged. Abilities outside this
    /// table are flat (see `entails`).
    fn tier(ability: &str) -> Option<(&'static str, u8)> {
        match ability {
            Self::DATA_LAYER_READ => Some(("data-layer", 1)),
            Self::DATA_LAYER_WRITE => Some(("data-layer", 2)),
            Self::DATA_LAYER_ADMIN => Some(("data-layer", 3)),
            _ => None,
        }
    }
}

/// A single granted capability: `with` a resource, `can` an ability, subject
/// to optional passthrough `caveats`. **`caveats` are not evaluated by
/// `grants`/`covers` today** -- they pass through unread; a caveat-restricted
/// capability is currently treated identically to an unrestricted one.
/// Rich caveat evaluation is FDAE/M04B; until it lands, do not rely on
/// `caveats` to actually narrow a grant (see
/// `caveats_passthrough_is_not_yet_enforced`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub with: ResourceUri,
    pub can: Ability,
    pub caveats: Option<serde_json::Value>,
}

impl Capability {
    /// Whether this capability grants `(resource, ability)`. A node-scoped
    /// (`substrate:<node_did>`) grant authorizes any resource on this node,
    /// per ADR-0015 §1 ("`substrate/admin` ⊇ everything on that node");
    /// otherwise the resource must match exactly. Either way `self.can` must
    /// entail `ability`.
    #[must_use]
    pub fn grants(&self, resource: &ResourceUri, ability: &Ability) -> bool {
        if self.with.is_substrate_scope() {
            return self.can.entails(ability);
        }
        self.with == *resource && self.can.entails(ability)
    }

    /// Whether `self` (a held/parent capability) authorizes everything
    /// `other` (a requested/child capability) asks for. A `substrate:`-scoped
    /// `self` covers any resource; otherwise resources must match exactly.
    /// This is the same resource-scope rule `grants` inlines, factored out so
    /// UCAN chain attenuation (B1) can reuse it directly.
    #[must_use]
    pub fn covers(&self, other: &Capability) -> bool {
        (self.with.is_substrate_scope() || self.with == other.with) && self.can.entails(&other.can)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ability(s: &str) -> Ability {
        Ability(s.to_string())
    }

    #[test]
    fn data_layer_admin_entails_write_and_read() {
        let admin = ability(Ability::DATA_LAYER_ADMIN);
        assert!(admin.entails(&ability(Ability::DATA_LAYER_WRITE)));
        assert!(admin.entails(&ability(Ability::DATA_LAYER_READ)));
        assert!(admin.entails(&admin));
    }

    #[test]
    fn data_layer_write_does_not_entail_admin() {
        let write = ability(Ability::DATA_LAYER_WRITE);
        assert!(!write.entails(&ability(Ability::DATA_LAYER_ADMIN)));
    }

    #[test]
    fn read_does_not_entail_write() {
        let read = ability(Ability::DATA_LAYER_READ);
        assert!(!read.entails(&ability(Ability::DATA_LAYER_WRITE)));
    }

    #[test]
    fn sibling_abilities_do_not_entail() {
        let blob_read = ability(Ability::BLOB_READ);
        assert!(!blob_read.entails(&ability(Ability::DATA_LAYER_READ)));
        assert!(
            !ability(Ability::MESSAGING_PUBLISH).entails(&ability(Ability::MESSAGING_SUBSCRIBE))
        );
    }

    #[test]
    fn substrate_admin_entails_everything() {
        let root = ability(Ability::SUBSTRATE_ADMIN);
        assert!(root.entails(&ability(Ability::DATA_LAYER_ADMIN)));
        assert!(root.entails(&ability(Ability::VAULT_REVEAL)));
        assert!(root.entails(&ability(Ability::BLOB_SIGN_URL)));
        assert!(root.entails(&root));
    }

    #[test]
    fn nothing_entails_substrate_admin_except_itself() {
        let root = ability(Ability::SUBSTRATE_ADMIN);
        assert!(!ability(Ability::DATA_LAYER_ADMIN).entails(&root));
    }

    #[test]
    fn capability_denies_on_resource_mismatch() {
        let cap = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: None,
        };
        assert!(
            !cap.grants(
                &ResourceUri::service("app-1", "svc-b"),
                &ability(Ability::DATA_LAYER_READ)
            )
        );
    }

    #[test]
    fn substrate_scoped_capability_grants_any_resource_on_the_node() {
        let cap = Capability {
            with: ResourceUri::substrate("did:key:z6MkAdminRoot"),
            can: ability(Ability::SUBSTRATE_ADMIN),
            caveats: None,
        };
        assert!(
            cap.grants(
                &ResourceUri::service("app-1", "svc-a"),
                &ability(Ability::DATA_LAYER_ADMIN)
            )
        );
        assert!(
            cap.grants(&ResourceUri::service("app-2", "svc-b"), &ability(Ability::VAULT_REVEAL))
        );
    }

    #[test]
    fn capability_grants_entailed_ability_on_matching_resource() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let cap = Capability {
            with: resource.clone(),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: None,
        };
        assert!(cap.grants(&resource, &ability(Ability::DATA_LAYER_WRITE)));
        assert!(cap.grants(&resource, &ability(Ability::DATA_LAYER_READ)));
    }

    #[test]
    fn capability_denies_escalation_beyond_its_ability() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let cap = Capability {
            with: resource.clone(),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        assert!(!cap.grants(&resource, &ability(Ability::DATA_LAYER_WRITE)));
        assert!(!cap.grants(&resource, &ability(Ability::DATA_LAYER_ADMIN)));
    }

    #[test]
    fn covers_holds_for_matching_resource_and_entailed_ability() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let parent = Capability {
            with: resource.clone(),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: None,
        };
        let child =
            Capability { with: resource, can: ability(Ability::DATA_LAYER_WRITE), caveats: None };
        assert!(parent.covers(&child));
    }

    #[test]
    fn covers_denies_escalation_and_resource_mismatch() {
        let parent = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        let escalated = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_WRITE),
            caveats: None,
        };
        assert!(!parent.covers(&escalated));

        let admin_wrong_resource = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: None,
        };
        let other_resource = Capability {
            with: ResourceUri::service("app-1", "svc-b"),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        assert!(!admin_wrong_resource.covers(&other_resource));
    }

    #[test]
    fn covers_substrate_scope_covers_any_resource() {
        let parent = Capability {
            with: ResourceUri::substrate("did:key:z6MkAdminRoot"),
            can: ability(Ability::SUBSTRATE_ADMIN),
            caveats: None,
        };
        let child = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: None,
        };
        assert!(parent.covers(&child));
    }

    #[test]
    fn resource_uri_helpers_format_as_expected() {
        assert_eq!(ResourceUri::service("app-1", "svc-a").0, "synapp:app-1:svc:svc-a");
        assert_eq!(ResourceUri::substrate("did:key:z6Mk").0, "substrate:did:key:z6Mk");
    }

    /// Pins the current (documented, not yet implemented) passthrough
    /// behavior: a caveat on either side of `grants`/`covers` is completely
    /// ignored -- a caveat-restricted capability behaves exactly like an
    /// unrestricted one. Rich caveat evaluation belongs to FDAE/M04B; this
    /// test exists so that gap isn't silently forgotten once caveats gain
    /// real meaning.
    #[test]
    fn caveats_passthrough_is_not_yet_enforced() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let restricted = Capability {
            with: resource.clone(),
            can: ability(Ability::DATA_LAYER_ADMIN),
            caveats: Some(serde_json::json!({"rows": "id = 1"})),
        };
        let unrestricted_child = Capability {
            with: resource.clone(),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        // A caveat on the *parent* does not narrow what it covers today.
        assert!(restricted.covers(&unrestricted_child));
        assert!(restricted.grants(&resource, &ability(Ability::DATA_LAYER_READ)));

        // A caveat on the *child*/*request* side is equally inert: same
        // resource and ability as `unrestricted_child`, only `caveats`
        // differs, and `covers` still holds.
        let restricted_request = Capability {
            with: resource,
            can: ability(Ability::DATA_LAYER_READ),
            caveats: Some(serde_json::json!({"rows": "id = 1"})),
        };
        assert!(unrestricted_child.covers(&restricted_request));
    }

    /// Pins the current, deliberately permissive `is_substrate_scope`
    /// behavior: it does not check *which* node's DID the `substrate:`
    /// resource names, so a capability scoped to a *different* node's
    /// substrate resource still covers/grants everything, exactly like one
    /// scoped to this node's own DID. At B1 this is inert (the only issuer
    /// of a substrate-scoped capability is this node's own admin root,
    /// naming its own DID), but once multi-node/owner-rooted trust exists
    /// (Slice B7) `covers`/`grants` should also verify the resource's node
    /// DID matches the node evaluating the capability.
    #[test]
    fn substrate_scope_does_not_check_which_node_it_names() {
        let other_nodes_admin = Capability {
            with: ResourceUri::substrate("did:key:z6MkSomeOtherNode"),
            can: ability(Ability::SUBSTRATE_ADMIN),
            caveats: None,
        };
        assert!(
            other_nodes_admin.grants(
                &ResourceUri::service("app-1", "svc-a"),
                &ability(Ability::DATA_LAYER_ADMIN)
            )
        );
    }
}

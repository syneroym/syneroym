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

    /// Whether this is a **bare** `substrate:<node_did>` node-scoped
    /// resource -- node-wide authority (ADR-0015 §1, "`substrate/admin` ⊇
    /// everything on that node") -- as opposed to a `synapp:...:svc:...`
    /// service resource or a *selector-bearing* `substrate:<node_did>/
    /// <selector>` one (ADR-0015 A1, M04A Slice B7b/F2). A selector-bearing
    /// `substrate:` resource (e.g. `orchestrator`'s
    /// `substrate:<node>/app/<name>`) names a specific resource, not
    /// node-wide authority, and must prefix-match like any other --
    /// `covers`/`grants` fall through to `covers_resource` for it.
    ///
    /// Note: this checks the `substrate:` *prefix* only, not which node's
    /// DID follows it -- `covers`/`grants` therefore treat *any* **bare**
    /// `substrate:<node_did>` capability as a wildcard over all resources,
    /// including a `substrate:<other-node>` one. Inert at B1 (the only
    /// issuer of a substrate-scoped capability is this node's own admin
    /// root, naming its own DID -- see `covers`'s tests); at B7b this is
    /// exactly the shape `build_caller` still issues for the F4 unowned
    /// posture and a real owner's `substrate/admin`, so it stays inert
    /// there too. The node-locality check ADR-0015 A6/F6 calls for lives in
    /// the *chain-rooting* predicate (`ChainVerifyOpts::is_trusted_root`,
    /// `crates/router/src/route_handler/io.rs`'s `resource_is_local`), not
    /// here -- threading `local_node_did` through `Capability::grants`/
    /// `covers` would touch every call site and every test for a check that
    /// belongs one layer up, where the evaluating node's own DID is already
    /// known.
    #[must_use]
    pub fn is_substrate_scope(&self) -> bool {
        self.0.starts_with("substrate:") && !self.has_selector()
    }

    /// The `[/<selector>]` tail introduced by ADR-0015 A1, if any. The base
    /// is `synapp:<app>:svc:<svc>` or `substrate:<node_did>`; a selector is
    /// interface-shaped (`collection/<name>[/<id>]`, `blob/<prefix>`,
    /// `topic/<pattern>`, `rpc/<method>`, `orchestrator`'s `app/<name>`).
    ///
    /// Parses by structure, not by splitting the whole string on `:` --
    /// `substrate:<node_did>` and `synapp:<app>:svc:<svc>` both embed a
    /// `did:key:z...` value that itself contains `:`. Neither an app/service
    /// name nor a `did:key` encoding contains `/`, so the base never does
    /// either; the first `/` in the whole string is therefore always the
    /// selector boundary.
    fn split_selector(&self) -> (&str, Option<&str>) {
        match self.0.find('/') {
            Some(idx) => (&self.0[..idx], Some(&self.0[idx + 1..])),
            None => (&self.0[..], None),
        }
    }

    /// Whether this resource carries a `[/<selector>]` tail (ADR-0015 A1).
    #[must_use]
    fn has_selector(&self) -> bool {
        self.split_selector().1.is_some()
    }

    /// Segment-wise prefix cover (ADR-0015 A1): bases must be equal, and
    /// `self`'s selector segments must be a prefix of `other`'s. No selector
    /// at all on `self` covers every selector on that base (a
    /// service-granularity grant keeps meaning what it means today); a
    /// selector on `self` but none on `other` is **not** covered (a
    /// selector-restricted grant does not cover the broader, unrestricted
    /// resource). A trailing `/` or a `*` segment is a prefix wildcard,
    /// matching whole segments only, never a partial string (M04A Slice
    /// B7b/F8) -- `app/acme-` does **not** cover `app/acme-evil`.
    #[must_use]
    pub fn covers_resource(&self, other: &Self) -> bool {
        let (self_base, self_selector) = self.split_selector();
        let (other_base, other_selector) = other.split_selector();
        if self_base != other_base {
            return false;
        }
        let Some(self_selector) = self_selector else {
            return true;
        };
        let Some(other_selector) = other_selector else {
            return false;
        };
        let self_segments: Vec<&str> =
            self_selector.trim_end_matches('/').split('/').filter(|s| !s.is_empty()).collect();
        let other_segments: Vec<&str> = other_selector.split('/').collect();
        self_segments.len() <= other_segments.len()
            && self_segments.iter().zip(other_segments.iter()).all(|(s, o)| *s == "*" || s == o)
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
    /// M04A Slice B7a/B7b: deploy/undeploy/status-check on the orchestrator
    /// interface. Flat -- each entails only itself (§6 Q6: no `tier` entry),
    /// so "deploy but not undeploy" stays expressible.
    pub const ORCHESTRATOR_DEPLOY: &'static str = "orchestrator/deploy";
    pub const ORCHESTRATOR_STATUS: &'static str = "orchestrator/status";
    pub const ORCHESTRATOR_UNDEPLOY: &'static str = "orchestrator/undeploy";
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
    /// Whether this capability grants `(resource, ability)`. A **bare**
    /// node-scoped (`substrate:<node_did>`, no selector) grant authorizes any
    /// resource on this node, per ADR-0015 §1 ("`substrate/admin` ⊇
    /// everything on that node"); otherwise the resource must be covered by
    /// `self.with` (ADR-0015 A1's segment-wise prefix cover, M04A Slice
    /// B7b/F2). Either way `self.can` must entail `ability`.
    #[must_use]
    pub fn grants(&self, resource: &ResourceUri, ability: &Ability) -> bool {
        if self.with.is_substrate_scope() {
            return self.can.entails(ability);
        }
        self.with.covers_resource(resource) && self.can.entails(ability)
    }

    /// Whether `self` (a held/parent capability) authorizes everything
    /// `other` (a requested/child capability) asks for. A bare
    /// `substrate:`-scoped `self` covers any resource; otherwise `self.with`
    /// must cover `other.with` (the same prefix-cover rule `grants` uses,
    /// factored out so UCAN chain attenuation (B1) can reuse it directly).
    #[must_use]
    pub fn covers(&self, other: &Capability) -> bool {
        (self.with.is_substrate_scope() || self.with.covers_resource(&other.with))
            && self.can.entails(&other.can)
    }

    /// ADR-0015 A3: whether this capability may be further delegated.
    /// Absent ⇒ `true` (B1's behavior, before caveats were evaluated at all).
    /// Once `false`, terminal along a chain -- composition is a *check*, not
    /// a conjunction (`token::granted_capabilities` is the one place a
    /// parent capability backs a child's; it requires `pc.can_delegate()`
    /// there). This is the one caveat B7b evaluates; `where`/`fields` (A3's
    /// other two forms) remain unevaluated passthrough, same as before --
    /// see `caveats_passthrough_is_not_yet_enforced`.
    #[must_use]
    pub fn can_delegate(&self) -> bool {
        self.caveats
            .as_ref()
            .and_then(|c| c.get("can_delegate"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
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
    /// behavior for `where`/`fields`-shaped caveats: a caveat on either side
    /// of `grants`/`covers` is completely ignored -- a caveat-restricted
    /// capability behaves exactly like an unrestricted one. Rich `where`/
    /// `fields` caveat evaluation belongs to FDAE/M04B; this test exists so
    /// that gap isn't silently forgotten once caveats gain real meaning.
    ///
    /// **Narrowed at M04A Slice B7b (ADR-0015 A3):** `can_delegate` is no
    /// longer part of this passthrough -- it is now evaluated (`Capability::
    /// can_delegate`, checked by `token::granted_capabilities` at
    /// attenuation time, not by `grants`/`covers` here). See
    /// `token.rs`'s `can_delegate_false_blocks_further_delegation` for that
    /// behavior. This test uses an unrelated caveat key (`rows`) so it stays
    /// a clean pin of what is still unenforced.
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
    /// behavior: it does not check *which* node's DID the *bare*
    /// `substrate:` resource names, so a capability scoped to a *different*
    /// node's substrate resource still covers/grants everything, exactly
    /// like one scoped to this node's own DID. At B1 this was inert (the
    /// only issuer of a substrate-scoped capability was this node's own
    /// admin root, naming its own DID).
    ///
    /// **M04A Slice B7b (F6): still true here, on purpose.** The plan's
    /// resolution does *not* thread `local_node_did` through
    /// `Capability::grants`/`covers` -- doing so would touch every call site
    /// and every other test in this module for a check that belongs one
    /// layer up, at the *chain-rooting* predicate
    /// (`ChainVerifyOpts::is_trusted_root`, evaluated in
    /// `crates/router/src/route_handler/io.rs`'s `build_caller`), which
    /// already knows the evaluating node's own DID and is where an
    /// owner-rooted chain's resource is checked for locality
    /// (`resource_is_local`, pinned by that module's own tests). This
    /// module's `grants`/`covers` remain node-DID-agnostic by design.
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

    // -- M04A Slice B7b: ADR-0015 A1 selectors / F2 / F8 ---------------

    fn substrate_app(node_did: &str, selector: &str) -> ResourceUri {
        ResourceUri(format!("substrate:{node_did}/{selector}"))
    }

    /// F2 -- the test that would have caught the wildcard bug the plan
    /// documents: a capability scoped to one app's selector must not grant
    /// on a *different* app's selector, even though both share the
    /// `substrate:` prefix.
    #[test]
    fn selector_scoped_substrate_capability_does_not_grant_a_different_app() {
        let cap = Capability {
            with: substrate_app("did:key:zNode", "app/foo"),
            can: ability(Ability::ORCHESTRATOR_DEPLOY),
            caveats: None,
        };
        assert!(cap.grants(
            &substrate_app("did:key:zNode", "app/foo"),
            &ability(Ability::ORCHESTRATOR_DEPLOY)
        ));
        assert!(!cap.grants(
            &substrate_app("did:key:zNode", "app/bar"),
            &ability(Ability::ORCHESTRATOR_DEPLOY)
        ));
    }

    /// `app/*` (whole-segment wildcard) covers every sibling app selector.
    #[test]
    fn wildcard_selector_covers_every_app() {
        let cap = Capability {
            with: substrate_app("did:key:zNode", "app/*"),
            can: ability(Ability::ORCHESTRATOR_DEPLOY),
            caveats: None,
        };
        for app in ["foo", "bar", "acme-shop"] {
            assert!(
                cap.grants(
                    &substrate_app("did:key:zNode", &format!("app/{app}")),
                    &ability(Ability::ORCHESTRATOR_DEPLOY)
                ),
                "app/* must cover app/{app}"
            );
        }
    }

    /// F8 -- `*` is a whole-segment wildcard only, never a partial-string
    /// prefix: `app/acme-` must not cover `app/acme-evil`.
    #[test]
    fn wildcard_is_whole_segment_only_not_a_string_prefix() {
        let cap = Capability {
            with: substrate_app("did:key:zNode", "app/acme-"),
            can: ability(Ability::ORCHESTRATOR_DEPLOY),
            caveats: None,
        };
        assert!(!cap.grants(
            &substrate_app("did:key:zNode", "app/acme-evil"),
            &ability(Ability::ORCHESTRATOR_DEPLOY)
        ));
    }

    /// A selector-less `synapp:` capability covers every selector on that
    /// base (service-granularity grants keep meaning what they mean today).
    #[test]
    fn no_selector_covers_every_selector_on_the_same_base() {
        let cap = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        let selectored =
            ResourceUri(format!("{}/collection/orders", ResourceUri::service("app-1", "svc-a").0));
        assert!(cap.grants(&selectored, &ability(Ability::DATA_LAYER_READ)));
    }

    /// The inverse: a selector-scoped capability does not cover the broader,
    /// selector-less base -- a narrower grant cannot stand in for the whole
    /// resource.
    #[test]
    fn selector_scoped_capability_does_not_cover_the_bare_base() {
        let base = ResourceUri::service("app-1", "svc-a");
        let selectored = ResourceUri(format!("{}/collection/orders", base.0));
        let cap =
            Capability { with: selectored, can: ability(Ability::DATA_LAYER_READ), caveats: None };
        assert!(!cap.grants(&base, &ability(Ability::DATA_LAYER_READ)));
    }

    /// A prefix selector (no wildcard, no trailing `/`) still covers deeper
    /// segments beneath it -- segment-wise prefix cover, not exact match.
    #[test]
    fn selector_prefix_covers_deeper_segments() {
        let cap = Capability {
            with: ResourceUri(format!(
                "{}/collection/orders",
                ResourceUri::service("app-1", "svc-a").0
            )),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        let deeper = ResourceUri(format!(
            "{}/collection/orders/123",
            ResourceUri::service("app-1", "svc-a").0
        ));
        assert!(cap.grants(&deeper, &ability(Ability::DATA_LAYER_READ)));
    }

    // -- ADR-0015 A3: can_delegate --------------------------------------

    #[test]
    fn can_delegate_absent_defaults_to_true() {
        let cap = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: None,
        };
        assert!(cap.can_delegate());
    }

    #[test]
    fn can_delegate_false_is_read_from_caveats() {
        let cap = Capability {
            with: ResourceUri::service("app-1", "svc-a"),
            can: ability(Ability::DATA_LAYER_READ),
            caveats: Some(serde_json::json!({"can_delegate": false})),
        };
        assert!(!cap.can_delegate());
    }
}

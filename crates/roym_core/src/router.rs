//! Method prefix routing table.

use crate::services::{CATALOG, CONVERSATION, DIRECTORY, PROFILE, Service, TRANSACTION};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodAuth {
    /// Reachable with no session. Nothing here reads or writes anything
    /// belonging to a person.
    Public,
    /// Requires a verified person session whose subject is the DID this
    /// installation is recorded as belonging to.
    Owner,
}

/// JSON-RPC method prefix -> (sibling that owns it, auth level). The spec's own
/// API column, made executable. No default arm: an unlisted prefix is
/// `-32601`, because "which service owns this?" is a question with a
/// written answer or no answer.
///
/// `session.whoami` is handled directly by `web` and is not routed to a
/// sibling.
const ROUTES: &[(&str, Service, MethodAuth)] = &[
    ("conversation.", CONVERSATION, MethodAuth::Owner),
    ("profile.", PROFILE, MethodAuth::Owner),
    ("contacts.", PROFILE, MethodAuth::Owner),
    ("block.", PROFILE, MethodAuth::Owner),
    ("report.", PROFILE, MethodAuth::Owner),
    ("listing.", CATALOG, MethodAuth::Owner),
    ("availability.", CATALOG, MethodAuth::Owner),
    ("request.", TRANSACTION, MethodAuth::Owner),
    ("quote.", TRANSACTION, MethodAuth::Owner),
    ("agreement.", TRANSACTION, MethodAuth::Owner),
    ("receipt.", TRANSACTION, MethodAuth::Owner),
    ("directory.", DIRECTORY, MethodAuth::Owner),
];

/// Methods a person may reach before signing in. Full method names, never
/// prefixes: an exception granted to a prefix is an exception granted to
/// methods nobody has written yet.
const PUBLIC_METHODS: &[&str] = &["profile.policy"];

pub fn route(method: &str) -> Option<Service> {
    ROUTES.iter().find(|(p, _, _)| method.starts_with(p)).map(|(_, s, _)| *s)
}

pub fn method_auth(method: &str) -> Option<MethodAuth> {
    if PUBLIC_METHODS.contains(&method) {
        return Some(MethodAuth::Public);
    }
    ROUTES.iter().find(|(p, _, _)| method.starts_with(p)).map(|(_, _, a)| *a)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::services::SIBLINGS;

    #[test]
    fn every_prefix_maps_to_a_service_in_siblings() {
        for &(prefix, service, _) in ROUTES {
            assert!(
                SIBLINGS.contains(&service),
                "Prefix '{prefix}' mapped to service '{}' which is not in SIBLINGS",
                service.name
            );
        }
    }

    #[test]
    fn no_prefix_is_a_prefix_of_another() {
        for (i, &(p1, _, _)) in ROUTES.iter().enumerate() {
            for (j, &(p2, _, _)) in ROUTES.iter().enumerate() {
                if i != j {
                    assert!(
                        !p1.starts_with(p2),
                        "Prefix '{p1}' is a prefix of or shadowed by prefix '{p2}'"
                    );
                }
            }
        }
    }

    #[test]
    fn unlisted_method_returns_none() {
        assert_eq!(route("unknown.method"), None);
        assert_eq!(route("session.whoami"), None);
        assert_eq!(route(""), None);
    }

    #[test]
    fn reachable_services_set_equals_siblings() {
        let reachable: HashSet<&'static str> = ROUTES.iter().map(|(_, s, _)| s.name).collect();
        let expected: HashSet<&'static str> = SIBLINGS.iter().map(|s| s.name).collect();
        assert_eq!(reachable, expected);
    }

    #[test]
    fn manifest_depends_on_equals_siblings() {
        let manifest_str = include_str!("../app/roym.toml");
        let manifest: toml::Value = toml::from_str(manifest_str).expect("parse roym.toml");
        let web_deps: HashSet<&str> = manifest["services"]["web"]["depends_on"]
            .as_array()
            .expect("web depends_on is array")
            .iter()
            .map(|v| v.as_str().expect("dep is string"))
            .collect();
        let expected: HashSet<&str> = SIBLINGS.iter().map(|s| s.name).collect();
        assert_eq!(web_deps, expected);
    }

    #[test]
    fn only_web_declares_http_routes_and_assets_in_manifest() {
        let manifest_str = include_str!("../app/roym.toml");
        let manifest: toml::Value = toml::from_str(manifest_str).expect("parse roym.toml");
        let services_table = manifest["services"].as_table().expect("services table");

        assert!(services_table["web"].get("assets").is_some());
        assert!(services_table["web"].get("custom_config").is_some());

        for (name, service_val) in services_table {
            if name != "web" {
                assert!(
                    service_val.get("assets").is_none(),
                    "Service '{name}' must not declare assets"
                );
                if let Some(custom_config) = service_val.get("custom_config") {
                    let cfg_str = custom_config.as_str().unwrap_or_default();
                    assert!(
                        !cfg_str.contains("http_routes"),
                        "Service '{name}' must not declare http_routes"
                    );
                }
            }
        }
    }

    #[test]
    fn every_public_method_is_routable() {
        for method in PUBLIC_METHODS {
            assert!(route(method).is_some(), "Public method '{method}' is not routable");
            assert_eq!(method_auth(method), Some(MethodAuth::Public));
        }
    }

    #[test]
    fn every_route_prefix_has_an_auth_classification() {
        for &(prefix, _, auth) in ROUTES {
            let sample_method = format!("{prefix}test");
            assert_eq!(method_auth(&sample_method), Some(auth));
        }
    }

    #[test]
    fn method_auth_is_none_for_an_unroutable_method() {
        assert_eq!(method_auth("unknown.method"), None);
        assert_eq!(method_auth(""), None);
    }
}

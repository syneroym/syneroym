//! Method prefix routing table.

use crate::services::{CATALOG, CONVERSATION, DIRECTORY, PROFILE, Service, TRANSACTION};

/// JSON-RPC method prefix -> the sibling that owns it. The spec's own API
/// column, made executable. No default arm: an unlisted prefix is
/// `-32601`, because "which service owns this?" is a question with a
/// written answer or no answer.
///
/// `session.whoami` is handled directly by `web` and is not routed to a
/// sibling.
const ROUTES: &[(&str, Service)] = &[
    ("conversation.", CONVERSATION),
    ("profile.", PROFILE),
    ("contacts.", PROFILE),
    ("block.", PROFILE),
    ("listing.", CATALOG),
    ("availability.", CATALOG),
    ("request.", TRANSACTION),
    ("quote.", TRANSACTION),
    ("agreement.", TRANSACTION),
    ("receipt.", TRANSACTION),
    ("directory.", DIRECTORY),
];

pub fn route(method: &str) -> Option<Service> {
    for (prefix, service) in ROUTES {
        if method.starts_with(prefix) {
            return Some(*service);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::services::SIBLINGS;

    #[test]
    fn every_prefix_maps_to_a_service_in_siblings() {
        for &(prefix, service) in ROUTES {
            assert!(
                SIBLINGS.contains(&service),
                "Prefix '{prefix}' mapped to service '{}' which is not in SIBLINGS",
                service.name
            );
        }
    }

    #[test]
    fn no_prefix_is_a_prefix_of_another() {
        for (i, &(p1, _)) in ROUTES.iter().enumerate() {
            for (j, &(p2, _)) in ROUTES.iter().enumerate() {
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
        let reachable: HashSet<&'static str> = ROUTES.iter().map(|(_, s)| s.name).collect();
        let expected: HashSet<&'static str> = SIBLINGS.iter().map(|s| s.name).collect();
        assert_eq!(reachable, expected);
    }
}

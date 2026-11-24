//! The linked build's dependency bindings must match the manifest.

use syneroym_app_orchestration::{LogicalServiceName, SynAppManifest};
use syneroym_roym_core::services;

use super::{ROYM_BINDINGS, ROYM_MANIFEST_TOML};

#[test]
fn every_manifest_dependency_has_a_binding() {
    let manifest = SynAppManifest::from_toml(ROYM_MANIFEST_TOML).expect("the manifest parses");

    for (name, spec) in &manifest.services {
        if name.as_str() == services::WEB.name {
            continue;
        }
        for dep in &spec.depends_on {
            assert!(
                ROYM_BINDINGS.iter().any(|(consumer, dependency)| *consumer == name.as_str()
                    && *dependency == dep.as_str()),
                "{} declares a dependency on {} with no binding",
                name,
                dep
            );
        }
    }

    for (consumer, dependency) in ROYM_BINDINGS {
        let spec = manifest
            .services
            .get(&LogicalServiceName::new(*consumer))
            .unwrap_or_else(|| panic!("the manifest declares no service '{consumer}'"));
        assert!(
            spec.depends_on.iter().any(|declared| declared.as_str() == *dependency),
            "{consumer} has a binding to {dependency} that the manifest does not declare"
        );
    }
}

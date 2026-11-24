//! Every list of Roym services outside this crate must name every service.
//!
//! These lists are in the manifest, the build task, the CLI, the Hub and the
//! test harnesses. A service that is added to [`ALL`] and missed in one of
//! them fails here, and the message names the file and the service. The
//! checks are text checks on purpose: each reads one file and looks for the
//! form that file uses.

use std::{fs, path::PathBuf};

use super::{ALL, Service, WEB};

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn without_web() -> Vec<Service> {
    ALL.iter().copied().filter(|s| *s != WEB).collect()
}

/// The text between the first `[` after `marker` and `=`, and the next `]`.
fn list_after<'a>(text: &'a str, marker: &str) -> &'a str {
    let at = text.find(marker).unwrap_or_else(|| panic!("'{marker}' not found"));
    let equals = at + text[at..].find('=').unwrap_or_else(|| panic!("no '=' after '{marker}'"));
    let open =
        equals + text[equals..].find('[').unwrap_or_else(|| panic!("no list for '{marker}'"));
    let close = open + text[open..].find(']').unwrap_or_else(|| panic!("list '{marker}' is open"));
    &text[open + 1..close]
}

/// Fails with a message that says what to add and where.
fn require_each(file: &str, place: &str, services: &[Service], found: impl Fn(&str) -> bool) {
    for service in services {
        assert!(
            found(service.name),
            "{file}: {place} is missing the service '{}'; add it",
            service.name
        );
    }
}

fn require_in_list(file: &str, marker: &str, services: &[Service]) {
    let text = read(file);
    let list = list_after(&text, marker);
    require_each(file, &format!("the list '{marker}'"), services, |name| {
        list.contains(&format!("\"{name}\"")) || list.contains(&format!("'{name}'"))
    });
}

fn require_text(file: &str, services: &[Service], form: impl Fn(&str) -> String) {
    let text = read(file);
    require_each(file, "the file", services, |name| text.contains(&form(name)));
}

#[test]
fn every_service_has_a_table_in_the_roym_manifest() {
    let manifest: toml::Value = toml::from_str(&read("app/roym.toml")).unwrap();
    let tables = manifest["services"].as_table().unwrap();
    let mut declared: Vec<&str> = tables.keys().map(String::as_str).collect();
    declared.sort_unstable();
    let mut expected: Vec<&str> = ALL.iter().map(|s| s.name).collect();
    expected.sort_unstable();
    assert_eq!(declared, expected, "app/roym.toml: [services.*] tables differ from services::ALL");
}

#[test]
fn every_service_is_in_the_roym_build_task() {
    let mise = read("../../mise.toml");
    let task = mise.find("[tasks.\"build:roym\"]").expect("the build:roym task");
    let line = mise[task..]
        .lines()
        .find(|l| l.contains("for pkg in"))
        .expect("the build:roym task has a 'for pkg in' line");
    let built: Vec<&str> = line
        .trim()
        .trim_start_matches("for pkg in")
        .trim_end_matches("; do")
        .split_whitespace()
        .collect();
    require_each("mise.toml", "the build:roym loop", &ALL, |name| built.contains(&name));
}

#[test]
fn every_service_is_checked_by_the_dependency_task() {
    require_text("../../xtask/src/main.rs", &ALL, |name| format!("\"syneroym-roym-{name}\""));
}

#[test]
fn every_service_has_a_wasm_path_in_the_test_constants() {
    require_text("../core/src/test_constants.rs", &ALL, |name| {
        format!("syneroym_roym_{name}.wasm")
    });
}

#[test]
fn every_service_but_web_is_in_the_cli_backup_orders() {
    let file = "../../apps/roymctl/src/commands/roym/backup.rs";
    require_in_list(file, "const EXPORT_SERVICES", &without_web());
    require_in_list(file, "const IMPORT_ORDER", &without_web());
}

/// A restore keeps every agreed seat only when the seats are exported after
/// the agreements and imported before them.
#[test]
fn the_backup_orders_put_scheduling_around_transaction() {
    for file in
        ["../../apps/roymctl/src/commands/roym/backup.rs", "../substrate/tests/roym_restore_e2e.rs"]
    {
        let text = read(file);
        let position = |list: &str, name: &str| {
            list_after(&text, list).find(&format!("\"{name}\"")).expect("service in list")
        };
        assert!(
            position("const EXPORT_SERVICES", "transaction")
                < position("const EXPORT_SERVICES", "scheduling"),
            "{file}: export transaction before scheduling"
        );
        assert!(
            position("const IMPORT_ORDER", "scheduling")
                < position("const IMPORT_ORDER", "transaction"),
            "{file}: import scheduling before transaction"
        );
    }
}

#[test]
fn every_service_but_web_has_a_backup_line_in_the_hub() {
    require_text("../roym_web/ui/src/screens/backup.ts", &without_web(), |name| {
        format!("exportMethod: \"{name}.export\"")
    });
}

#[test]
fn every_service_has_a_native_init_in_the_substrate() {
    require_text("../substrate/src/runtime/roym.rs", &ALL, |name| format!("fn init_roym_{name}("));
}

#[test]
fn every_service_is_in_the_parity_harness() {
    let file = "../roym_web/tests/dual_build_parity/helpers.rs";
    require_text(file, &ALL, |name| {
        format!("(\"{name}\", test_constants::roym_{name}_wasm_path())")
    });
    let text = read(file);
    require_each(file, "the two dispatch arms", &ALL, |name| {
        text.matches(&format!("\"{name}\" => self.")).count() >= 2
    });
}

#[test]
fn every_service_is_in_the_playwright_boot_list() {
    require_in_list("../substrate/tests/e2e/node-boot.ts", "const ROYM_SERVICES", &ALL);
}

#[test]
fn the_test_harness_visibility_matches_the_manifest() {
    let manifest: toml::Value = toml::from_str(&read("app/roym.toml")).unwrap();
    let harness = read("../substrate/tests/common/roym.rs");
    let arms = harness
        .split("pub fn service_visibility")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("service_visibility in the test harness");
    for service in ALL {
        let declared = manifest["services"][service.name]["visibility"].as_str().unwrap();
        let word = match declared {
            "internal" => "Internal",
            "private" => "Private",
            _ => continue,
        };
        let arm = format!("=> Visibility::{word}");
        let named = format!("\"{}\"", service.name);
        assert!(
            arms.lines().any(|line| line.contains(&arm) && line.contains(&named)),
            "crates/substrate/tests/common/roym.rs: service_visibility must map '{}' to {word}, \
             as app/roym.toml does",
            service.name
        );
    }
}

#[test]
fn every_service_but_web_is_in_the_restore_test_orders() {
    let file = "../substrate/tests/roym_restore_e2e.rs";
    require_in_list(file, "const EXPORT_SERVICES", &without_web());
    require_in_list(file, "const IMPORT_ORDER", &without_web());
}

#[test]
fn every_service_but_web_is_in_the_trust_test_data_list() {
    require_in_list("../substrate/tests/roym_trust_e2e.rs", "const DATA_SERVICES", &without_web());
}

/// A service that exports a bundle signs it as the person, so it needs a
/// certificate. Every service but `web` signs.
#[test]
fn every_service_but_web_is_in_each_signing_list() {
    require_in_list(
        "../../apps/roymctl/src/commands/roym/signing.rs",
        "const SIGNING_SERVICES",
        &without_web(),
    );
    require_in_list(
        "../substrate/tests/common/roym.rs",
        "pub const SIGNING_SERVICES",
        &without_web(),
    );
    require_in_list(
        "../roym_web/ui/src/session/enrolment.ts",
        "export const SIGNING_SERVICES",
        &without_web(),
    );
}

#[test]
fn by_name_finds_each_service_and_nothing_else() {
    for service in ALL {
        assert_eq!(super::by_name(service.name), Some(service));
    }
    assert_eq!(super::by_name("nope"), None);
    assert_eq!(super::by_name(""), None);
}

/// `transaction` calls the reserve hooks of the service that an extension
/// manifest names, as a sibling. That works only for a service in its
/// `depends_on` and in the substrate's native bindings of `transaction`.
#[test]
fn every_hook_service_is_a_dependency_of_transaction() {
    let manifest: toml::Value = toml::from_str(&read("app/roym.toml")).unwrap();
    let depends_on: Vec<&str> = manifest["services"]["transaction"]["depends_on"]
        .as_array()
        .expect("transaction depends_on")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    let bindings = read("../substrate/src/runtime/roym.rs");
    let transaction_bindings = bindings
        .split("// `transaction` declares")
        .nth(1)
        .expect("the transaction bindings in the substrate");
    for extension in crate::extension::registry().manifests() {
        let Some(service) = extension.hook_service() else { continue };
        assert!(
            depends_on.contains(&service.name),
            "app/roym.toml: transaction must depend on '{}', the hook service of extension '{}'",
            service.name,
            extension.id
        );
        let binding = format!("services::{}.name", service.name.to_uppercase());
        assert!(
            transaction_bindings.contains(&binding),
            "crates/substrate/src/runtime/roym.rs: transaction needs a binding to '{}'",
            service.name
        );
    }
}

// Linked-build wake registration is verified by the substrate unit test
// the_linked_build_registers_every_wake_of_the_manifest in
// crates/substrate/src/runtime/tests.rs.

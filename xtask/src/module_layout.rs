//! Check that module layouts follow the sibling-file convention under `src/`.
//!
//! In `src/`, a module's entry point is always `<name>.rs` beside its `<name>/`
//! directory. This holds at every depth: `foo.rs` + `foo/`, `foo/tests.rs` +
//! `foo/tests/`, etc. `mod.rs` is never used anywhere under `src/`.
//!
//! The single exception is `tests/<name>/mod.rs` for code shared between
//! integration tests, where cargo requires it to avoid creating standalone
//! empty test binaries.

use std::{path::Path, process::Command};

use anyhow::{Result, anyhow, bail};

/// Returns true if a path refers to a `mod.rs` file located under a crate's
/// `src/` directory hierarchy.
///
/// Paths within integration test or benchmark directories (where a component
/// named `tests` or `benches` appears before `src`) are permitted exceptions.
pub(crate) fn is_src_mod_rs(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) != Some("mod.rs") {
        return false;
    }

    for component in path.components() {
        let s = component.as_os_str();
        if s == "tests" || s == "benches" {
            return false;
        }
        if s == "src" {
            return true;
        }
    }

    false
}

pub fn check_module_layout() -> Result<()> {
    println!("Checking module layout under src/...");
    let workspace_root = crate::get_workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .current_dir(&workspace_root)
        .output()
        .map_err(|e| anyhow!("failed to run `git ls-files`: {e}"))?;

    if !output.status.success() {
        bail!("`git ls-files` failed");
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();
    let mut checked = 0;

    for line in files.lines() {
        let rel_path = line.trim();
        if rel_path.is_empty() {
            continue;
        }

        checked += 1;
        let path = Path::new(rel_path);
        if is_src_mod_rs(path) {
            violations.push(rel_path.to_string());
        }
    }

    if !violations.is_empty() {
        eprintln!("\nFound {} module layout violation(s) (mod.rs under src/):", violations.len());
        for v in &violations {
            eprintln!("  ERROR: {v}");
        }
        bail!(
            "Module layout check failed: `mod.rs` is forbidden under `src/`. Use sibling files \
             instead (e.g., `<name>.rs` alongside `<name>/`)."
        );
    }

    println!("All {checked} Rust source files adhere to the module layout convention.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_is_src_mod_rs_detects_under_src() {
        assert!(is_src_mod_rs(Path::new("crates/router/src/proxy/tests/mod.rs")));
        assert!(is_src_mod_rs(Path::new("crates/foo/src/mod.rs")));
        assert!(is_src_mod_rs(Path::new("src/bar/mod.rs")));
    }

    #[test]
    fn test_is_src_mod_rs_allows_tests_and_benches() {
        assert!(!is_src_mod_rs(Path::new("crates/sandbox_wasm/tests/common/mod.rs")));
        assert!(!is_src_mod_rs(Path::new("crates/substrate/tests/common/mod.rs")));
        assert!(!is_src_mod_rs(Path::new("benches/common/mod.rs")));
        assert!(!is_src_mod_rs(Path::new("crates/router/src/proxy/tests.rs")));
        assert!(!is_src_mod_rs(Path::new("crates/core/tests/src/mod.rs")));
        assert!(!is_src_mod_rs(Path::new("tests/foo/src/mod.rs")));
        assert!(!is_src_mod_rs(Path::new("crates/core/benches/src/mod.rs")));
    }
}

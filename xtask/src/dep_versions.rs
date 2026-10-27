//! Check that dependency version requirements name only `major.minor`.
//!
//! `Cargo.lock` holds the exact version, so a patch number in `Cargo.toml`
//! only raises the lowest version Cargo may pick. It is right only when an
//! older patch release is broken or vulnerable for us. Such a requirement
//! carries a `# patch-pin: <reason>` comment on its line or on the line
//! above. A pre-release requirement (`0.11.0-rc.2`) needs all three parts and
//! is allowed.

use std::{fs, process::Command};

use anyhow::{Result, anyhow, bail};

const MARKER: &str = "# patch-pin:";

/// Returns one message per requirement in `content` that has a patch part
/// and no `patch-pin` comment. `file` labels the messages.
pub(crate) fn violations_in(file: &str, content: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let mut section = String::new();
    let mut previous = "";
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            section = trimmed.trim_matches(|c| c == '[' || c == ']').to_string();
        } else if let Some(req) = requirement(&section, trimmed)
            && has_patch_part(req)
            && !line.contains(MARKER)
            && !previous.trim_start().starts_with(MARKER)
        {
            violations.push(format!("{file}:{}: \"{req}\"", index + 1));
        }
        if !trimmed.is_empty() {
            previous = line;
        }
    }
    violations
}

/// The version requirement on a dependency line, if `line` is one.
fn requirement<'a>(section: &str, line: &'a str) -> Option<&'a str> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim_start();
    if section.ends_with("dependencies") {
        if let Some(rest) = value.strip_prefix('"') {
            return rest.split('"').next();
        }
        let after = value.strip_prefix('{')?.split("version").nth(1)?;
        return after
            .trim_start()
            .strip_prefix('=')?
            .trim_start()
            .strip_prefix('"')?
            .split('"')
            .next();
    }
    // A `[dependencies.<name>]` table.
    if section.contains("dependencies.") && key == "version" {
        return value.strip_prefix('"')?.split('"').next();
    }
    None
}

/// True when any comparator in `req` names `major.minor.patch` without a
/// pre-release.
fn has_patch_part(req: &str) -> bool {
    req.split(',').any(|comparator| {
        let version = comparator.trim().trim_start_matches(['=', '^', '~', '>', '<', ' ']);
        !version.contains('-') && version.split('.').count() >= 3
    })
}

pub fn check_dep_versions() -> Result<()> {
    println!("Checking dependency version requirements...");
    let workspace_root = crate::get_workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*Cargo.toml"])
        .current_dir(&workspace_root)
        .output()
        .map_err(|e| anyhow!("failed to run `git ls-files`: {e}"))?;
    if !output.status.success() {
        bail!("`git ls-files` failed");
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();
    let mut checked = 0;
    for rel_path in files.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let content = fs::read_to_string(workspace_root.join(rel_path))?;
        violations.extend(violations_in(rel_path, &content));
        checked += 1;
    }

    if !violations.is_empty() {
        eprintln!("\nFound {} dependency version(s) with a patch part:", violations.len());
        for v in &violations {
            eprintln!("  ERROR: {v}");
        }
        bail!(
            "Dependency version check failed: write `major.minor` (e.g. \"1.1\", \"0.13\"). Keep \
             a patch part only when older patches are broken or vulnerable for us, and say why in \
             a `{MARKER} <reason>` comment on the line or the line above."
        );
    }

    println!("All {checked} Cargo.toml files name dependency versions as major.minor.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_a_patch_part_in_each_dependency_form() {
        let content = r#"
[workspace.dependencies]
flate2 = "1.1.9"
axum-server = { version = "0.8.0", features = ["tls-rustls"] }

[target.'cfg(unix)'.dev-dependencies]
walkdir = "^2.5.0"

[dependencies.swc_core]
version = "76.0.0"
"#;
        let found = violations_in("Cargo.toml", content);
        assert_eq!(
            found,
            [
                "Cargo.toml:3: \"1.1.9\"",
                "Cargo.toml:4: \"0.8.0\"",
                "Cargo.toml:7: \"^2.5.0\"",
                "Cargo.toml:10: \"76.0.0\"",
            ]
        );
    }

    #[test]
    fn allows_major_minor_pre_release_and_marked_pins() {
        let content = r#"
[package]
version = "0.1.0"
rust-version = "1.95.0"

[dependencies]
flate2 = "1.1"
p256 = { version = "0.14", features = ["ecdh"] }
sha2 = "0.11.0-rc.2"
iroh.workspace = true
local = { path = "../local" }
# patch-pin: 48.0.0 to 48.0.6 miscompile br_table on aarch64.
wasmtime = { version = "48.0.7", features = ["cache"] }
wasmtime-wasi = "48.0.7" # patch-pin: matches wasmtime.
"#;
        assert!(violations_in("Cargo.toml", content).is_empty());
    }

    #[test]
    fn a_marker_covers_only_the_next_requirement() {
        let content = r#"
[dependencies]
# patch-pin: needed.
a = "1.0.1"
b = "1.0.1"
"#;
        assert_eq!(violations_in("Cargo.toml", content), ["Cargo.toml:5: \"1.0.1\""]);
    }
}

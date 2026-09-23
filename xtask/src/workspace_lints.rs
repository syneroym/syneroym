use std::{collections::HashSet, fs, path::Path, process::Command};

use anyhow::Result;
use toml::Value;

pub fn check_workspace_lint_config(
    workspace_root: &Path,
    violations: &mut Vec<String>,
) -> Result<()> {
    check_global_switches(workspace_root, violations)?;
    check_clippy_toml(workspace_root, violations)?;
    check_workspace_lints(workspace_root, violations)?;
    Ok(())
}

fn check_clippy_toml(workspace_root: &Path, violations: &mut Vec<String>) -> Result<()> {
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*clippy.toml"])
        .current_dir(workspace_root)
        .output()?;
    let out = String::from_utf8_lossy(&output.stdout);
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed != "clippy.toml" {
            violations.push(format!(
                "forbidden crate-local clippy.toml found: {trimmed} (only root clippy.toml is \
                 allowed)"
            ));
        }
    }
    Ok(())
}

fn check_global_switches(workspace_root: &Path, violations: &mut Vec<String>) -> Result<()> {
    // 1. Root clippy.toml: threshold must exist and be <= 100
    let clippy_toml_path = workspace_root.join("clippy.toml");
    if !clippy_toml_path.exists() {
        violations.push("root clippy.toml is missing".to_string());
    } else {
        let content = fs::read_to_string(&clippy_toml_path)?;
        let val: Value = toml::from_str(&content)?;
        match val.get("too-many-lines-threshold").and_then(|v| v.as_integer()) {
            Some(threshold) if threshold <= 100 => {}
            Some(threshold) => {
                violations.push(format!(
                    "root clippy.toml too-many-lines-threshold is {threshold} (maximum allowed is \
                     100)"
                ));
            }
            None => {
                violations.push(
                    "root clippy.toml must define too-many-lines-threshold <= 100".to_string(),
                );
            }
        }
    }

    // 2. Root Cargo.toml: workspace.lints.clippy.too_many_lines must not be "allow"
    let cargo_toml_path = workspace_root.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml_path)?;
    let val: Value = toml::from_str(&content)?;
    let clippy_lints =
        val.get("workspace").and_then(|w| w.get("lints")).and_then(|l| l.get("clippy"));

    if let Some(clippy) = clippy_lints {
        let too_many_lines = clippy.get("too_many_lines");
        match too_many_lines {
            Some(Value::String(level)) => {
                if level == "allow" {
                    violations.push(
                        "workspace.lints.clippy.too_many_lines must not be 'allow' in root \
                         Cargo.toml"
                            .to_string(),
                    );
                }
            }
            Some(Value::Table(tbl)) => {
                if tbl.get("level").and_then(|v| v.as_str()) == Some("allow") {
                    violations.push(
                        "workspace.lints.clippy.too_many_lines must not be 'allow' in root \
                         Cargo.toml"
                            .to_string(),
                    );
                }
            }
            Some(_) => {}
            None => {
                violations.push(
                    "workspace.lints.clippy.too_many_lines is missing in root Cargo.toml"
                        .to_string(),
                );
            }
        }
    } else {
        violations.push("workspace.lints.clippy is missing in root Cargo.toml".to_string());
    }

    // 3. .cargo/config.toml: must not reference too_many_lines
    let cargo_config_path = workspace_root.join(".cargo/config.toml");
    if cargo_config_path.exists() {
        let content = fs::read_to_string(&cargo_config_path)?;
        if content.contains("too_many_lines") {
            violations.push(
                ".cargo/config.toml must not reference too_many_lines in flags or settings"
                    .to_string(),
            );
        }
    }

    Ok(())
}

fn check_workspace_lints(workspace_root: &Path, violations: &mut Vec<String>) -> Result<()> {
    let root_manifest_path = workspace_root.join("Cargo.toml");
    let content = fs::read_to_string(&root_manifest_path)?;
    let val: Value = toml::from_str(&content)?;

    let members = val
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    let exclude: HashSet<String> = val
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim_end_matches('/').to_string()))
                .collect()
        })
        .unwrap_or_default();

    for member_val in members {
        let member_str = match member_val.as_str() {
            Some(s) => s,
            None => continue,
        };

        if member_str.contains('*') {
            let prefix = member_str.trim_end_matches('*').trim_end_matches('/');
            let dir = workspace_root.join(prefix);
            if dir.is_dir() {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() && path.join("Cargo.toml").exists() {
                        let rel = path
                            .strip_prefix(workspace_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        if !exclude.contains(&rel) {
                            check_member_lints(&path, &rel, violations)?;
                        }
                    }
                }
            }
        } else {
            let path = workspace_root.join(member_str);
            if path.join("Cargo.toml").exists() && !exclude.contains(member_str) {
                check_member_lints(&path, member_str, violations)?;
            }
        }
    }

    Ok(())
}

fn check_member_lints(
    member_dir: &Path,
    member_name: &str,
    violations: &mut Vec<String>,
) -> Result<()> {
    let manifest_path = member_dir.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)?;
    let val: Value = toml::from_str(&content)?;

    let has_workspace_lints =
        val.get("lints").and_then(|l| l.get("workspace")).and_then(|w| w.as_bool()) == Some(true);

    if !has_workspace_lints {
        violations.push(format!("{member_name}/Cargo.toml is missing `[lints] workspace = true`"));
    }

    Ok(())
}

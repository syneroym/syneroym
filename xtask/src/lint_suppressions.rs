use std::{collections::HashSet, fs, path::Path, process::Command};

use anyhow::{Result, bail};

/// Ceiling on clippy::too_many_lines suppressions across the workspace.
///
/// This is a ratchet: it moves down only. The intent is to cap existing
/// suppressions and ratchet down as oversized functions are decomposed.
pub const MAX_TOO_MANY_LINES_SUPPRESSIONS: usize = 36;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedAttribute {
    pub(crate) is_inner: bool,
    pub(crate) body: String,
    pub(crate) line_no: usize,
    pub(crate) end_byte: usize,
}

fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], mut i: usize, current_line: &mut usize) -> usize {
    i += 2;
    let mut depth = 1;
    while i + 1 < bytes.len() && depth > 0 {
        if bytes[i] == b'\n' {
            *current_line += 1;
        }
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_string_literal(bytes: &[u8], mut i: usize, current_line: &mut usize) -> usize {
    i += 1;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\n' {
            *current_line += 1;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'\n' {
                *current_line += 1;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    if i < bytes.len() {
        i += 1;
    }
    i
}

fn skip_char_literal(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() && bytes[i] != b'\'' && bytes[i] != b'\n' {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i < bytes.len() && bytes[i] == b'\'' {
        i += 1;
    }
    i
}

fn parse_attribute_at(
    content: &str,
    i: usize,
    current_line: &mut usize,
) -> Option<(ParsedAttribute, usize)> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let start_line = *current_line;
    let mut peek = i + 1;
    let mut is_inner = false;
    if peek < len && bytes[peek] == b'!' {
        is_inner = true;
        peek += 1;
    }
    if peek >= len || bytes[peek] != b'[' {
        return None;
    }

    let body_start = peek + 1;
    let mut bracket_depth = 1;
    let mut scan = body_start;

    while scan < len && bracket_depth > 0 {
        if bytes[scan] == b'\n' {
            *current_line += 1;
        }
        if bytes[scan] == b'"' {
            scan = skip_string_literal(bytes, scan, current_line);
            continue;
        }
        if bytes[scan] == b'[' {
            bracket_depth += 1;
        } else if bytes[scan] == b']' {
            bracket_depth -= 1;
        }
        scan += 1;
    }

    if bracket_depth == 0 {
        let body_end = scan - 1;
        let body = content[body_start..body_end].to_string();
        Some((ParsedAttribute { is_inner, body, line_no: start_line, end_byte: scan }, scan))
    } else {
        None
    }
}

pub(crate) fn extract_attributes(content: &str) -> Vec<ParsedAttribute> {
    let mut attrs = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current_line = 1;

    while i < len {
        if bytes[i] == b'\n' {
            current_line += 1;
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i, &mut current_line);
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string_literal(bytes, i, &mut current_line);
            continue;
        }
        if bytes[i] == b'\'' {
            i = skip_char_literal(bytes, i);
            continue;
        }
        if bytes[i] == b'#'
            && let Some((attr, next_i)) = parse_attribute_at(content, i, &mut current_line)
        {
            attrs.push(attr);
            i = next_i;
            continue;
        }
        i += 1;
    }

    attrs
}

pub(crate) fn next_item_is_mod(content: &str, start_byte: usize) -> bool {
    let mut i = start_byte;
    let bytes = content.as_bytes();
    let len = bytes.len();

    while i < len {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < len && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'#' && i + 1 < len && bytes[i + 1] == b'[' {
            i += 2;
            let mut bracket_depth = 1;
            while i < len && bracket_depth > 0 {
                if bytes[i] == b'[' {
                    bracket_depth += 1;
                } else if bytes[i] == b']' {
                    bracket_depth -= 1;
                } else if bytes[i] == b'"' {
                    i += 1;
                    while i < len && bytes[i] != b'"' {
                        if bytes[i] == b'\\' && i + 1 < len {
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                }
                i += 1;
            }
            continue;
        }
        break;
    }

    let rem = &content[i..];
    let trimmed = rem.trim_start();
    let after_vis = if let Some(rest) = trimmed.strip_prefix("pub") {
        let rest = rest.trim_start();
        if let Some(rest2) = rest.strip_prefix('(') {
            if let Some(close_idx) = rest2.find(')') {
                rest2[close_idx + 1..].trim_start()
            } else {
                rest
            }
        } else {
            rest
        }
    } else {
        trimmed
    };

    if let Some(rest) = after_vis.strip_prefix("mod") {
        rest.chars().next().is_none_or(|c| c.is_whitespace() || c == '{' || c == ';')
    } else {
        false
    }
}

pub(crate) fn strip_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in s.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
            }
        } else if c == '"' {
            in_str = true;
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn contains_lint(stripped_body: &str, lint_name: &str) -> bool {
    for part in stripped_body.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')) {
        let trimmed = part.trim();
        if trimmed == lint_name || trimmed.strip_prefix("clippy::") == Some(lint_name) {
            return true;
        }
    }
    false
}

fn check_attribute(
    attr: &ParsedAttribute,
    content: &str,
    rel_path: &str,
    violations: &mut Vec<String>,
) -> usize {
    let stripped = strip_strings(&attr.body);
    let names_pedantic = contains_lint(&stripped, "pedantic");
    let names_too_many_lines = contains_lint(&stripped, "too_many_lines");

    if names_pedantic && (stripped.contains("allow") || stripped.contains("expect")) {
        violations.push(format!("{rel_path}:{}: attribute names clippy::pedantic", attr.line_no));
    }

    if names_too_many_lines {
        if attr.is_inner {
            violations.push(format!(
                "{rel_path}:{}: inner attribute (#![...]) names clippy::too_many_lines",
                attr.line_no
            ));
            return 0;
        }
        if next_item_is_mod(content, attr.end_byte) {
            violations.push(format!(
                "{rel_path}:{}: outer attribute on a mod names clippy::too_many_lines",
                attr.line_no
            ));
            return 0;
        }
        return 1;
    }

    0
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

/// Crates exempted from mandatory `[lints] workspace = true` with explicit
/// rationale.
const WORKSPACE_LINTS_EXCEPTIONS: &[(&str, &str)] =
    &[("tests/perf", "perf soak/concurrency benchmark harness is excluded from workspace linting")];

fn check_workspace_lints(workspace_root: &Path, violations: &mut Vec<String>) -> Result<()> {
    let root_manifest_path = workspace_root.join("Cargo.toml");
    let content = fs::read_to_string(&root_manifest_path)?;
    let val: toml::Value = toml::from_str(&content)?;

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

    let mut member_paths = Vec::new();
    for m in members {
        let pattern = match m.as_str() {
            Some(s) => s.trim_end_matches('/'),
            None => continue,
        };
        if let Some(base) = pattern.strip_suffix("/*") {
            let base_dir = workspace_root.join(base);
            if let Ok(entries) = fs::read_dir(base_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("Cargo.toml").exists() {
                        let rel = path
                            .strip_prefix(workspace_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        if !exclude.contains(&rel) {
                            member_paths.push(rel);
                        }
                    }
                }
            }
        } else {
            let path = workspace_root.join(pattern);
            if path.join("Cargo.toml").exists() && !exclude.contains(pattern) {
                member_paths.push(pattern.to_string());
            }
        }
    }

    for member in member_paths {
        let manifest_path = workspace_root.join(&member).join("Cargo.toml");
        let manifest_content = fs::read_to_string(&manifest_path)?;
        let manifest_val: toml::Value = toml::from_str(&manifest_content)?;

        let has_workspace_lints =
            manifest_val.get("lints").and_then(|l| l.get("workspace")).and_then(|w| w.as_bool())
                == Some(true);

        if !has_workspace_lints {
            if let Some((_, _reason)) =
                WORKSPACE_LINTS_EXCEPTIONS.iter().find(|(m, _)| *m == member)
            {
                continue;
            }
            violations.push(format!("{member}/Cargo.toml: missing `[lints] workspace = true`"));
        }
    }

    Ok(())
}

pub fn check_lint_suppressions() -> Result<()> {
    println!(
        "Checking clippy::too_many_lines suppressions (max {MAX_TOO_MANY_LINES_SUPPRESSIONS})..."
    );
    let workspace_root = crate::get_workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "*.rs"])
        .current_dir(&workspace_root)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `git ls-files`: {e}"))?;

    if !output.status.success() {
        bail!("`git ls-files` failed");
    }

    let files_str = String::from_utf8_lossy(&output.stdout);
    let mut suppressions = Vec::new();
    let mut violations = Vec::new();

    for rel_path in files_str.lines() {
        let path = workspace_root.join(rel_path);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let attrs = extract_attributes(&content);
        for attr in attrs {
            let count = check_attribute(&attr, &content, rel_path, &mut violations);
            if count > 0 {
                suppressions.push((rel_path.to_string(), attr.line_no));
            }
        }
    }

    check_clippy_toml(&workspace_root, &mut violations)?;
    check_workspace_lints(&workspace_root, &mut violations)?;

    if !violations.is_empty() {
        for v in &violations {
            eprintln!("ERROR: {v}");
        }
        bail!("Lint suppressions check found {} violation(s)", violations.len());
    }

    let total = suppressions.len();
    println!("Found {total} clippy::too_many_lines suppression(s) across tracked files.");

    if total < MAX_TOO_MANY_LINES_SUPPRESSIONS {
        bail!(
            "clippy::too_many_lines suppressions ({total}) are below cap \
             ({MAX_TOO_MANY_LINES_SUPPRESSIONS}); lower MAX_TOO_MANY_LINES_SUPPRESSIONS to {total}"
        );
    }

    if total > MAX_TOO_MANY_LINES_SUPPRESSIONS {
        eprintln!(
            "ERROR: too_many_lines suppressions ({total}) exceed maximum allowed \
             ({MAX_TOO_MANY_LINES_SUPPRESSIONS}):"
        );
        for (file, line) in &suppressions {
            eprintln!("  {file}:{line}");
        }
        bail!(
            "Lint suppressions check failed: {total} exceeds maximum of \
             {MAX_TOO_MANY_LINES_SUPPRESSIONS}"
        );
    }

    println!(
        "All clippy::too_many_lines suppressions are within the limit ({total} <= \
         {MAX_TOO_MANY_LINES_SUPPRESSIONS})."
    );
    Ok(())
}

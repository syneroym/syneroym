use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{method::MethodInfo, router, services::Service};

/// Asserts that a service's method table and implementation adhere to contract
/// rules.
///
/// This checks:
/// - Method names are unique within the table.
/// - Every method routes to the given [`Service`] in [`router::route`].
/// - Input and output types do not contain `serde_json::value::Value`.
/// - Local-only methods reject unknown fields, while wire-reachable methods
///   allow them.
/// - Every method listed in `wire` exists in the method table.
/// - Service source files under `service_src_dir` do not have hand-written
///   match arms, inspect `req.method` outside `Response::method_not_found`, or
///   mention unlisted method string literals.
///
/// # Limitations
/// - The `Value` check inspects only the outer type name; nested fields
///   containing `Value` (such as `FooOutput { data: Value }`) are not detected.
/// - The unknown-field probe checks only top-level fields; nested input structs
///   must explicitly add `#[serde(deny_unknown_fields)]` because the check does
///   not inspect them.
#[expect(clippy::panic, reason = "test-assertion helper for service tables")]
pub fn assert_service_table(
    service: Service,
    rows: &[MethodInfo],
    wire: &[&str],
    service_src_dir: &Path,
) {
    let mut problems = Vec::new();

    // Duplicate name
    let mut seen = HashSet::new();
    for row in rows {
        if !seen.insert(&row.name) {
            problems.push(format!("duplicate method name: {}", row.name));
        }
    }

    // Routing
    for row in rows {
        if router::route(&row.name) != Some(service) {
            problems.push(format!(
                "method {} does not route to service {:?} (routes to {:?})",
                row.name,
                service,
                router::route(&row.name)
            ));
        }
    }

    // Type name containing serde_json::value::Value
    for row in rows {
        if row.input.contains("serde_json::value::Value") {
            problems.push(format!("method {} input contains Value: {}", row.name, row.input));
        }
        if row.output.contains("serde_json::value::Value") {
            problems.push(format!("method {} output contains Value: {}", row.name, row.output));
        }
    }

    // Probe unknown fields
    check_probe_unknown_fields(rows, wire, &mut problems);

    // Name in wire that is not a row
    for wire_name in wire {
        if !rows.iter().any(|r| r.name == *wire_name) {
            problems.push(format!("wire method {} is not in method table", wire_name));
        }
    }

    // Source code scan under service_src_dir
    scan_source_dir(service, rows, service_src_dir, &mut problems);

    if !problems.is_empty() {
        panic!("assert_service_table failed for {:?}:\n{}", service, problems.join("\n"));
    }
}

fn check_probe_unknown_fields(rows: &[MethodInfo], wire: &[&str], problems: &mut Vec<String>) {
    let probe_value = json!({ "__probe__": 1 });
    for row in rows {
        let probe_res = (row.probe)(&probe_value);
        let is_wire = wire.contains(&row.name.as_str());
        let rejects_unknown = match &probe_res {
            Err(e) => e.message.contains("unknown field"),
            Ok(_) => false,
        };

        if !is_wire && !rejects_unknown {
            problems.push(format!(
                "method {} is not in wire, but does not reject unknown field with 'unknown \
                 field': {:?}",
                row.name, probe_res
            ));
        } else if is_wire && rejects_unknown {
            problems.push(format!(
                "method {} is in wire, but rejects unknown field: {:?}",
                row.name, probe_res
            ));
        }
    }
}

fn scan_source_dir(service: Service, rows: &[MethodInfo], dir: &Path, problems: &mut Vec<String>) {
    if !dir.exists() {
        problems.push(format!("service_src_dir does not exist: {}", dir.display()));
        return;
    }
    let mut files = Vec::new();
    collect_rs_files(dir, &mut files, problems);
    if files.is_empty() {
        problems.push(format!("no .rs files found in {}", dir.display()));
        return;
    }
    for path in files {
        if path.file_name().and_then(|n| n.to_str()) == Some("bindings.rs") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                problems.push(format!("failed to read {}: {e}", path.display()));
                continue;
            }
        };
        let is_app_rs = path.file_name().and_then(|n| n.to_str()) == Some("app.rs");
        check_file_lines(service, rows, &path, &content, is_app_rs, problems);
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>, problems: &mut Vec<String>) {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            problems.push(format!("failed to read directory {}: {e}", dir.display()));
            return;
        }
    };
    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let p = entry.path();
                if p.is_dir() {
                    collect_rs_files(&p, out, problems);
                } else if p.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
            Err(e) => {
                problems.push(format!("failed to read entry in {}: {e}", dir.display()));
            }
        }
    }
}

fn check_file_lines(
    service: Service,
    rows: &[MethodInfo],
    path: &Path,
    content: &str,
    is_app_rs: bool,
    problems: &mut Vec<String>,
) {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        if is_match_arm(trimmed) {
            problems.push(format!(
                "{}:{}: hand-written match arm: {}",
                path.display(),
                line_idx + 1,
                trimmed
            ));
        }

        if line.contains("req.method") {
            let is_allowed = is_app_rs && line.contains("Response::method_not_found(&req.method)");
            if !is_allowed {
                problems.push(format!(
                    "{}:{}: req.method used outside allowed method_not_found: {}",
                    path.display(),
                    line_idx + 1,
                    trimmed
                ));
            }
        }

        check_string_literals(service, rows, path, line_idx + 1, line, problems);
    }
}

fn is_match_arm(line: &str) -> bool {
    let rest = match line.strip_prefix('"') {
        Some(r) => r,
        None => return false,
    };
    let dot_pos = match rest.find('.') {
        Some(p) => p,
        None => return false,
    };
    let prefix = &rest[..dot_pos];
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return false;
    }
    let rest_after_dot = &rest[dot_pos + 1..];
    let quote_pos = match rest_after_dot.find('"') {
        Some(p) => p,
        None => return false,
    };
    let verb = &rest_after_dot[..quote_pos];
    if verb.is_empty() || !verb.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return false;
    }
    let after_quote = rest_after_dot[quote_pos + 1..].trim_start();
    after_quote.starts_with("=>")
}

fn check_string_literals(
    service: Service,
    rows: &[MethodInfo],
    path: &Path,
    line_no: usize,
    line: &str,
    problems: &mut Vec<String>,
) {
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        let end = match after_start.find('"') {
            Some(e) => e,
            None => break,
        };
        let candidate = &after_start[..end];
        if is_valid_method_format(candidate)
            && router::route(candidate) == Some(service)
            && !rows.iter().any(|r| r.name == candidate)
        {
            problems.push(format!(
                "{}:{}: string literal '{}' routes to {:?} but is not in method table",
                path.display(),
                line_no,
                candidate,
                service
            ));
        }
        rest = &after_start[end + 1..];
    }
}

fn is_valid_method_format(s: &str) -> bool {
    let Some(dot_pos) = s.find('.') else { return false };
    let prefix = &s[..dot_pos];
    let verb = &s[dot_pos + 1..];
    !prefix.is_empty()
        && !verb.is_empty()
        && prefix.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        && verb.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

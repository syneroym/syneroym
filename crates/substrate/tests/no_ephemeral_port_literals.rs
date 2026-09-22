#![allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
//! Guards against hardcoded port numbers in integration and end-to-end tests.
//!
//! A hardcoded port can collide with other test runs or fall inside the
//! OS ephemeral port range. `common::alloc_ports` avoids collisions by
//! finding available ports and holding probe listeners until bind time.
//!
//! All `crates/substrate/tests/*.rs` test files boot through
//! `common::SubstrateNode` or `common::SubstrateTestContext`, using ports
//! from `common::alloc_ports`.
//!
//! This scan checks three things:
//! 1. Any numeric literal assigned to an identifier containing "port" (except
//!    `port` and `container_port`) in Rust test files.
//! 2. Any numeric literal passed as an argument to
//!    `SubstrateTestContext::setup(...)` or `setup_with(...)` (including
//!    multi-line calls) in Rust test files.
//! 3. Any hardcoded port literal, URL port, or legacy static port in TypeScript
//!    E2E spec files (`crates/substrate/tests/e2e/tests/*.spec.ts`).
//!
//! A hand-picked port anywhere causes test collisions. The only safe source
//! of test ports is `common::alloc_ports` in Rust, and `readE2EPorts()` /
//! `readMultihopPorts()` from `../ports` in TypeScript E2E tests.

use std::{fs, path::Path};

/// `ident: <number>` or `ident = <number>`, where `ident`'s last word
/// (case-insensitive) contains "port" as part of a longer name --
/// `backend_port`, `supervisor_iroh_port` -- the shape this crate's own
/// port-carrying `let` bindings and config-struct fields use.
///
/// Two identifiers are deliberately excluded:
/// * the bare identifier `port` on its own: also used for manifest fields (e.g.
///   `NetworkEndpoint { port: 41303, .. }`) that name a service's *declared*
///   address without binding it as a real OS listener in the test process.
/// * `container_port`: the port *inside* an OCI image in a podman port mapping
///   (`ContainerPortMapping { container_port: 80, .. }`), with the host side
///   left to `host_port: None` -- again never a bind in the test process.
fn port_literals_on_line(line: &str) -> Vec<u32> {
    let mut found = Vec::new();
    for sep in [':', '='] {
        let Some(sep_idx) = line.rfind(sep) else { continue };
        let (before, after) = line.split_at(sep_idx);
        let after = &after[1..];

        let ident_is_port_like = before
            .trim_end()
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .is_some_and(|word| {
                let lower = word.to_ascii_lowercase();
                lower.contains("port") && lower != "port" && lower != "container_port"
            });
        if !ident_is_port_like {
            continue;
        }

        let digits: String =
            after.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '_').collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(value) = digits.replace('_', "").parse::<u32>() {
            found.push(value);
        }
    }
    found
}

/// Numeric literal arguments passed to `SubstrateTestContext::setup(...)` or
/// `setup_with(...)`, scanning across line breaks.
fn setup_call_port_literals(contents: &str) -> Vec<(usize, u32)> {
    let mut found = Vec::new();
    let line_starts: Vec<usize> =
        std::iter::once(0).chain(contents.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_at = |byte_offset: usize| -> usize {
        match line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx + 1,
            Err(idx) => idx,
        }
    };

    let patterns = ["SubstrateTestContext::setup(", "setup_with("];
    for pat in patterns {
        let mut search_start = 0;
        while let Some(pos) = contents[search_start..].find(pat) {
            let pat_pos = search_start + pos;
            let start = pat_pos + pat.len();
            let is_boundary = pat_pos == 0
                || !contents[..pat_pos].ends_with(|c: char| c.is_alphanumeric() || c == '_');
            if !is_boundary {
                search_start = start;
                continue;
            }

            let remainder = &contents[start..];
            let end_idx = remainder.find([')', '|']).unwrap_or(remainder.len());
            let args_segment = &remainder[..end_idx];

            let mut seg_offset = 0;
            for piece in args_segment.split(',') {
                let piece_len = piece.len();
                let mut clean_piece = String::new();
                for l in piece.lines() {
                    let uncommented = match l.find("//") {
                        Some(idx) => &l[..idx],
                        None => l,
                    };
                    clean_piece.push_str(uncommented);
                    clean_piece.push(' ');
                }
                let trimmed = clean_piece.trim();
                let digits: String =
                    trimmed.chars().take_while(|c| c.is_ascii_digit() || *c == '_').collect();
                if !digits.is_empty()
                    && digits.len() == trimmed.len()
                    && let Ok(value) = digits.replace('_', "").parse::<u32>()
                {
                    let lit_offset = piece.find(&digits).unwrap_or(0);
                    let abs_offset = start + seg_offset + lit_offset;
                    found.push((line_at(abs_offset), value));
                }
                seg_offset += piece_len + 1;
            }
            search_start = start;
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

#[test]
fn no_test_hardcodes_a_port_literal() {
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut violations = Vec::new();

    for entry in fs::read_dir(&tests_dir).expect("read tests dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("no_ephemeral_port_literals.rs") {
            continue; // this file's own test assertions mention port numbers
        }

        let contents = fs::read_to_string(&path).expect("read test file");
        for (line_no, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue;
            }
            let line_ports = port_literals_on_line(line);
            for value in line_ports {
                if value == 0 {
                    continue;
                }
                violations.push(format!(
                    "{}:{}: hardcoded port literal {value} -- allocate ports with \
                     `common::alloc_ports` (via `common::SubstrateNode`) instead",
                    path.display(),
                    line_no + 1,
                ));
            }
        }
        for (line_no, value) in setup_call_port_literals(&contents) {
            if value == 0 {
                continue;
            }
            violations.push(format!(
                "{}:{}: hardcoded port literal {value} in setup call -- allocate ports with \
                 `common::alloc_ports` (via `common::SubstrateNode`) instead",
                path.display(),
                line_no,
            ));
        }
    }

    assert!(violations.is_empty(), "\n{}\n", violations.join("\n"));
}

#[test]
fn setup_call_literals_detection() {
    let line1 = "let ctx = SubstrateTestContext::setup(7910, 7911, 7912).await;";
    let res1: Vec<u32> = setup_call_port_literals(line1).into_iter().map(|(_, p)| p).collect();
    assert_eq!(res1, vec![7910, 7911, 7912]);

    let line2 = "let ctx = SubstrateTestContext::setup_with(7946, 7947, 7948, |config| {";
    let res2: Vec<u32> = setup_call_port_literals(line2).into_iter().map(|(_, p)| p).collect();
    assert_eq!(res2, vec![7946, 7947, 7948]);

    let line3 =
        "let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;";
    assert!(setup_call_port_literals(line3).is_empty());

    let multiline1 = r#"
        let ctx = SubstrateTestContext::setup(
            7910,
            7911,
            7912,
        ).await;
    "#;
    let res_ml1: Vec<u32> =
        setup_call_port_literals(multiline1).into_iter().map(|(_, p)| p).collect();
    assert_eq!(res_ml1, vec![7910, 7911, 7912]);

    let multiline2 = r#"
        let ctx = SubstrateTestContext::setup_with(
            7946,
            7947,
            7948,
            |config| {
    "#;
    let res_ml2: Vec<u32> =
        setup_call_port_literals(multiline2).into_iter().map(|(_, p)| p).collect();
    assert_eq!(res_ml2, vec![7946, 7947, 7948]);
}

#[test]
fn no_typescript_e2e_spec_hardcodes_a_port() {
    let e2e_specs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/tests");
    let mut violations = Vec::new();

    let forbidden_static_ports =
        [7660, 7661, 7662, 7663, 7664, 7665, 7960, 7961, 7962, 7963, 7964, 7965];

    for entry in fs::read_dir(&e2e_specs_dir).expect("read e2e specs dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }

        let contents = fs::read_to_string(&path).expect("read spec file");
        for (line_no, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue;
            }

            // 1. Check for URL with literal port: http(s)://...:<digits>
            if let Some(pos) = line.find("http://").or_else(|| line.find("https://")) {
                let rest = &line[pos..];
                let end = rest.find(['\'', '"', '`', ' ', '\n', '\r', ';']).unwrap_or(rest.len());
                let url = &rest[..end];
                if let Some(colon_idx) = url.rfind(':')
                    && colon_idx > 5
                {
                    let after_colon = &url[colon_idx + 1..];
                    let port_digits: String =
                        after_colon.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if !port_digits.is_empty()
                        && let Ok(port) = port_digits.parse::<u32>()
                        && port > 0
                    {
                        violations.push(format!(
                            "{}:{}: hardcoded port {port} in URL \"{url}\" -- use \
                             `readE2EPorts()` or `readMultihopPorts()` instead",
                            path.display(),
                            line_no + 1,
                        ));
                    }
                }
            }

            // 2. Check for port-like assignments: e.g. gatewayPort: 7660, const port = 3000
            for sep in [':', '='] {
                let Some(sep_idx) = line.rfind(sep) else { continue };
                let (before, after) = line.split_at(sep_idx);
                let after = &after[1..];

                let ident_is_port_like = before
                    .trim_end()
                    .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .is_some_and(|word| {
                        let lower = word.to_ascii_lowercase();
                        lower.contains("port")
                    });
                if !ident_is_port_like {
                    continue;
                }

                let digits: String = after
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '_')
                    .collect();
                if digits.is_empty() {
                    continue;
                }
                if let Ok(value) = digits.replace('_', "").parse::<u32>() {
                    violations.push(format!(
                        "{}:{}: hardcoded port literal {value} in TypeScript spec -- use \
                         `readE2EPorts()` or `readMultihopPorts()` instead",
                        path.display(),
                        line_no + 1,
                    ));
                }
            }

            // 3. Check for any known static substrate port token
            for &static_port in &forbidden_static_ports {
                let s = static_port.to_string();
                if line.contains(&s) {
                    let mut start = 0;
                    while let Some(idx) = line[start..].find(&s) {
                        let abs_idx = start + idx;
                        let before_char =
                            if abs_idx > 0 { line[..abs_idx].chars().last() } else { None };
                        let after_idx = abs_idx + s.len();
                        let after_char = line[after_idx..].chars().next();
                        let is_digit_before = before_char.is_some_and(|c| c.is_ascii_digit());
                        let is_digit_after = after_char.is_some_and(|c| c.is_ascii_digit());
                        if !is_digit_before && !is_digit_after {
                            let msg = format!(
                                "{}:{}: forbidden static port {static_port} found in TypeScript \
                                 spec",
                                path.display(),
                                line_no + 1,
                            );
                            if !violations.contains(&msg) {
                                violations.push(msg);
                            }
                        }
                        start = after_idx;
                    }
                }
            }
        }
    }

    assert!(violations.is_empty(), "\n{}\n", violations.join("\n"));
}

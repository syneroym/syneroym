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

fn check_typescript_line(line: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('*') {
        return violations;
    }

    // 1. Check for URL with literal port: http(s)://...:<digits> or
    //    ws(s)://...:<digits>
    for scheme in ["http://", "https://", "ws://", "wss://"] {
        if let Some(pos) = line.find(scheme) {
            let rest = &line[pos..];
            let end = rest.find(['\'', '"', '`', ' ', '\n', '\r', ';']).unwrap_or(rest.len());
            let url = &rest[..end];
            if let Some(colon_idx) = url.rfind(':')
                && colon_idx > scheme.len() - 3
            {
                let after_colon = &url[colon_idx + 1..];
                let port_digits: String =
                    after_colon.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !port_digits.is_empty()
                    && let Ok(port) = port_digits.parse::<u32>()
                    && port > 0
                {
                    violations.push(format!(
                        "hardcoded port {port} in URL \"{url}\" -- use `readE2EPorts()` or \
                         `readMultihopPorts()` instead"
                    ));
                }
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

        let digits: String =
            after.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '_').collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(value) = digits.replace('_', "").parse::<u32>()
            && value > 0
        {
            violations.push(format!(
                "hardcoded port literal {value} in TypeScript file -- use `readE2EPorts()` or \
                 `readMultihopPorts()` instead"
            ));
        }
    }

    // 3. Check for any known static substrate port token in a port/host context
    let forbidden_static_ports =
        [7660, 7661, 7662, 7663, 7664, 7665, 7960, 7961, 7962, 7963, 7964, 7965];
    let lower_line = line.to_ascii_lowercase();
    let has_port_context = lower_line.contains("port");

    for &static_port in &forbidden_static_ports {
        let s = static_port.to_string();
        if line.contains(&s) {
            let mut start = 0;
            while let Some(idx) = line[start..].find(&s) {
                let abs_idx = start + idx;
                let before_char = if abs_idx > 0 { line[..abs_idx].chars().last() } else { None };
                let after_idx = abs_idx + s.len();
                let after_char = line[after_idx..].chars().next();

                let is_word_char_before =
                    before_char.is_some_and(|c| c.is_alphanumeric() || c == '_');
                let is_word_char_after =
                    after_char.is_some_and(|c| c.is_alphanumeric() || c == '_');

                if !is_word_char_before && !is_word_char_after {
                    let is_preceded_by_colon = before_char == Some(':')
                        || (abs_idx >= 2 && line[..abs_idx].ends_with(":'")
                            || line[..abs_idx].ends_with(":\""));
                    if has_port_context || is_preceded_by_colon {
                        let msg =
                            format!("forbidden static port {static_port} found in TypeScript file");
                        if !violations.contains(&msg) {
                            violations.push(msg);
                        }
                    }
                }
                start = after_idx;
            }
        }
    }

    violations
}

#[test]
fn no_typescript_e2e_file_hardcodes_a_port() {
    let e2e_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e");
    let mut ts_files = Vec::new();

    // Scan tests/e2e/*.ts (global setup, configs, ports helpers)
    for entry in fs::read_dir(&e2e_dir).expect("read e2e dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("ts") {
            ts_files.push(path);
        }
    }

    // Scan tests/e2e/tests/*.ts (test spec files)
    let specs_dir = e2e_dir.join("tests");
    if specs_dir.is_dir() {
        for entry in fs::read_dir(&specs_dir).expect("read e2e specs dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("ts") {
                ts_files.push(path);
            }
        }
    }

    let mut violations = Vec::new();

    for path in ts_files {
        let contents = fs::read_to_string(&path).expect("read ts file");
        for (line_no, line) in contents.lines().enumerate() {
            let line_violations = check_typescript_line(line);
            for v in line_violations {
                violations.push(format!("{}:{}: {v}", path.display(), line_no + 1));
            }
        }
    }

    assert!(violations.is_empty(), "\n{}\n", violations.join("\n"));
}

#[test]
fn typescript_line_detection() {
    // False positive checks: DIDs and non-port numbers should NOT be flagged
    assert!(check_typescript_line("const did = 'did:key:z6MkabcQ7660xyzHash';").is_empty());
    assert!(check_typescript_line("const budgetMs = 7663;").is_empty());
    assert!(check_typescript_line("http_port = 0").is_empty());
    assert!(
        check_typescript_line("http_bind_address = \"0.0.0.0:${ports.registryPort}\"").is_empty()
    );

    // True positive checks: URLs and hardcoded ports SHOULD be flagged
    assert!(!check_typescript_line("const url = 'ws://host:19999';").is_empty());
    assert!(!check_typescript_line("const setupUrl = 'http://127.0.0.1:7660/x';").is_empty());
    assert!(!check_typescript_line("const gatewayPort = 7660;").is_empty());
    assert!(!check_typescript_line("bind_address = \"0.0.0.0:7661\"").is_empty());
    assert!(!check_typescript_line("spawn(BIN, ['--port', '7662'])").is_empty());
}

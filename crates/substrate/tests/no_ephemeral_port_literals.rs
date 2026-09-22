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
//! from `common::alloc_ports`. Five files that previously passed literal
//! port numbers to `SubstrateTestContext::setup` or `setup_with`
//! (`websocket_e2e.rs`, `miniapp_demo1_wasm_e2e.rs`, `guest_http_e2e.rs`,
//! `static_assets_e2e.rs`, and `http_passthrough_e2e.rs`) now use
//! dynamic port allocation.
//!
//! This scan checks two things in sibling `tests/*.rs` files:
//! 1. Any numeric literal assigned to an identifier containing "port" (except
//!    `port` and `container_port`).
//! 2. Any numeric literal passed as an argument to
//!    `SubstrateTestContext::setup(...)` or `setup_with(...)`.
//!
//! A hand-picked port anywhere causes test collisions. The only safe source
//! of test ports is `common::alloc_ports`.

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
/// `setup_with(...)`.
fn setup_call_port_literals_on_line(line: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let patterns = ["SubstrateTestContext::setup(", "setup_with("];
    for pat in patterns {
        let mut search_start = 0;
        while let Some(pos) = line[search_start..].find(pat) {
            let start = search_start + pos + pat.len();
            let remainder = &line[start..];
            let end_idx = remainder.find([')', '|']).unwrap_or(remainder.len());
            let args_segment = &remainder[..end_idx];
            for piece in args_segment.split(',') {
                let trimmed = piece.trim();
                let digits: String =
                    trimmed.chars().take_while(|c| c.is_ascii_digit() || *c == '_').collect();
                if !digits.is_empty()
                    && digits.len() == trimmed.len()
                    && let Ok(value) = digits.replace('_', "").parse::<u32>()
                {
                    found.push(value);
                }
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
            continue; // this file's own doc comment mentions 42_600
        }

        let contents = fs::read_to_string(&path).expect("read test file");
        for (line_no, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue; // doc comments, e.g. this file's own
            }
            let mut line_ports = port_literals_on_line(line);
            line_ports.extend(setup_call_port_literals_on_line(line));
            for value in line_ports {
                // `0` is the universal "let the OS assign a free port"
                // sentinel (`http_port = 0`), never a fixed bind.
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
    }

    assert!(violations.is_empty(), "\n{}\n", violations.join("\n"));
}

#[test]
fn setup_call_literals_detection() {
    let line1 = "let ctx = SubstrateTestContext::setup(7910, 7911, 7912).await;";
    assert_eq!(setup_call_port_literals_on_line(line1), vec![7910, 7911, 7912]);

    let line2 = "let ctx = SubstrateTestContext::setup_with(7946, 7947, 7948, |config| {";
    assert_eq!(setup_call_port_literals_on_line(line2), vec![7946, 7947, 7948]);

    let line3 =
        "let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;";
    assert!(setup_call_port_literals_on_line(line3).is_empty());
}

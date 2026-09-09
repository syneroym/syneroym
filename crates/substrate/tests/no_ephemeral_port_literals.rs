#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Guards the fix for the intermittent `AddrInUse` CI failure that started
//! in `gateway_hostname_e2e.rs`: a hardcoded backend port (`42_600u16`) fell
//! inside the OS's ephemeral port range (roughly 32768-60999 on Linux),
//! where the kernel can hand the same number to an unrelated outbound
//! socket at any moment. `common::alloc_ports` fixes that by allocating
//! below 32_768 and verifying with a real bind.
//!
//! Every `crates/substrate/tests/*_e2e.rs` file now boots through
//! `common::SubstrateNode`, which allocates every listener via
//! `alloc_ports`, so no test constructs a substrate bind address, gateway
//! port, or `PortBlock` from a numeric literal any more. This scan keeps it
//! that way: it fails on ANY numeric literal assigned to a `*port*`-suffixed
//! identifier in a sibling `tests/*.rs` file, not just one in the ephemeral
//! danger zone -- a hand-picked port anywhere is a latent cross-binary
//! collision, and the only safe source is `common::alloc_ports`.
//!
//! Deliberately dependency-free (no `regex`): a per-line heuristic textual
//! scan, not a real Rust parser, so it can both false-negative (a literal
//! it doesn't recognise as port-shaped) and, in principle, false-positive.
//! It has been checked clean against the current `tests/*.rs` tree.

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
            for value in port_literals_on_line(line) {
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

# M06D status

*Reader: developers who build this milestone.*

The plan and the slice order are in [task.md](task.md). This file records what
is done, with evidence, and which tool implemented and reviewed each slice.

| Slice | Status | Implemented by | Reviewed by | Evidence |
| --- | --- | --- | --- | --- |
| S0 | Done. Decisions, ADR-0015 A10, change-doc updates, and the iroh spike are done. | Gemini | — | `crates/coordinator_iroh/tests/relay_map_runtime.rs` (5/5 passes) and `mise run verify` pass. |
| A1 | Done. Core config (optional relay URL, CoordinatorAccess), router HomeRelay + no n0 preset, substrate HomeRelaySource and watch-channel record publisher, coordinator Iroh background logging and strict access, and E2E tests. | Antigravity | — | `crates/substrate/tests/home_relay_e2e.rs` (4/4 passes), `crates/coordinator_iroh/tests/startup_config.rs` (4/4 passes), `crates/coordinator_iroh/tests/relay_map_runtime.rs` (10/10 passes), `crates/coordinator_iroh/tests/multi_hop_relay.rs` (5/5 passes), and `mise run verify` pass. |
| A2 | Done. Curated lists (core trait, wire types, rendezvous ranking, coordinator entry and filter), community registry list serving with hop limit and parent forwarding, and substrate startup checks. | Antigravity | — | `crates/core/src/curated_list/tests.rs` (14/14 passes), `crates/community_registry/src/curated_list/tests.rs` (11/11 passes), `crates/substrate/tests/curated_list_e2e.rs` (2/2 passes), and `mise run verify` pass. |
| A3 | Done. Coordinator info wire types and endpoint (`/v1/info`), relay connection metrics and status reporting, max_relay_clients validation, and periodic registry refresh loop. | Antigravity | Claude Code | `crates/coordinator_iroh/tests/info_endpoint.rs` (4/4 passes), `crates/coordinator_iroh/src/registration/tests.rs` (2/2 passes), `crates/coordinator_iroh/tests/startup_config.rs` (6/6 passes), `crates/coordinator_iroh/tests/connection_limit.rs` (1/1 passes), `crates/core/src/coordinator_info/tests.rs` (3/3 passes), review fixes incorporated, and `mise run verify` pass. |
| A4 | Done. Substrate home-relay resolver: the `Registry` source, `coordinators.json` cache bound to its registry, 60 s `/v1/info` health probes, fail-over after 3 failures in 5 minutes, degraded mode with backoff and limited warnings, `HomeRelay::clear_unconfirmed` and `closed`, and per-hop list timeouts. Decisions `D-A4-1` to `D-A4-19` are in the change doc. | Claude Code | — | `crates/substrate/src/runtime/home_relay/resolver/tests/` (35 resolver tests, including the probe, failure-window and backoff unit tests; the first 30 were stable over 200 repeated runs), `io/tests.rs` (6/6, real local servers), `cache/tests.rs` (8/8), `home_relay/tests.rs` (7/7), `crates/router/src/home_relay/tests.rs` (8/8), `crates/core/src/curated_list/tests.rs` (`list_request_timeout_shrinks_with_each_hop`, `fetch_gives_up_after_the_time_of_its_hop`), `crates/substrate/tests/home_relay_registry_e2e.rs` (10/10), `crates/substrate/tests/home_relay_e2e.rs` (3/3). All 12 `cargo xtask verify` gates pass: fmt, clippy, file-lengths, lint-suppressions, module-layout, change-docs, duplication, roym-deps, planning-refs, doctests, audit, deny-licenses. nextest: 3052 passed, 1 skipped. Playwright e2e: 4 passed. Run from a worktree: `mise` does not trust the worktree config, so the nextest and e2e gates ran the commands of their `mise` tasks directly. |
| B1 to B3, C1 to C8 | Not started | — | — | — |

## Open in S0

None.

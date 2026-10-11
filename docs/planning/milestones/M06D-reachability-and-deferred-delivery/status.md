# M06D status

*Reader: developers who build this milestone.*

The plan and the slice order are in [task.md](task.md). This file records what
is done, with evidence, and which tool implemented and reviewed each slice.

| Slice | Status | Implemented by | Reviewed by | Evidence |
| --- | --- | --- | --- | --- |
| S0 | Done. Decisions, ADR-0015 A10, change-doc updates, and the iroh spike are done. | Gemini | — | `crates/coordinator_iroh/tests/relay_map_runtime.rs` (5/5 passes) and `mise run verify` pass. |
| A1 | Done. Core config (optional relay URL, CoordinatorAccess), router HomeRelay + no n0 preset, substrate HomeRelaySource and watch-channel record publisher, coordinator Iroh background logging and strict access, and E2E tests. | Antigravity | — | `crates/substrate/tests/home_relay_e2e.rs` (3/3 passes), `crates/coordinator_iroh/tests/startup_config.rs` (4/4 passes), `crates/coordinator_iroh/tests/relay_map_runtime.rs` (5/5 passes), `crates/coordinator_iroh/tests/multi_hop_relay.rs` (5/5 passes). |
| A2 to A4, B1 to B3, C1 to C8 | Not started | — | — | — |

## Open in S0

None.

# M06D status

*Reader: developers who build this milestone.*

The plan and the slice order are in [task.md](task.md). This file records what
is done, with evidence, and which tool implemented and reviewed each slice.

| Slice | Status | Implemented by | Reviewed by | Evidence |
| --- | --- | --- | --- | --- |
| S0 | In progress. Decisions, ADR-0015 A10 and change-doc updates are done. The iroh spike is open. | Claude Code | — | `mise run verify` passes for the docs change. |
| A1 to A4, B1 to B3, C1 to C8 | Not started | — | — | — |

## Open in S0

- The iroh spike: an endpoint with an empty relay map, then `insert_relay`. Record the result in the [registry-listed-coordinators](../../changes/registry-listed-coordinators/change.md) change doc. A1 waits for it.

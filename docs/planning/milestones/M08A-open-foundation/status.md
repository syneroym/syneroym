# M08A status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool implemented and reviewed each slice.

| Slice | Status | Implemented by | Reviewed by | Evidence |
| --- | --- | --- | --- | --- |
| S0 | In review | Gemini 3.8 Flash | Pending review | [crates/sandbox_wasm/tests/epoch_wait_credit.rs](../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs), 20/20 runs passed, PR [#329](https://github.com/syneroym/syneroym/pull/329) |
| A1 | In review | Claude Sonnet 5.5 | Pending review | PR [#331](https://github.com/syneroym/syneroym/pull/331). The parity harness still has 222 scenarios, all passing. `crates/roym_core/src/record/tests/wire_words.rs` and `crates/roym_web/ui/src/cards/wording.test.ts` keep the old words off the wire and off the cards. |
| A2a | Not started | — | — | — |
| A2b | Not started | — | — | — |
| A2c | Not started | — | — | — |
| A3 | Not started | — | — | — |
| A4 | Not started | — | — | — |
| A5 | Not started | — | — | — |
| A6 | Not started | — | — | — |
| A7 | Not started | — | — | — |

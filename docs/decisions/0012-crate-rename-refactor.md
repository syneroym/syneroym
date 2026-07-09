# ADR-0012: Workspace Crate Rename & Import Cleanup

## Status

Accepted (2026-07-09)

## Context

`crates/` had drifted to a mixed naming convention: most directories use
`snake_case` (`app_orchestration`, `coordinator_iroh`, ...) but four —
`data-layer`, `blob-store`, `key-store`, `smoke-tests` — use hyphens. Several
crate names also don't group by domain in the IDE explorer (`app_sandbox`
and `podman_sandbox` don't sort next to each other; `bindings` doesn't read
as "the WIT interface crate").

M03B (messaging) had just closed and M4 had not yet started, making this a
low-risk window to do a mechanical, workspace-wide rename before new code
lands on top of the old names.

Full execution detail lives in
[docs/planning/crate-rename-refactor.md](../planning/crate-rename-refactor.md).

## Decision

Rename six crates/directories to a consistent `domain_subdomain` /
`syneroym-domain-subdomain` scheme, in one atomic commit, then run a
workspace-wide import cleanup pass (per the `AGENTS.md` import rules) in five
follow-up batches:

| Directory (old) | Directory (new) | Crate (old) | Crate (new) |
|---|---|---|---|
| `crates/app_sandbox` | `crates/sandbox_wasm` | `syneroym-app-sandbox` | `syneroym-sandbox-wasm` |
| `crates/podman_sandbox` | `crates/sandbox_podman` | `syneroym-podman-sandbox` | `syneroym-sandbox-podman` |
| `crates/data-layer` | `crates/data_db` | `syneroym-data-layer` | `syneroym-data-db` |
| `crates/blob-store` | `crates/data_blob` | `syneroym-blob-store` | `syneroym-data-blob` |
| `crates/key-store` | `crates/data_keystore` | `syneroym-key-store` | `syneroym-data-keystore` |
| `crates/bindings` | `crates/wit_interfaces` | `syneroym-bindings` | `syneroym-wit-interfaces` |

`crates/app_orchestration` is explicitly left unchanged — its domain is
orchestration, not sandboxing.

**Revised during PR review, before merge:** the sandbox crate landed first
as `sandbox_app`/`syneroym-sandbox-app`, then was renamed again to
`sandbox_wasm`/`syneroym-sandbox-wasm` — "app" didn't distinguish the WASM
backend from the Podman backend and collided with `syneroym-rpc`'s
`NativeService` terminology; `sandbox_wasm` matches the WASM/Podman split
`system-architecture.md` already uses. This table reflects the final state;
the "Historical documents are not retroactively renamed" rule below applies
from this ADR's acceptance onward, not to revisions made while it was still
an open PR.

### WIT package names are decoupled from the Rust crate rename

WIT package identifiers (`syneroym:data-layer/store@0.1.0`,
`syneroym:blob-store/blob-store@0.1.0`, etc., referenced by
[ADR-0007](0007-data-layer-wit-interface.md) and
[ADR-0009](0009-blob-storage-object-store.md)) are **not** renamed to match.
They are a versioned component-ABI contract consumed by `test-components/*`,
which sit outside the Cargo workspace (`Cargo.toml` `exclude`) and are easy
to miss with an in-workspace search/replace. The crate directory holding a
`.wit` file is no longer required to match that file's WIT package name —
those are two independent namespaces going forward, and future crate
reorganizations should not assume they need to move together.

### Historical documents are not retroactively renamed

`docs/decisions/*.md` (this file included, and ADR-0007/0009 above) and any
`status.md`/`task.md` for a closed milestone keep the old names verbatim.
They are records of what was decided/built at the time; rewriting their
subject to match current naming would falsify the record. Only *current*
docs (architecture, requirements, the active meta-plan, `AGENTS.md`,
`GEMINI.md`) get the rename applied.

## Consequences

- All in-workspace Rust source, `Cargo.toml` references, and current-state
  docs consistently use the new names going forward.
- `test-components/data-layer-test` and the `.wit` files under
  `crates/wit_interfaces/wit/` (formerly `crates/bindings/wit/`) keep
  referencing `syneroym:data-layer/...` / `syneroym:blob-store/...` — this is
  intentional, not a leftover.
- Anyone grepping historical ADRs or closed milestone docs for a crate name
  will find the *old* name there by design; grep current docs / `crates/`
  for the current name.
- No runtime behavior changes; this is a pure rename + import-order cleanup.

# Workspace Crate Renaming & Import Cleanup

One-shot maintenance refactor to normalize crate/directory naming and enforce
`AGENTS.md` import rules across the workspace, done at the M03B → next
milestone boundary while the tree is clean.

## Goals
1. **Crate renaming** — consistent `snake_case` directories / `kebab-case`
   crate names, grouped by domain, for IDE-explorer clarity.
2. **Documentation alignment** — architecture docs, `AGENTS.md`/`GEMINI.md`,
   and *current* planning docs reflect the new names.
3. **Mandatory import cleanup** — enforce the `AGENTS.md` import rules
   (types via `use`, functions qualified by parent module, no inline
   multi-`::` paths) workspace-wide.

## Explicit non-goals
- **WIT package/interface names are not renamed.** `syneroym:data-layer/store@0.1.0`,
  `syneroym:blob-store/blob-store@0.1.0`, etc. stay exactly as-is. They are a
  versioned component-ABI contract consumed by `test-components/*`, which
  live outside the Cargo workspace (`Cargo.toml` `exclude`) and are easy to
  miss with an in-workspace search/replace. Decoupling the WIT name from the
  Rust crate name is intentional, not an oversight — the crate directory
  holding those `.wit` files may no longer match the WIT package name after
  this refactor, and that's fine.
- **No renaming inside `docs/decisions/` (ADRs) or completed/archived
  milestone docs.** ADRs and closed milestone `task.md`/`status.md` files are
  historical records of what was decided *at the time*; retroactively
  renaming their subject falsifies the record. `docs/archive/` is already
  excluded per `AGENTS.md`.
- No behavior changes. Every diff in this refactor should be mechanical
  (rename + reformat + import reorder), never a logic change.

## Phase 1: Crate Renaming — single atomic pass

Per the user's call: do all six rename groups together in one shot, then run
the full verification suite once, rather than compiling after each group.
This is appropriate here because the changes are mechanical and the compiler
is a strong safety net — a missed reference fails the build immediately
rather than silently producing wrong behavior — so bisecting mid-rename adds
process overhead without reducing real risk. This is *not* the same
tolerance we'd apply to a refactor that changes logic.

| Directory (old) | Directory (new) | Crate (old) | Crate (new) |
|---|---|---|---|
| `crates/app_sandbox` | `crates/sandbox_wasm` | `syneroym-app-sandbox` | `syneroym-sandbox-wasm` |
| `crates/podman_sandbox` | `crates/sandbox_podman` | `syneroym-podman-sandbox` | `syneroym-sandbox-podman` |

> **Revised during PR review (still pre-merge):** the first pass landed
> `sandbox_app`/`syneroym-sandbox-app`, pairing it with `sandbox_podman` on a
> `sandbox_<backend-kind>` shape. On review this was found to be vague and
> collided with existing vocabulary: "app" doesn't distinguish the WASM
> backend from the Podman backend (a `SynApp` can run in either), and
> `system-architecture.md` already labels the two backends "WASM sandbox"
> vs. "Podman container." `syneroym-rpc`'s `NativeService`/
> `NativeDispatchRegistry` also already claims "native" for the substrate's
> own in-process Rust services, unrelated to this crate. Renamed to
> `sandbox_wasm`/`syneroym-sandbox-wasm` to match the docs' existing
> WASM/Podman split. The `app_sandbox` Cargo feature flag and
> `SubstrateConfig` role name are unaffected — those stay put as
> external-facing contracts per the non-goals below.
| `crates/data-layer` | `crates/data_db` | `syneroym-data-layer` | `syneroym-data-db` |
| `crates/blob-store` | `crates/data_blob` | `syneroym-blob-store` | `syneroym-data-blob` |
| `crates/key-store` | `crates/data_keystore` | `syneroym-key-store` | `syneroym-data-keystore` |
| `crates/bindings` | `crates/wit_interfaces` | `syneroym-bindings` | `syneroym-wit-interfaces` |

`crates/app_orchestration` is unchanged (orchestration, not sandboxing — see
original rationale).

Steps, all in one commit:
1. `git mv` each directory.
2. Update `name = "..."` in each moved crate's `Cargo.toml`.
3. Update `[workspace.dependencies]` in root `Cargo.toml` (path + name).
4. Workspace-wide search/replace of the Rust identifier form
   (`syneroym_data_layer` → `syneroym_data_db`, etc.) across `**/*.rs` and
   `**/*.toml` in `crates/` and `apps/` — **not** `test-components/*`, whose
   crate names/imports are unaffected since the WIT names they bind against
   don't change.
5. Regenerate `Cargo.lock` (root workspace only — `test-components/*` have
   their own independent lockfiles and aren't touched).
6. Explicitly grep afterward for any leftover old identifiers
   (`data_layer`, `data-layer`, `blob_store`, `blob-store`, `key_store`,
   `key-store`, `app_sandbox`, `app-sandbox`, `podman_sandbox`,
   `podman-sandbox`, `syneroym_bindings`, `syneroym-bindings`) restricted to
   `crates/`, `apps/`, root `Cargo.toml` — confirm zero hits outside the
   intentionally-excluded WIT/`test-components` surface.

**Verification (once, at the end of Phase 1):**
```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
mise run test:e2e
mise run test:smoke
```
All must pass before Phase 2 begins. If they don't, fix forward within the
same phase rather than partially reverting — the rename is all-or-nothing by
design.

## Phase 2: Documentation Updates

Search/replace old directory and crate names in:
- `docs/VISION.md`, `docs/system-requirements-spec.md`,
  `docs/system-architecture.md`, `docs/developer-guide.md`
- `docs/planning/meta-implementation-plan.md`, `docs/planning/session-strategy.md`,
  `docs/planning/traceability-matrix.md`
- `docs/planning/milestones/<current-and-future-milestone>/` (i.e. any
  milestone not yet closed out — check each `status.md` for completion state
  before touching)
- `AGENTS.md`, `GEMINI.md`, `README.md`

**Excluded** (per non-goals above): `docs/decisions/*.md`,
`docs/archive/*`, and `status.md`/`task.md` for any milestone already marked
complete (e.g. confirm M01/M02/M03/M03B status before editing — closed ones
are left as historical record even if they mention old crate names).

## Phase 3: Mandatory Import Cleanup

Same batches as originally proposed — each batch gets its own commit and its
own `cargo +nightly fmt --all && cargo clippy --workspace --all-targets --all-features && cargo test --workspace`
pass, since unlike the mechanical rename, this phase touches `use` statements
and call sites by hand and benefits from smaller, bisectable diffs:

- **Batch 1:** `core`, `wit_interfaces`, `rpc`, `identity`, `data_keystore`
- **Batch 2:** `data_db`, `data_blob`, `observability`, `community_registry`
- **Batch 3:** `app_orchestration`, `sandbox_app`, `sandbox_podman`, `control_plane`
- **Batch 4:** `router`, `coordinator`, `coordinator_iroh`, `coordinator_webrtc`, `client_gateway`
- **Batch 5:** `substrate`, `sdk`, `smoke-tests`, `apps/roymctl`

Rules enforced per `AGENTS.md`:
1. Types via standard `use`.
2. Functions qualified by parent module (`use std::fs;` → `fs::read()`).
3. Remove inline fully-qualified paths (multi-`::` in code bodies).
4. Conflicting types (`Result`, `Error`) resolved by importing the parent
   module (`use std::fmt;` → `fmt::Result`).
5. `cargo +nightly fmt --all` finalizes group/sort order — don't hand-sort.

Final pass after all batches: `mise run test:all` and `cargo xtask perf-summary`
is *not* required (no performance-sensitive code paths change), but run
`mise run test:e2e` once more at the very end as a final sanity check.

## Rollback posture
Phase 1 is a single commit — if verification fails and the cause isn't
quickly obvious, revert the whole commit rather than debugging a half-renamed
tree. Phase 3 batches are independent and can be reverted individually
without affecting Phase 1 or other batches.

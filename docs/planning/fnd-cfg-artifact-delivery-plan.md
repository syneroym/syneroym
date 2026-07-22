# Deploy-Time Artifact Delivery — Spec, Design, Implementation Plan

**Requirement IDs:** `[FND-CFG]` (primary), `[FND-IAM]` (FDAE policy delivery)
**ADR:** [ADR-0019](../decisions/0019-deploy-time-artifact-delivery.md)
**Milestone placement:** interstitial within Milestone 4, executed between M04B
Slice B2 and Slice B3 — see
[meta-implementation-plan.md](meta-implementation-plan.md). This is `[FND-CFG]`
completion (M3A Slice 4 debt), not an M04B slice: M04B's charter is explicitly
"purely the new engine" and "carries no M3 debt." It is executed here because
the work is independent of B3/B4/B5 in both directions, and a breaking
`service-config` WIT change is cheapest at a quiescent point.
**Depends on:** M04B Slice B2 (shipped `fdae_policy_path` and the canonicalizing
deploy path guard), M3A Slice 4 (shipped `schema_path` and Podman env-var
config).
**Status:** In progress (2026-07-22)

---

## 1. Problem

A deploy call carries some artifacts and merely points at others, and the split
is an artifact of build order rather than a design.

### 1.1 Verified current state

Re-verified by reading, on `main` @ `1df8e4e` — not carried over from earlier
notes.

**Delivered inline today** — a client can deploy with nothing pre-staged:

| Artifact | Mechanism |
| --- | --- |
| WASM bytes | `artifact-source { url, binary }`, [control-plane.wit:4-10](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L4). The SDK mapper reads a non-URL `source` client-side into `Binary` at [mapper.rs:33-41](../../crates/sdk/src/mapper.rs#L33). |
| `custom_config` | `custom-config: option<string>`, [control-plane.wit:29](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L29), consumed at [orchestration.rs:464](../../crates/control_plane/src/service/orchestration.rs#L464). |

**Not deliverable at all** — no inline-content field exists anywhere in the WIT,
the SDK mapper, or the deploy handling:

| # | Artifact | Declaration | Consumption |
| --- | --- | --- | --- |
| 1 | JSON Schema validating `custom_config` | `schema-path: option<string>`, [control-plane.wit:31](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L31) | `fs::read_to_string` on the substrate, [orchestration.rs:475](../../crates/control_plane/src/service/orchestration.rs#L475) |
| 2 | `fdae/v1` policy document (ADR-0017) | `fdae-policy-path: option<string>`, [control-plane.wit:36](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L36) | `fs::read_to_string` on the substrate, [orchestration.rs:514](../../crates/control_plane/src/service/orchestration.rs#L514) |
| 3 | Podman volume contents | `container-volume-mapping { host-path, container-path }`, [control-plane.wit:55-58](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L55) — no content field | `resolve_host_path` + `create_dir_all` produce an **empty** sandboxed dir, bind-mounted as-is, [engine.rs:87-94](../../crates/sandbox_podman/src/engine.rs#L87) |

### 1.2 Impact

For (1) and (2), a deploy that uses them only succeeds if someone with direct
filesystem access to the substrate host staged the file out of band.

For (3) the gap is total. `custom_config` reaches a container only as flattened
`-e KEY=VALUE` env vars ([engine.rs:139-142](../../crates/sandbox_podman/src/engine.rs#L139)).
A third-party image that reads its configuration from a mounted *file* — the
common case for off-the-shelf containers — cannot be deployed through the API at
all.

### 1.3 This is partly unfinished work, not new scope

`[FND-CFG]` already specifies that the orchestrator will "serialize nested
configurations (JSON/TOML/YAML) into temporary files and mount them read-only
into the container" ([system-requirements-spec.md:958](../system-requirements-spec.md),
[system-architecture.md:1819](../system-architecture.md)), and M3A Slice 4 scoped
"Podman env-var **and file-mount** fallback"
([M03-sss/task.md:46](milestones/M03-sss/task.md)). Only the env-var half
shipped. Gap (3) is the unimplemented remainder.

---

## 2. Requirements

**R1.** A deploy client can supply a JSON Schema document's **content** in the
deploy call. Validation of `custom_config` behaves identically to today.

**R2.** A deploy client can supply an FDAE policy document's **content** in the
deploy call. Parsing, validation, persistence, and rollback behave identically
to today.

**R3.** A deploy client can supply **file content for a Podman volume**. The
substrate materializes those files inside the existing sandboxed volume
directory before the container starts.

**R4.** The existing host-side path mode remains available for all three, and is
explicitly selectable. Pointing at a file the substrate already holds is a
legitimate mode (large or shared assets, an operator-managed policy directory).

**R5.** In the author-side `SynAppManifest` TOML, a bare relative path is read
**client-side and shipped inline**, matching what WASM `source` already does.
Host-side resolution is opt-in via `{ remote_path = "..." }` under the same
key. The keys are renamed (`schema_path` → `schema`, `fdae.policy_path` →
`fdae.policy`) rather than aliased — see ADR-0019 §3.

**R6.** No new path-traversal or arbitrary-file-read surface. Inline content is
size-capped. The existing FDAE error-leakage rule is preserved.

### 2.1 End-user-visible behavior

Given an app repository:

```
my-app/
  synapp.toml
  config-schema.json
  fdae-policy.json
  nginx.conf
  service.wasm
```

```toml
[services.api]
service_type = "wasm"
source = "./service.wasm"          # already inlined today
schema = "./config-schema.json"    # NEW: inlined client-side
custom_config = '{ "port": 8080 }'

[services.api.fdae]
policy = "./fdae-policy.json"      # NEW: inlined client-side

[services.proxy]
service_type = "container"
source = "docker.io/library/nginx:1.27"
custom_config = '''
{
  "volumes": [
    {
      "container_path": "/etc/nginx/conf.d",
      "host_path": "conf",
      "files": [ { "relative_path": "default.conf", "content": "./nginx.conf" } ]
    }
  ]
}
'''
```

`roymctl svc deploy` against a **remote** substrate with nothing pre-staged now
succeeds, and `nginx` starts with its config file present and the volume mounted
read-only.

Opting into host-side resolution:

```toml
[services.api]
schema = { remote_path = "/etc/syneroym/schemas/shared.json" }
```

---

## 3. Non-Goals

Explicitly **out of scope**:

- **Redesigning `custom_config`'s env-var delivery.** The flattening at
  [engine.rs:139-142](../../crates/sandbox_podman/src/engine.rs#L139) and the
  `config_generations` store are unchanged. They are not load-bearing for this
  fix: the gap is the absence of a file-content channel, not a defect in the
  env-var one.
- **Auto-rendering the config generation into a mounted file.** A third-party
  image expects *its* format, not ours. Rejected in ADR-0019 §2.
- **Secret delivery via volumes.** `[FND-CFG]`'s `tmpfs` secret path stays
  vault-only. Inline volume files are for non-secret configuration; a deploy
  manifest is not a secret channel.
- **The OCI-style bundle and ConfigMap-style independent resources.** Recorded
  as direction in ADR-0019 §5; no resource lifecycle, digest store, or update
  propagation is built here.
- **Blob-sized artifacts.** Inline is capped at 1 MiB; `artifact-source::url`
  remains the answer for large payloads.
- **`registry-certificate`.** Already an inline `option<string>`; untouched.
- **Hot-reload of schema or policy without redeploy.** Policy remains
  last-write-wins at deploy.

---

## 4. Design

### 4.1 The WIT change

Per ADR-0019 §1-2. Breaking, applied in place, no compatibility shim — the
package is not published outside this repo (precedent: `syneroym:messaging`
between M3B and M3C).

```wit
/// Where a deploy-time text document comes from.
variant document-source {
    /// A path on the substrate host's own filesystem.
    path(string),
    /// The document's content, carried in the deploy call itself.
    inline(string),
}

record service-config {
    env: list<tuple<string, string>>,
    args: list<string>,
    custom-config: option<string>,
    quota: option<resource-quota>,
    schema: option<document-source>,          // was schema-path: option<string>
    rotation-policy: option<rotation-policy>,
    fdae-policy: option<document-source>,     // was fdae-policy-path: option<string>
}

record container-volume-file {
    /// Path relative to the volume root. Must not escape it.
    relative-path: string,
    content: document-source,
}

record container-volume-mapping {
    host-path: string,
    container-path: string,
    files: list<container-volume-file>,       // NEW; empty = today's behavior
}
```

### 4.2 Where the resolver lives

Both `control_plane` (schema, policy) and `sandbox_podman` (volume files) must
resolve a `document-source` and must apply the same traversal guard. Today
`reject_path_escape` is private to
[orchestration.rs:56](../../crates/control_plane/src/service/orchestration.rs#L56).

`syneroym-core` is the only crate both already depend on, and it deliberately
does **not** depend on `syneroym-wit-interfaces`. So the split is:

- **`syneroym-core`** gains `deploy_docs` with the security-critical, WIT-free
  half: `MAX_DEPLOY_DOCUMENT_BYTES`, `reject_path_escape` (moved),
  `read_host_document(path, field_name)` (guard + metadata size check + read),
  `check_inline_size(content, field_name)`, and
  `reject_relative_escape(relative_path, field_name)` for volume file names.
- **Each consumer** matches the two-arm variant itself — two lines — and calls
  `core` for the `path` arm. No new dependency edge, one home for the guard.

`reject_path_escape` moves rather than being duplicated; its existing unit tests
move with it. The version that moves is the canonicalizing one that arrived with
Slice B2 (it catches a symlink under the working directory pointing outside it,
which the lexical `..` check alone misses) — not the older lexical-only guard.

### 4.3 Author-side manifest model

`ServiceConfig.schema_path: Option<String>` and `FdaeManifest.policy_path:
String` ([models.rs:226](../../crates/app_orchestration/src/models.rs#L226),
[:239](../../crates/app_orchestration/src/models.rs#L239)) become a serde-level
enum that keeps the ergonomic bare-string form:

```rust
/// Author-side declaration of a deploy-time document. A bare string is a
/// path relative to the manifest, read by the client and shipped inline;
/// `{ remote_path = "..." }` defers resolution to the substrate host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum DocumentRef {
    Local(String),
    Remote { remote_path: String },
}
```

`#[serde(untagged)]` means the bare-string form stays as terse as it was, while
the same key also accepts `{ remote_path = "..." }` — so inline-vs-host-side is
one field with two arms rather than two mutually exclusive keys, mirroring the
WIT variant. The keys themselves are renamed (ADR-0019 §3): a field holding
"a path *or* content to ship inline" should not be called `_path`.

Container volumes carry the same idea through `custom_config`'s JSON, with an
author-side `VolumeSpec`/`VolumeFileSpec` pair in the mapper rather than the
generated wire record — `files` needs `#[serde(default)]` so a volume that
only wants an empty directory stays exactly as terse as it is today.

The mapper ([mapper.rs:23,28](../../crates/sdk/src/mapper.rs#L23)) resolves
`Local` via `util::read_local_artifact` — the same call WASM `source` already
uses — into `DocumentSource::Inline`, and passes `Remote` through as
`DocumentSource::Path`.

### 4.4 Podman volume materialization

Volume file content must be `inline`; the `path` arm is refused at this call
site (ADR-0019 §2). Unlike `schema`/`fdae-policy`, a volume file is copied into
a directory a caller-chosen container image can read, so a host-side read would
be an exfiltration channel rather than a convenience — and a working-directory
bound does not fix that, since that is where the substrate's own state lives.

Materialization is **staged**, not in-place:

1. Resolve and check every file first — relative-path escape, per-file cap,
   deploy-wide byte budget, duplicates. No filesystem writes yet.
2. Write the set into a sibling `<volume>.staging` directory, each file created
   with `O_CREAT|O_EXCL`.
3. Replace the live directory with one `rename`, moving the old one aside first
   so a mid-swap failure restores it rather than leaving the mount point gone.

The whole thing runs on `spawn_blocking` — it is several MiB of synchronous
file I/O, and the tokio worker it would otherwise occupy also carries router
dispatch, health, and metrics.

Three properties fall out of that shape instead of needing their own guards:

- **No partial writes.** A rejected file set writes nothing, because validation
  completes before step 2.
- **No mutation of a live mount.** A redeploy whose `podman run` later fails
  cannot strand a half-rewritten config directory under a container still
  serving from it, which the previous write-then-prune approach could.
- **No symlink redirect.** A container that planted a symlink in a
  previously-writable volume cannot redirect a later write out of it: the
  staging directory is new, and `O_CREAT|O_EXCL` fails on a symlink where
  `fs::write` follows it. Guarding only the parent directory was not enough —
  the final path component is the one that matters.

Stale-file pruning disappears entirely, along with its recursion-depth cap: the
directory is replaced whole, so the mount is exactly what the manifest
declares.

**Redeploy:** a volume with an **empty** `files` list is left strictly alone —
not replaced — because that is the scratch/data case and wiping it would
destroy whatever the container wrote. The accepted cost: a volume converted
from config back to scratch keeps its old files and reverts to a writable
mount, since a deploy cannot distinguish that transition from a plain data
volume without per-volume state we deliberately don't keep. Documented in the
developer guide rather than solved.

### 4.5 Preserved invariants

- **FDAE error leakage** ([orchestration.rs:520-531](../../crates/control_plane/src/service/orchestration.rs#L520)):
  policy validation failures log fully server-side and return a generic message,
  because the error embeds the offending document. Now covers `inline` too —
  where it matters *more*, since inline content is caller-supplied.
- **Schema validation failures** keep returning detail: the offending instance
  is the caller's own `custom_config`.
- **FDAE policy rollback** ([orchestration.rs:547-560](../../crates/control_plane/src/service/orchestration.rs#L547))
  is untouched — it operates on the resolved document string, which is exactly
  what changes source here.

---

## 5. Implementation Plan

Phases 1-6 implemented 2026-07-22 on `feat/fnd-cfg-artifact-delivery`.

### Phase 1 — `core` primitives ✅
- Add `crates/core/src/deploy_docs.rs`: `MAX_INLINE_DOCUMENT_BYTES` (1 MiB),
  `read_host_document`, `reject_relative_escape`.
- Move `reject_path_escape` out of `orchestration.rs` into it; move its tests.
- Unit tests: `..` rejection, absolute rejection, symlink-escape rejection,
  oversize rejection, relative-escape rejection (`..`, absolute, prefix).

### Phase 2 — WIT + generated-type fallout ✅
- Apply the §4.1 WIT change.
- Update every `ServiceConfig`/`ContainerVolumeMapping` construction site:
  `crates/sdk/src/{mapper.rs,lib.rs}`,
  `crates/control_plane/src/{service.rs,service/orchestration.rs}`,
  `crates/app_orchestration/src/{models.rs,journal.rs,reconcile.rs,catalog.rs}`,
  `crates/substrate/tests/http_passthrough_e2e.rs`.
- Confirm `wasm32-wasip2` still builds.

### Phase 3 — control-plane consumption ✅
- `schema` and `fdae-policy`: resolve the variant, then run today's validation
  paths unchanged.
- Keep the `spawn_blocking` boundary for the `path` arm; `inline` needs no
  blocking hop.

### Phase 4 — Podman volume files ✅
- Materialization + `:ro` + stale-file cleanup per §4.4.

### Phase 5 — author manifest + SDK mapper ✅
- `DocumentRef` in `models.rs`; mapper resolution per §4.3.
- `roymctl` surface check: confirm no CLI flag change is needed.

### Phase 6 — tests, docs, verification ✅
- Integration test: deploy with inline schema + inline policy against a
  substrate whose working directory contains neither file.
- Integration test: container deploy with inline volume file → file present,
  mount read-only.
- Negative tests: oversize inline, escaping `relative_path`, both traversal
  guards.
- Regression: `remote_path` still resolves host-side; empty `files` still yields
  a writable empty dir.
- Docs: `developer-guide.md` manifest reference; traceability matrix
  `[FND-CFG]` row; meta-implementation-plan interstitial entry; M04B `task.md`
  pointer.
- Gate: `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
  --all-features`, `cargo test --workspace`, `mise run test:e2e`.

---

## 6. Failure and Security Tests

| Scenario | Expected |
| --- | --- |
| `path` with `..` | Deploy rejected, no read |
| `path` absolute | Deploy rejected, no read |
| `path` via symlink escaping cwd | Deploy rejected, no read |
| Inline document > 1 MiB | Deploy rejected before parse |
| Local document > 1 MiB | Rejected client-side, before the RPC |
| Volume `relative_path` with `..` / absolute / prefix | Deploy rejected, nothing written |
| Volume file content via `path` | Rejected — inline only, no host read |
| Symlink planted in a writable volume, then a deploy naming that file | Write lands in the volume, symlink target untouched |
| Volume file set over the deploy-wide budget | Rejected, nothing written (no partial set) |
| Duplicate `relative_path` in one volume | Rejected |
| Failed materialization on a redeploy | Live volume byte-identical to before |
| Invalid inline FDAE policy | Deploy fails; generic message to caller, full detail server-side only |
| Invalid inline schema | Deploy fails; violation detail returned (caller's own config) |
| Redeploy with shrunk `files` | Removed files absent from the mount |
| Malformed `volumes` or `ports` JSON | Deploy fails loudly, neither silently dropped |

Added after review (findings 1-5, 8-10). The `path`-arm refusal, the symlink
redirect, the partial-write case, and the shared budget each have a dedicated
test in `sandbox_podman`; the client-side cap and the strict `volumes`/`ports`
parsing in `sdk`.

## 7. Exit Criteria

1. A deploy client supplies schema, FDAE policy, and Podman volume file content
   in the deploy call, against a substrate with nothing pre-staged (R1-R3).
2. `remote_path` still resolves host-side for all three (R4).
3. Bare manifest paths inline client-side (R5).
4. Every §6 row is covered by a test (R6).
5. Full gate green; `wasm32-wasip2` unbroken.

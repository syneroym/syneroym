# ADR-0019: Deploy-Time Artifact Delivery

**Status**: Accepted (2026-07-22). Surfaced while reviewing the control-plane
deploy path after M04B Slice B2 Phase 4 shipped `fdae_policy_path`.

**Context**:

A deploy call carries some of its artifacts and merely *points at* others. The
split is not a design — it is an accident of the order things were built.

**Carried in the call** (a client can deploy with nothing pre-staged on the
substrate host):

- WASM bytes — `artifact-source` already offers `binary(list<u8>)` alongside
  `url(string)`
  ([control-plane.wit:4-10](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L4)),
  and the SDK mapper resolves a non-URL `source` by reading the local file
  client-side into `Binary`
  ([mapper.rs:33-41](../../crates/sdk/src/mapper.rs#L33)).
- `custom-config` — `option<string>`
  ([control-plane.wit:29](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L29)),
  consumed at
  [orchestration.rs:464](../../crates/control_plane/src/service/orchestration.rs#L464).

**Assumed to already exist on the substrate host's filesystem**, with no upload
path anywhere in the WIT, the SDK mapper, or the deploy handling (verified by
reading, not assumed):

1. `schema-path: option<string>`
   ([control-plane.wit:31](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L31))
   — the JSON Schema validating `custom-config`, read via `fs::read_to_string`
   on the substrate at
   [orchestration.rs:475](../../crates/control_plane/src/service/orchestration.rs#L475).
2. `fdae-policy-path: option<string>`
   ([control-plane.wit:36](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L36))
   — the `fdae/v1` policy document ([ADR-0017](0017-fdae-policy-schema-and-compilation.md)),
   read at
   [orchestration.rs:514](../../crates/control_plane/src/service/orchestration.rs#L514).
3. Podman `container-volume-mapping`
   ([control-plane.wit:55-58](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L55))
   — `host-path`/`container-path` only, no content field. `deploy` resolves the
   host path into a sandboxed directory under `containers_dir`, `create_dir_all`s
   it, and bind-mounts it *empty*
   ([engine.rs:87-94](../../crates/sandbox_podman/src/engine.rs#L87)).

Consequence: a deploy that needs any of the three only succeeds if someone with
direct filesystem access to the substrate host staged the file out of band —
which defeats the purpose of a deploy API. For (3) the effect is total: a
container image that reads its configuration from a mounted *file* rather than
the `-e KEY=VALUE` env vars `custom-config` is flattened into
([engine.rs:139-142](../../crates/sandbox_podman/src/engine.rs#L139)) cannot be
deployed through the API at all.

(3) is not a new capability. `[FND-CFG]` already specifies that the orchestrator
will "serialize nested configurations (JSON/TOML/YAML) into temporary files and
mount them read-only into the container"
([system-requirements-spec.md:958](../system-requirements-spec.md),
[system-architecture.md:1819](../system-architecture.md)), and M3A Slice 4 scoped
"Podman env-var **and file-mount** fallback"
([M03-sss/task.md:46](../planning/milestones/M03-sss/task.md)). Only the env-var
half shipped. This ADR closes the other half.

**Decision**:

## 1. One `document-source` variant, reused everywhere

Inline content and host-side path become two arms of a single variant rather
than parallel optional fields:

```wit
/// Where a deploy-time text document comes from.
variant document-source {
    /// A path on the substrate host's own filesystem, resolved relative to
    /// its working directory and guarded against traversal.
    path(string),
    /// The document's content, carried in the deploy call itself.
    inline(string),
}
```

`service-config` drops `schema-path` and `fdae-policy-path` in favour of:

```wit
schema: option<document-source>,
fdae-policy: option<document-source>,
```

**Why a variant and not sibling `*-inline` fields.** Two optional strings make
"both set" and "neither set but meant one" representable, so every consumer must
hand-write an exactly-one-of check and every producer must remember it. The
variant makes the invalid state unconstructible, which is the same reason
`artifact-source` is already a variant rather than `url: option<string>` +
`binary: option<list<u8>>`. Reusing one type across three call sites also means
the traversal guard, the size cap, and the error vocabulary are written once.

This is a **breaking WIT change**. That is acceptable and deliberate: the
package is never published outside this repository, and the project's standing
position is to change schema and behavior in place rather than carry
compatibility shims — the same reasoning recorded for `syneroym:messaging`
between M3B and M3C ([M03B-messaging/task.md](../planning/milestones/M03B-messaging/task.md),
"WIT Boundary Versioning", Finding A3).

## 2. Podman volumes gain per-file content, mounted read-only

`container-volume-mapping` gains a `files` list:

```wit
record container-volume-file {
    /// Path relative to the volume root. Must not escape it.
    relative-path: string,
    content: document-source,
}

record container-volume-mapping {
    host-path: string,
    container-path: string,
    files: list<container-volume-file>,
}
```

When `files` is non-empty the engine writes each entry beneath the already
sandboxed volume directory and mounts the volume `:ro`, satisfying `[FND-CFG]`'s
"mount them read-only". When `files` is empty the behavior is exactly today's:
an empty, writable directory — so existing scratch and data volumes are
untouched.

A list of files rather than one `content` field, because a container's config
directory is routinely more than one file (`config.yaml` + `certs/ca.pem`), and
because a single `content` field would have to switch the mount between
file-mount and dir-mount semantics based on whether it is set.

**Not decided here**: rendering the flattened config generation into a file
automatically. The blocker is that a third-party image expects *its* file
format, not ours; a conventional auto-rendered JSON path solves a problem nobody
has. `custom-config`'s existing env-var flattening is unchanged.

## 3. The author-side manifest inlines by default

In the `SynAppManifest` TOML, a bare relative path now means "read this file
next to my manifest and ship it inline":

```toml
[services.my-svc]
schema = "./config-schema.json"          # read client-side, shipped inline

[services.my-svc.fdae]
policy = "./fdae-policy.json"            # read client-side, shipped inline
```

Pointing at a file the substrate already holds becomes explicit, under the
same key:

```toml
[services.my-svc]
schema = { remote_path = "/etc/syneroym/schemas/shared.json" }
```

**The keys are renamed**, `schema_path` → `schema` and `policy_path` →
`policy`, rather than keeping the old spellings as serde aliases. A field
holding "a path, or content to ship inline" should not be called `_path`, and
the alternative — accepting both spellings forever — is the compatibility shim
this project's standing position rejects while the product is unreleased. The
cost is that existing manifests must rename two keys; there is no version
ladder and no dual code path.

**Why inline is the default.** It is what WASM `source` already does at
[mapper.rs:33-41](../../crates/sdk/src/mapper.rs#L33) — an author writing
`source = "./service.wasm"` does not expect the substrate to look for that path
on its own disk, and there is no reason `schema_path` should differ. The common
case is a schema and a policy living beside the manifest in the app's own
repository, and that case must work against a remote substrate with nothing
pre-staged.

The host-side path arm is kept, not deprecated: large or shared assets and an
operator-managed policy directory are legitimate, and an operator who wants
policy documents to live under their control rather than in each app's manifest
should be able to say so.

## 4. Limits and validation

- **Size cap.** `MAX_DEPLOY_DOCUMENT_BYTES` (1 MiB) bounds a single document,
  and it applies to **both** arms for two different reasons: inline, because a
  deploy manifest is not a blob store (`artifact-source::url` exists for
  anything large); `path`, because the path is chosen by a remote caller, so an
  unbounded `read_to_string` is a memory-exhaustion lever. Host-side reads
  check file metadata before reading, so an oversized file is never loaded. A
  volume's file set is additionally capped in aggregate, since a per-file cap
  alone still permits many files.
- **Traversal.** `path` keeps the existing `reject_path_escape` +
  filesystem-resolving check. `relative-path` on a volume file is rejected if it
  contains `..`, a root, or a prefix component, in addition to the existing
  `resolve_host_path` sandboxing — belt and braces, because the two guards catch
  different things.
- **Error leakage.** The rule established for `fdae-policy-path` at
  [orchestration.rs:520-531](../../crates/control_plane/src/service/orchestration.rs#L520)
  is unchanged and now also covers `inline`: a policy validation failure logs in
  full server-side and returns a generic message, because the underlying error
  embeds the offending policy document. Schema validation failures still return
  detail, because there the offending instance is the caller's own
  `custom-config`.

## 5. Recorded direction: toward a deploy bundle

This ADR deliberately stops short of the larger manifest redesign, but records
the direction it is shaped to grow into, so the next step is an extension rather
than a rewrite.

**The end-state.** A deploy manifest becomes a *bundle*: a content-addressed set
of named artifacts (WASM, schema, policy, volume files, static assets) plus a
manifest that references them by logical name, built from files spread across a
project by a `roymctl` packaging step — the OCI-image-build analogue. Every
artifact then travels one road, and the substrate can dedupe and cache by digest
instead of re-receiving unchanged bytes on every redeploy.

**The other shape considered** — independently-updatable resources in the
Kubernetes idiom (`SchemaMap`, `PolicyResource`) that a service references by
name and that can be updated without redeploying it — is not rejected, but it is
a *lifecycle* change, not a delivery change: it needs its own identity,
versioning, garbage-collection, and update-propagation story (what happens to a
running service when its policy resource changes?). The FDAE policy row is
already last-write-wins per service
([orchestration.rs:547-560](../../crates/control_plane/src/service/orchestration.rs#L547)),
which is the primitive such a resource would build on. Deferred until there is a
concrete need; delivery parity does not depend on it.

**Why `document-source` is the right first step for either.** Both end-states
require the substrate to accept artifact *content* over the wire and to have a
single place where "where did this document come from" is answered. This ADR
builds exactly that, at three call sites, without inventing a resource
lifecycle. A bundle adds a third arm (`digest(string)` resolved against the
bundle's content-addressed store) to a variant that already exists, and the
consumers keep working.

**Consequences**:

- Deploy clients reach parity across all deploy-time artifacts: schema, FDAE
  policy, and container config files can all be supplied in the deploy call,
  matching WASM bytes and `custom-config`.
- `[FND-CFG]`'s Podman file-mount half, specified in M3A and never implemented,
  is closed.
- The WIT change is breaking; `service-config` producers and consumers are
  updated in place across the workspace in the same change. No compatibility
  shim, no field ladder.
- Inline documents make deploy payloads larger. Bounded by the size cap, and the
  `path` and `url` arms remain for anything that should not travel inline.
- A substrate operator who relied on `schema_path` resolving against the
  substrate's filesystem from a bare manifest path must now say so explicitly
  with `remote_path`. This is a user-visible behavior change, taken because the
  previous default was the broken one.
- Manifests must rename `schema_path` → `schema` and `fdae.policy_path` →
  `fdae.policy`. A bare string still parses, so only the key changes.

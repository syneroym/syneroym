# ADR-0008: Service Configuration Host Function Contract (D-03-03)

## Status

Accepted

## Context

`[FND-CFG]` requires WASM services to receive versioned, non-secret configuration
through a typed host function rather than environment variables or config files.
The host function must provide immutable generation semantics: a single WASM
invocation always sees the configuration generation that was active when the
invocation began, even if a new deployment occurs mid-execution.

Two design questions required resolution:

1. **Missing key semantics:** Should a missing key return `Err(not-found)` or
   `Ok(None)`?
2. **Generation versioning model:** Is the configuration generation pinned at
   invocation start (immutable per-invocation) or resolved on each `get` call
   (latest generation)?

## Decision

### Function Signature

```wit
interface app-config {
    variant config-error {
        internal(string),
    }

    /// Returns the value for `key` in the active configuration generation.
    /// Returns Ok(none) if the key does not exist (not an error condition).
    get: func(key: string) -> result<option<string>, config-error>;

    /// Returns all key-value pairs whose key starts with `prefix`.
    /// Returns an empty list if no keys match.
    get-section: func(prefix: string) -> result<list<tuple<string, string>>, config-error>;
}
```

`config-error` contains only `internal(string)` — a missing key is `Ok(None)`,
not an error. This allows callers to cleanly apply defaults without error-path
handling for a completely normal application state.

### Generation Versioning

Configuration is **pinned at invocation start**. When the host creates a new
`Store` for a WASM invocation, it reads the current maximum generation for that
service from `substrate.db` and stores it in `HostState.config_generation`. All
`config/get` and `config/get-section` calls within that invocation read from
exactly that generation. A concurrent re-deployment that bumps the generation
does not affect in-flight invocations.

### Resolution Source

Configuration is resolved from a `config_generations` table in `substrate.db`:

```sql
CREATE TABLE IF NOT EXISTS config_generations (
    service_id  TEXT    NOT NULL,
    generation  INTEGER NOT NULL,
    config_blob TEXT    NOT NULL,  -- JSON-encoded flat key-value map
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (service_id, generation)
);
```

On `deploy`, the orchestrator:
1. Flattens the service's `custom_config` from the manifest into a JSON map of
   `{ "key": "value" }` pairs. Nested TOML structures are dot-flattened
   (e.g., `[db] host = "localhost"` becomes `"db.host": "localhost"`).
2. Inserts a new row with `generation = COALESCE(MAX(generation), 0) + 1`.
3. Sets `HostState.config_generation = current_generation` for every new
   invocation of that service.

### Schema Validation

If `config.schema` is set, the orchestrator validates `custom_config` against
the referenced JSON Schema using the `jsonschema` crate before writing the
generation row. Deployment fails with a structured error listing all
violations. Validation fires at deploy time, not at runtime.

> **Amended by [ADR-0019](0019-deploy-time-artifact-delivery.md) (2026-07-22).**
> This field was `schema_path: option<string>` — a path on the substrate host,
> with no way to upload the document, so a schema could only be used if
> someone staged it out of band. It is now `schema: option<document-source>`,
> carrying either inline content or a host-side path. Nothing about the
> validation itself changed.

### Podman Compatibility

For `container` service types, the orchestrator resolves the active generation
and injects non-secret values as environment variables into the Podman container
spec. The mapping follows dot-notation to `UPPER_SNAKE_CASE` conversion
(e.g., `db.host` → `DB_HOST`).

Secrets from the vault are injected per the manifest's `secret_mode` field:
- `"tmpfs"` (recommended): mounted as a `tmpfs` file inside the container.
- `"env"` (degraded): injected as environment variable. The host logs a
  persistent `warn!` on every deployment when this mode is active:
  `"Degraded secret isolation: secret '<key>' injected as env var for Podman
  container <service_id>"`.

### Out-of-Band Secret Rotation Policy

`ServiceManifest.config.rotation_policy`:
- `"restart-on-rotation"` (default): orchestrator queues a graceful restart of
  the service when a vault secret is updated out-of-band.
- `"none"`: no automatic restart; the service reads the new secret on its next
  invocation.

## Consequences

- `HostState` gains `config_generation: u64` and `service_id: String` fields
  (the latter was already present; the former is new).
- The `config_generations` table must be created in the `substrate.db` migration
  in Slice 4, alongside the M3A schema version bump.
- `get-section` with an empty prefix returns all keys in the generation — callers
  must be aware this can return a large map for services with many config keys.
- The `jsonschema` crate must be added to `Cargo.toml` workspace dependencies.

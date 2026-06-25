# D-02-05: WASM Fuel Quota Manifest Schema

**Status**: Accepted

**Context**: 
Requirement `[FND-SEC]` requires Wasmtime fuel metering for `max_instructions` derived from the SynApp manifest. The `SynAppManifest` and `ServiceManifest` types in `crates/app_orchestration` currently lack resource quota fields.

**Decision**: 
We will define a `ResourceQuota` struct with `max_instructions: Option<u64>` and `max_memory_bytes: Option<u64>`. These quotas will apply strictly per-invocation to natively leverage `wasmtime`'s `store.set_fuel()` mechanism. If unset in the manifest, the system will fall back to conservative substrate-global defaults (e.g., 10B instructions, 256MB memory) defined in `SubstrateConfig`.

**Consequences**: 
- **Enables**: Immediate protection against infinite loops and excessive memory allocation from a single WASM invocation without requiring complex state-tracking across time windows.
- **Defers**: Per-service time-window rate limits (e.g., max instructions per hour), which would require persistent state tracking.

**Implementation Notes**: 
- Update `crates/app_orchestration/src/models.rs` to include `pub quota: Option<ResourceQuota>` in `ServiceManifest`.
- Enable `config.consume_fuel(true)` in `crates/app_sandbox/src/engine.rs` and apply the per-invocation limit.

# D-04-05: Native-Dispatch Identity Threading

**Status**: Accepted

**Context**:

Milestone 4A must close the tracked M3B/M3C native-dispatch authentication gap
(see `docs/planning/milestones/M04A-proxy-and-auth-foundation/task.md`, gate
item #1). Two defects exist today, confirmed by code read on `main`:

1. **Conditional identity check.** `RouteHandler::handle_stream`
   (`crates/router/src/route_handler/io.rs:90-92`) only calls
   `HandshakeVerifier::verify_preamble` inside
   `if preamble.delegation.is_some()`. With no delegation on the connection,
   **no identity check runs**, and native-capability interfaces never require
   one.
2. **No downstream identity plumbing.** `NativeInvocation`
   (`crates/rpc/src/native.rs:15-18`) carries only `interface`/`method`/`params`.
   `NativeService::dispatch(&self, invocation) -> RpcResult<NativeResponse>`
   therefore never receives a caller. Consequently
   `SynSvcNativeService::dispatch` (`crates/control_plane/src/synsvc_native.rs:588`)
   passes `&self.service_id` — the **DB owner**, not the caller — as
   `creator_id` into the data layer (`do_put(..., &self.service_id)`). There is
   no distinct caller identity anywhere below dispatch.

Two `TODO(M4)` gates currently stand in for real authorization: the guest-side
`execute-ddl` gate (`crates/sandbox_wasm/src/host_capabilities.rs:452-463`, keyed
on `is_init_context`) and the native-side mirror
(`crates/control_plane/src/synsvc_native.rs:309-316`).

This ADR decides **how a verified caller identity is threaded through native
dispatch**, what the "Admin capability" gate concretely is, and how identity
survives a cross-node proxy hop. It is co-designed with
[D-04-01](0015-ucan-capability-model.md), which defines the `SessionContext` and
capabilities this ADR carries.

**Decision**:

1. **Add a `caller: CallerContext` field to `NativeInvocation`** (not a second
   `dispatch` parameter). `NativeInvocation` is an in-process struct constructed
   by the router after parsing (it is *not* the wire type — `crates/rpc/src/converter.rs`
   bridges JSON↔native), so folding identity into it threads cleanly through the
   existing `dispatch_data_layer`/`dispatch_vault`/`dispatch_app_config`/
   `dispatch_blob_store`/`dispatch_messaging` fan-out without adding a parameter
   to each helper. The trait becomes, unchanged in arity:
   `async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse>`
   with `invocation.caller` now available to every arm.

2. **`CallerContext` shape:**
   ```rust
   pub struct CallerContext {
       pub caller_did: String,             // verified did:key of the immediate caller
       pub app_instance: Option<String>,   // app-instance the caller acts as (creator_id, per-app KEK)
       pub session: SessionContext,        // verified capabilities/claims (D-04-01)
       pub auth: AuthLevel,                 // how identity was established
   }
   pub enum AuthLevel {
       Delegated,   // verified DelegationCertificate only (pre-UCAN / transport identity)
       Ucan,        // full verified UCAN capability chain (B1)
       LocalElevated, // substrate-injected lifecycle context (init/migrate), carries Admin capability
   }
   ```
   At Slice B0, `caller_did`/`app_instance`/`auth` are populated from the
   (now-mandatory) handshake; `session.capabilities` may be empty until B1 wires
   full UCAN verification. `CallerContext` lives in `syneroym-rpc` (alongside
   `NativeInvocation`); `SessionContext` is re-exported from `syneroym-ucan`.

3. **Make `verify_preamble` mandatory.** Remove the
   `if preamble.delegation.is_some()` gate in `io.rs`: every native-capability
   dispatch and the HTTP-bridge call path (`crates/router/src/route_handler/http.rs`)
   must resolve a `CallerContext` before dispatch, or the stream is rejected
   before reaching the registry lookup. An unauthenticated connection to a
   native-capability interface fails closed.

4. **`creator_id` becomes the caller, not the service.** Replace
   `do_put(..., &self.service_id)` and siblings with `invocation.caller.caller_did`
   (or the resolved `app_instance` identity), so records are attributed to the
   actual writer — the precondition FDAE (D-04-02) filters against.

5. **The Admin-capability gate replaces `is_init_context`.** Both `TODO(M4)`
   sites check `invocation.caller.session` for `data-layer/admin` on the target
   service resource (D-04-01), via a helper `CallerContext::has_capability(&self,
   resource, ability) -> bool`. Lifecycle init/migrate runs with
   `AuthLevel::LocalElevated` and a substrate-injected token bearing
   `data-layer/admin`, so `execute-ddl` continues to work for init while being
   denied to ordinary callers — same outcome as `is_init_context`, now
   capability-driven and uniform across guest and native paths.

6. **Cross-node hop: send the proof, re-verify at the destination.** A proxied
   call (M04A Slice A1) carries the caller's DID **and its UCAN token(s) /
   delegation proof** in the request *envelope metadata* (never in `params`, and
   never a raw trusted capability list). The receiving — data-owning — node
   verifies those tokens locally and constructs a **fresh** `CallerContext`
   before dispatch. Capabilities are never trusted across the wire; only signed
   proofs are, and enforcement always happens at the node that owns the resource.

**Consequences**:

- **Enables**: closes gate items #1 and #4 in one slice (B0); a single
  authenticated request object flowing through all native dispatch; spoof-proof
  `creator_id`; a uniform Admin gate across guest and native `execute-ddl` and
  `query-raw` (B5); the identity substrate FDAE (M04B) requires.
- **Breaking (internal only)**: `NativeInvocation` gains a required field; every
  `NativeService` impl (`SynSvcNativeService`, `ControlPlaneService`'s `security`
  interface) and every construction site recompiles together. No WIT surface
  changes, so no version-compat shim.
- **Defers**: rich per-method caveat evaluation (D-04-02 FDAE policy); the full
  UCAN chain population of `session.capabilities` (B1) — B0 may ship with an
  interim `[iam].admin_ucan_root` allowlist check for the Admin gate if B1 lands
  after B0.

**Implementation Notes**:

- Edit targets: `crates/rpc/src/native.rs` (add `caller`, define `CallerContext`/
  `AuthLevel`), `crates/router/src/route_handler/io.rs` (mandatory verify),
  `.../http.rs` (same on the bridge path), `crates/control_plane/src/synsvc_native.rs`
  (thread `invocation.caller`; replace `creator_id`; Admin gate at :309-316),
  `crates/sandbox_wasm/src/host_capabilities.rs:452-463` (Admin gate replaces
  `is_init_context`), and the `is_init_context` field/compute in
  `crates/sandbox_wasm/src/engine.rs:547,594,630` (removed or subsumed by
  `LocalElevated`).
- The single most important new test: an unauthenticated peer's
  `data-layer`/`messaging`/`blob-store`/`vault`/`app-config` call and
  HTTP-bridge request are all rejected; an authenticated caller's identity
  reaches `dispatch_data_layer` and becomes `creator_id`.

**Alternatives considered**:

- **Second `dispatch` parameter** (`dispatch(&self, invocation, caller:
  &CallerContext)`) instead of a field. Cleaner what/who separation, but forces
  the extra parameter through every internal fan-out helper and every call site;
  the field threads through the existing helpers unchanged and matches the arch
  doc's request-scoped `SessionContext` framing (`system-architecture.md:1823`).
  Rejected on ergonomics.
- **Serialize `CallerContext` (capabilities included) across the proxy hop and
  trust it.** Rejected: it would let a peer assert arbitrary capabilities. Only
  signed tokens cross the wire; the destination re-verifies (enforce at the
  data-owning node).

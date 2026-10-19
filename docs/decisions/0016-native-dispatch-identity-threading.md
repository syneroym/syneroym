# D-04-05: Native-Dispatch Identity Threading

**Status**: Accepted (amended 2026-08-27 and 2026-10-19 — see the dated
amendment notes at the end of **Decision**)

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
   **Note (M04A Slice B6):** the sketch above's `app_instance` comment
   ("per-app KEK") predates B6's derivation model. B6 derives each service's
   KEK from the bound `service_id` (== `app_instance` today) at the storage
   layer (`syneroym_data_keystore::key_store`), not by reading this field —
   see `plans/B6.md` §2 and `crates/rpc/src/native.rs`'s current doc comment.

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

**Amendment (2026-08-27, [ADR-0024](0024-client-gateway-identity-and-auth-service.md)).**
For **gateway-origin traffic only**, the caller identity no longer arrives as
a delegation certificate in the route preamble. ADR-0024 makes the client
gateway a dumb proxy and moves person identity to a node **auth service**: the
caller presents a short-lived UCAN session token in the `syneroym_session`
cookie, signed by the auth service's key, and each service verifies that token
itself (signature, `exp`, subject) rather than reading a preamble delegation
the gateway minted.

Concretely, for that path:

- `AuthLevel::Delegated` as *"the client gateway vouched for this person"* is
  replaced by *"a valid session token is present, subject = the person's
  **master DID**"*. The `CallerContext` a gateway-origin request resolves to is
  built from the verified token, not from `verify_preamble`'s delegation.
- The token's `fct` claims (ADR-0024 §3) are the account attributes the auth
  service vouches for. A service trusts them exactly as far as it trusts the
  auth service's key — the same trust relationship §1's delegation cert had,
  moved from the transport preamble to an application-layer cookie.
- A shared "verify session token" helper serves both native and WASM services,
  so the check is one implementation rather than per-service.

**Everything else in this ADR is unchanged.** Direct peer connections,
`roymctl`, and cross-node proxied calls still establish identity through the
handshake and the preamble; §3's *"`verify_preamble` is mandatory"* rule,
§4's `creator_id`, §5's Admin-capability gate, and §6's send-the-proof /
re-verify-at-the-destination rule all stand as written. The transport-level
identity the gateway itself puts on the preamble becomes a function of the
gateway's configured `identity_mode` (ADR-0024 §1): the gateway's own node key
in `login` mode, a configured person delegation in `fixed` mode, nothing in
`open` mode.

Landing slice: **M06C C1.1** —
[slice-c1.1-implementation-plan.md](../planning/milestones/M06C-roym-product/slice-c1.1-implementation-plan.md).

**Amendment 2 (2026-10-19, signed route preamble).**
§3 and §6 change as follows. Everything else in this ADR, and the 2026-08-27
amendment, stands.

1. **A caller proves that it holds its key.** The caller signs
   `syneroym-preamble-v1\n<service_id>\n<interface>\n<ts>` with the Ed25519
   key in `pubkey`. When the preamble has a handshake key (`epk`), that key is
   added at the end. The preamble carries `ts` (Unix seconds) and `sig`.
   `verify_preamble` refuses a preamble that has a `pubkey` and no valid
   signature, and a `ts` more than 15 minutes from its own clock. This holds
   with and without a delegation certificate.
2. **`pubkey` is only the identity key.** The P-256 key of the end-to-end
   handshake moves to its own field, `epk`, inside the signed bytes. A hop
   that swaps it breaks the signature.
3. **Each node presents its own identity; the original caller's proof
   travels in `on_behalf`.** This replaces "send the proof" in §6 for a proof
   that the sending node does not hold the key for. A node signs as itself
   (its service instance key or its node key). It passes the original
   caller's signed proof unchanged in `on_behalf`. Each later service passes
   the same value on. The destination checks both, and sets `anchor_did` to
   the original caller. Trust rule: a caller trusts the services its call
   passes through. A preamble carries `on_behalf` or `ucan`, not both. A
   guest never sends `on_behalf`.
4. **Only coordinators forward.** On a local registry miss, a coordinator
   forwards the stream to the host. Any other node refuses it
   (`not_hosted_here`).
5. **A call through a forwarder is encrypted end to end.** A client uses the
   handshake when the endpoint it dials is not the host named in the target's
   record, or when it does not know the host. A direct QUIC connection needs
   no handshake.

Why: without proof of possession, a delegation certificate is a bearer
credential. A service sends its instance certificate on every remote call, so
every node it calls could act as that service. The time window keeps the
check stateless. A preamble can be used again inside the window only by a
party that reads it in plaintext: a forwarder before rule 5, or a reader of
`debug` logs.

Rejected: a nonce and replay cache (state for a rare case), a server
challenge (one more round trip on every stream), and binding the proof to the
QUIC connection (breaks forwarders).

Landing slices: **M06D C4b** (rules 1 to 4) and **M06D C4c** (rule 5) —
[signed-route-preamble change doc](../planning/changes/signed-route-preamble/change.md).

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
  `crates/sandbox_wasm/src/engine.rs:547,594,630` (removed or replaced by
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

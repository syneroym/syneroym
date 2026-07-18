# Milestone 4A: Universal Proxy & Auth Foundation (M04A-proxy-and-auth-foundation)

> **Provenance.** Split on 2026-07-13 from a combined M4 planning draft along
> the **capability-plumbing vs. data-aware-policy-engine** boundary. The FDAE
> policy engine (the pushdown sieve, RLS/CLS, the 4-stage pipeline, stage-4
> ABAC) is the sibling milestone
> [M04B-fdae-policy](../M04B-fdae-policy/task.md). All code anchors below were
> re-verified against `main` @ `6c6e859` (after the #64 file-split cleanup).
>
> **Why this milestone exists as its own slice-set.** It is the ADR-light,
> buildable-now half of M4 *and* it independently closes the tracked M3
> security debt: by the end of M04A every native-capability call and the HTTP
> bridge require a verified caller identity, DDL/raw-SQL are admin-gated, and
> cross-node calls have a typed Universal Proxy. FDAE (M04B) then layers
> data-centric row/column filtering on top of the identity/capability
> foundation this milestone lays.

## Goal

Give cross-node component/service calls a typed **Universal Proxy over JSON-RPC
/ Iroh QUIC** — with full WIT⇄JSON value conversion so calls are genuinely
typed at the dispatch boundary (not string-wrapped JSON) — and establish the
**authentication + capability-admission foundation**: a verified, unspoofable
caller identity threaded through every native-capability dispatch, UCAN context
extraction at ingress, and an Admin-UCAN capability replacing today's
`is_init_context` scaffold.

JSON-RPC is the **uniform inter-service wire** for M4 (WASM↔WASM, WASM↔native);
wRPC as a binary wire-efficiency + type-fidelity optimization is deferred
(Decision Register A.5), as is protocol negotiation (A.7). WASM↔Podman/TCP
proxy targets are **not** on this wire yet — Slice A1 returns a typed
`unsupported-target` error for them (Flag F4); there is no framing/JSON-RPC
contract for a `SubstrateEndpoint::TcpHostPort` service today (TCP endpoints
are byte-passthrough proxies only). Adding it later is one match arm in
`ProxyRouter::invoke_local`.

This milestone does **not** implement the FDAE relational policy engine — that
is M04B. M04A delivers Tier 0–2 enforcement (context init, service admission,
method/argument + admin admission); M04B adds Tier 3 (data-plane RLS/CLS).

---

## Requirement IDs (Traceability)

| Requirement ID | Sub-scope in M04A | Current matrix status |
|---|---|---|
| `[PLT-DAT]` (Universal Proxy) | Full WIT⇄JSON value conversion (typed dispatch), transport-agnostic proxy routing over JSON-RPC via the existing `AdaptationStage` seam, retry/backoff hook points. **wRPC binary wire deferred — A.5** | No Universal-Proxy row exists yet |
| `[PLT-DAT]` (data-layer extensions) | `AggregationPipeline` (`$group`/`$having`/projections, ADR-0007) and privileged `query-raw` (ADR-0011) | M3A `[PLT-DAT]` rows cover CRUD/query only |
| `[FND-IAM]` (foundation) | UCAN context init at ingress, verified caller-identity threading through native dispatch, Admin-UCAN capability. **Not** the FDAE pushdown sieve (M04B) | No matrix row exists |
| `[FND-SEC]` (per-app KEK) | Per-SynApp-Instance KEK narrowing (`system-architecture.md:1808`) | M2/M3A `[FND-SEC]` Complete; this is the D-03-01 follow-on |
| `[PLT-DAP-05]` | Data-pipeline streams — **spike-first / M5 candidate** only (A.4/A.6) | No matrix row exists |
| `[LFC-VER]` (protocol negotiation) | **Deferred out of M4 — A.7.** Minimal kept: typed *unsupported-protocol* error + reserved version tag | Matrix's only `[LFC-VER]` row is M1 (manifest semver) |

---

## Relationship to M04B (split boundary & co-design seams)

**M04A implementation runs in parallel with M04B *design* (ADR D-04-02).**
M04B's *implementation* follows M04A — hard deps: B0's `NativeInvocation`
identity field and A1 (Universal Proxy) for M04B's federated fetch (B3).

Two interfaces are **built here but consumed in M04B**; design them jointly with
an M04B sketch in view, or they will need rework:

1. **SessionContext / capability representation** (Slice B1 + ADR D-04-01) →
   M04B's SQL compiler binds normalized scopes/claims as `?` parameters
   (`system-architecture.md:978`). Shape B1's output against M04B's needs.
2. **Universal Proxy request shape** (Slice A1) → M04B's stage-2 federated
   relationship-proof fetch rides it. Ensure A1 can carry a proof-fetch, not
   only an ordinary call.

Design **D-04-01 (here) and D-04-02 (M04B) as a pair** — SessionContext is the
shared contract.

---

## Decision Register

### A. Documentation drift / scope resolved (no ADR needed)

1. **`[PRD-SAF]` is out of scope for all of M4.** `traceability-matrix.md:17`
   targets it at M4, but its acceptance evidence (report/dispute/deletion) is
   product/UX-level consent flow, not substrate plumbing. FDAE provides the
   *mechanism* consent would run on; the authoring/dispute UX belongs to a
   consumer surface (Chat/Hub, M6+). Retarget `[PRD-SAF]` → `TBD` at M04B
   closeout.
2. **`[FND-FDA]` = `[FND-IAM]`** (two IDs, one requirement). `[FND-IAM]` is
   canonical (matches the spec body and the matrix). Correct the spec Appendix
   and the M6 citation as a doc pass.
3. **"`http-native`" is not a literal interface.** No `"http-native"` arm exists
   in `SynSvcNativeService::dispatch` — the real arms are `data-layer`, `vault`,
   `app-config`, `blob-store`, `messaging` only
   (`crates/control_plane/src/synsvc_native.rs:588`). The HTTP passthrough bridge
   (`crates/router/src/route_handler/http.rs`) is a *call path* onto those five,
   not a sixth interface. Slice B0 closes the gap for that call path, not a
   nonexistent registry entry.
4. **`[PLT-DAP-05]` ships transport-only / spike-first.** Its only consumer
   (DataFusion `TableProvider` / Substrait) is M5. Slice A3 is a framing spike
   relying on QUIC-native flow control, or moves wholesale to M5 — no
   DataFusion coupling here.
5. **wRPC deferred; JSON-RPC is the uniform inter-service transport for M4.**
   The Universal Proxy's value is transparent routing + retry/backoff + identity
   threading, none of which depends on the wire encoding; the QUIC transport
   wRPC would ride on is already built (M3C,
   [ADR-0014](../../../decisions/0014-quic-stream-protocol-routing.md)); and in
   this codebase host-held guest values are wasmtime `Val`, so *both* wires
   require a `Val ↔ wire` conversion (`conversions.rs`) — wRPC is **not** "zero
   serialization," just a more efficient/faithful wire. The real trade is
   "wRPC-wire design" for "full WIT⇄JSON conversion" (Slice A0′). Drops former
   ADR D-04-03 and former Slice A0; the `wrpc://` scheme
   (`preamble.rs:140,153`) and `AdaptationStage` seam
   (`dispatch.rs:122-123`) stay reserved. JSON fidelity gaps (`u64 > 2^53`,
   `char` vs `string`, nested `option<option<T>>`) are documented as known
   limitations, not hacked around (Slice A0′).
6. **No bespoke credit-based backpressure; rely on QUIC-native flow control.**
   Iroh QUIC already provides per-stream/connection flow control. Application
   credits add value only for logical-unit or processing-completion
   backpressure, which M4 has no consumer to justify. Former ADR D-04-04 is
   withdrawn; Slice A3 uses QUIC's window.
7. **Protocol negotiation deferred (fail-fast instead).** With one protocol,
   negotiation is machinery for a one-member set. The preamble already carries
   the scheme; the callee returns a typed *unsupported-protocol/version* error.
   A future wRPC is added via negotiation-by-trial (try → typed error → fall
   back) without breaking older nodes. Former Slice A2 deferred. Kept: the typed
   error + a reserved `json-rpc/v1` version tag.

### B. Blocking ADRs — ✅ resolved

Both written 2026-07-13; resolve as designed before the dependent slice starts.

- **D-04-01 — UCAN Capability & Verification Model** →
  [ADR-0015](../../../decisions/0015-ucan-capability-model.md). Decides: the
  UCAN semantic model over existing ed25519/`did:key` primitives in a new
  `syneroym-ucan` crate (no `rs-ucan`/JWT for M4); `Capability { with, can,
  caveats }` with the `data-layer/admin` **Admin capability**; a
  `CapabilityToken` chain verified into a normalized `SessionContext`.
  **Co-designed with M04B's D-04-02** (co-design seam #1). **Blocks:** B0, B1.
- **D-04-05 — Native-Dispatch Identity Threading** →
  [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md).
  Decides: add a `caller: CallerContext` **field** to `NativeInvocation`
  (`crates/rpc/src/native.rs:15-18`); make `verify_preamble` mandatory;
  `creator_id` becomes the caller (not `self.service_id`); the Admin-capability
  gate replaces `is_init_context`; cross-node hops carry signed proofs
  re-verified at the data-owning node. **Blocks:** B0 — the highest-priority
  slice.

---

## Items Carried Forward from M3 Planning (all land in M04A)

All five M3→M4 gate items are capability-plumbing, so they close here (M04B
carries **no** M3 debt — it is purely the new FDAE engine):

1. **Native-dispatch / HTTP-bridge authentication gap.** `RouteHandler::handle_stream`
   (`crates/router/src/route_handler/io.rs:61`) only calls
   `HandshakeVerifier::verify_preamble` inside `if preamble.delegation.is_some()`
   (line 90-92) — when `delegation` is `None`, **no identity check runs**.
   Separately, the verified identity is **not threaded downstream**:
   `NativeInvocation` (`crates/rpc/src/native.rs:15-18`) carries no identity, so
   `SynSvcNativeService::dispatch` (`synsvc_native.rs:588`) — fanning out to
   `data-layer`/`vault`/`app-config`/`blob-store`/`messaging` — never receives a
   caller. Confirmed by code read: today `do_put(..., &self.service_id)` sets
   `creator_id` to the **service being called**, not the caller — there is no
   distinct caller identity yet. **Two bugs:** the conditional skip, and no
   downstream plumbing. Slice B0 fixes both. (M2 Slice 2 status flagged this as a
   deliberate "handshake authorization opt-in for now" deviation to close "when
   RBAC is introduced" — that is now.)
2. **`AggregationPipeline`** (ADR-0007): Slice B4.
3. **Privileged `query-raw`** (ADR-0011): Slice B5.
4. **`is_init_context` → Admin UCAN capability.** Two `TODO(M4)` sites:
   `crates/sandbox_wasm/src/host_capabilities.rs:452-463` (guest-side
   `execute-ddl` gate; `is_init_context` field/compute in
   `crates/sandbox_wasm/src/engine.rs:547,594,630`) and
   `crates/control_plane/src/synsvc_native.rs:309-316` (native-side, TODO at
   313). Both replaced by Slice B0.
5. **Per-SynApp-Instance KEK narrowing** (D-03-01 follow-on;
   `system-architecture.md:1808`): `KeyStore::inject_kek(&self, kek_bytes,
   _scope)` (`crates/data_keystore/src/key_store.rs:46`) accepts a scope that is
   underscore-prefixed (unused); the KEK stays substrate-global. Slice B6.

---

## Explicit Non-Goals

- **The FDAE relational policy engine** — pushdown sieve, RLS/CLS, ReBAC-to-SQL
  compilation, the 4-stage pipeline, stage-4 ABAC. That is **M04B**. M04A stops
  at capability admission (Tier 0–2), not data-plane row/column filtering.
- **wRPC binary wire** and the `syneroym-wrpc` crate (A.5) — scheme/seam stay
  reserved.
- **Handshake protocol negotiation** (A.7) — fail-fast errors instead.
- **Application-level credit-based backpressure** (A.6) — QUIC-native only.
- **Full-fidelity JSON numeric/char/nested-option encoding** — documented as a
  known limitation (Slice A0′).
- Full DataFusion/Substrait orchestration (M5); Slice A3 is spike/transport only.
- Outbox/DLQ/saga (M5) — retry/backoff *hook points* only; failed-after-retries
  fails directly, does not queue.
- `[PRD-SAF]` consent/dispute UX (A.1); attestation/supply-chain (M7); distributed
  cron (M5).
- Per-service KEK scoping — M04A narrows substrate-global → per-app-instance only.

---

## Dependency Gates

M04A may begin implementation **only when**:

1. **M3B/M3C fully closed.** ✅ (`M03B-messaging/status.md`, 2026-07-12).
2. **ADRs D-04-01 and D-04-05 resolved** — ✅
   [ADR-0015](../../../decisions/0015-ucan-capability-model.md),
   [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
   (2026-07-13). **Slice A0′ needs neither** — it is startable immediately.
3. `cargo test --workspace` clean, zero clippy warnings on the branch M04A
   starts from.

**Slice-level order (recommended):**

- **A0′ first / immediately** — no ADR, no dependency.
- **B0 next** — the security-gap closure + `NativeInvocation` identity type
  (needs D-04-05). B0 defines the type shape A1 and B1/B5/B6 build on.
- **A1** after A0′ and B0's type change (A1 shares the `NativeInvocation`/dispatch
  edit surface with B0 — do B0's type change first to avoid re-threading).
- **B1** after B0 (needs D-04-01).
- **B4, B5, B6** after B0 (B5/B6 need B0's Admin-UCAN/identity types; B4 is
  independent data-layer work).
- **A3** spike whenever convenient, or deferred to M5.

---

## Current State Inventory (anchors re-verified on `main` @ `6c6e859`)

| Crate / File | What Exists |
|---|---|
| `crates/sandbox_wasm/src/conversions.rs:9,38,48` | `json_to_wasm_params` handles only `String`/`U32`/`Bool` (else "Unsupported parameter type… Add conversion logic"); `wasm_results_to_json_string` is string-or-debug. Slice A0′'s target |
| `crates/rpc/src/native.rs:15-18` | `NativeInvocation { interface, method, params }` — no identity field; Slice B0's primary edit target |
| `crates/control_plane/src/synsvc_native.rs:588` | `SynSvcNativeService::dispatch` fans out to `data-layer`/`vault`/`app-config`/`blob-store`/`messaging` |
| `crates/router/src/route_handler/io.rs:61,90-92` | `handle_stream`; delegation check gated on `preamble.delegation.is_some()` (the conditional-skip bug) |
| `crates/router/src/handshake.rs` | `HandshakeVerifier::verify_preamble` — validates delegation cert vs. master DID + DHT revocation; no capability scoping |
| `crates/sandbox_wasm/src/host_capabilities.rs:452-463` | guest-side `execute_ddl`, gated by `is_init_context`; `TODO(M4)` at 453 |
| `crates/sandbox_wasm/src/engine.rs:547,594,630` | `is_init_context` field/compute (`method_name == "init" \|\| "migrate"`) |
| `crates/control_plane/src/synsvc_native.rs:309-316` | native-side `execute-ddl`, unconditionally denied; `TODO(M4)` at 313 |
| `crates/data_keystore/src/key_store.rs:46` | `inject_kek`'s unused `_scope: Option<&str>` — per-app KEK point, never wired (all call sites pass `None`) |
| `crates/router/src/route_handler/http.rs` | M3C HTTP passthrough bridge — the call path Slice B0 must also cover (A.3) |
| `crates/router/src/preamble.rs:140,153`, `crates/router/src/route_handler/dispatch.rs:122-123` | `RouteProtocol::Wrpc` + `AdaptationStage::JsonRpcToWrpc` stub ("not implemented yet") — reserved seam, left in place (A.5) |

---

## Migration Strategy

### `NativeInvocation` Type Change (Slice B0)
Gains an identity/capability field (shape per D-04-05). Every `NativeService`
impl (`SynSvcNativeService`, `ControlPlaneService`'s `security` interface)
updates together — internal type, not WIT surface, so no compat shim; the
workspace recompiles as one.

### `SubstrateConfig` Extension
```toml
[iam]                                    # M4A
admin_ucan_root = "did:key:..."          # root DID authorized to issue Admin UCANs
```
No `[proxy]` negotiation config (A.7); a reserved `json-rpc/v1` tag lives in the
wire, not config.

### WIT Boundary Versioning
`syneroym:iam@0.1.0` (or equivalent from D-04-01) added. `syneroym:proxy@0.1.0`
added (Slice A1, `crates/wit_interfaces/wit/proxy/proxy.wit`) — the guest-facing
Universal Proxy `call` import, wired into `host-environment`'s imports.
`syneroym:data-layer` gains additive `query-raw` (ADR-0011) + aggregation
(ADR-0007) — minor bump, not breaking. No new wRPC package (A.5). If Slice A3
is not moved to M5, `syneroym:data-stream@0.1.0` is added. `wasm32-wasip2` must
stay unbroken after every slice.

### Per-app KEK DEK Re-wrap (Slice B6)
No stored-data schema change; the `dek_store` schema is unchanged, only the
`scope` passed to `inject_kek` changes from always-`None` to an instance ID.
Because DEKs are wrapped by the KEK, changing the effective KEK requires
**re-wrapping** existing DEKs — B6 must specify and test that re-wrap path
(rotate under old global KEK → per-instance KEK), not assume it is free.

---

## Ordered Implementation Slices

#### Slice A0′: Full WIT⇄JSON Value Conversion — *startable immediately, no ADR*
**Requirement:** `[PLT-DAT]`. *(A short design note pins the lossy-edge JSON
encoding conventions — A.5.)*
Replace the `conversions.rs` stub (`crates/sandbox_wasm/src/conversions.rs:9-48`)
with a full component-model ↔ JSON converter covering the entire WIT type
system: records↔objects, variants/enums↔tagged, lists↔arrays, tuples,
`option`↔null, `result`, flags, `char`, all integer/float widths. This makes
calls *typed* over a JSON wire — dispatch validates against the real WIT
signature. Document known JSON fidelity gaps (`u64 > 2^53`, `char` vs `string`,
nested `option<option<T>>`) as limitations, not worked around now. Also switch
positional → named params (the `conversions.rs` TODO) while here.

#### Slice B0: Native-Dispatch Authentication Gap Closure — *highest priority*
**Blocked on:** ADR D-04-05. **Requirement:** `[FND-IAM]` foundation; closes gate
items #1 and #4.
Make `verify_preamble` mandatory (not conditional on `delegation.is_some()`) for
every native-capability interface and the HTTP bridge call path (`http.rs`). Add
the caller-identity field to `NativeInvocation` and thread it through every
`NativeService::dispatch` site — replacing the `creator_id = self.service_id`
stopgap with the real caller identity. Replace both `is_init_context` `TODO(M4)`
sites (`host_capabilities.rs:452-463`, `synsvc_native.rs:309-316`) with the Admin
UCAN check. Define how identity threads across a cross-node proxy hop (A1 seam).

#### Slice A1: Universal Proxy Dispatch (JSON-RPC transport)
**Depends on:** A0′, B0's `NativeInvocation` shape. **Requirement:** `[PLT-DAT]`.
Component↔component and native↔native typed calls over JSON-RPC / Iroh QUIC,
same-node and cross-node (`system-architecture.md:1930-1937`). Build the proxy
interface **transport-agnostic** behind the `AdaptationStage` seam (replacing the
`dispatch.rs:122-123` stub) so a future wRPC wire (A.5) slots in additively.
Establish retry/backoff hook points; failed-after-retries fails directly (DLQ is
M5). Callee returns a typed *unsupported-protocol* error for an unknown scheme
(the minimal `[LFC-VER]` behavior kept from deferred A2). **Co-design the request
shape with M04B's B3 federated fetch** (co-design seam #2).

#### Slice A2: Protocol Negotiation — DEFERRED (A.7)
Not implemented. The fail-fast error + reserved `json-rpc/v1` tag are handled in
A1. Full negotiation revisited with wRPC later.

#### Slice A3: `[PLT-DAP-05]` Data Pipeline Streams — DEFERRED TO M5 ✅ (2026-07-18)
**Requirement:** `[PLT-DAP-05]`. *(No bespoke-credit ADR — A.6.)*
`syneroym:data-stream` WIT interface; point-to-point QUIC streams, Arrow
`RecordBatch`-shaped framing, **relying on QUIC-native flow control**. Standalone
(A.4) — no DataFusion coupling. Run as a framing spike; if it cannot be validated
without its M5 consumer, **move wholesale to M5**. **Decided: moved wholesale to
M5** — validating the framing choice has no real signal without M5's actual
consumer; see `status.md`'s "Slice A3 — DEFERRED TO M5" section.

#### Slice B1: UCAN Context Extraction and Normalization
**Blocked on:** ADR D-04-01. **Depends on:** B0. **Requirement:** `[FND-IAM]`.
Gateway verifies the UCAN chain at ingress (`system-requirements-spec.md:977`),
normalizes external auth (OIDC/DIDs/WebAuthn) into internal DIDs, extracts
claims/capabilities/scopes into the **SessionContext**. **Shape the SessionContext
against M04B's SQL-binding needs** (co-design seam #1) — it is consumed by M04B's
pushdown compiler as bound `?` parameters.

#### Slice B4: `AggregationPipeline` ✅ (2026-07-16)
**Requirement:** `[PLT-DAT]`; closes gate item #2. *(Independent — no auth
dependency; may start any time.)*
`$group`/`$having`/projections, translating to SQLite `GROUP BY`/`HAVING`
per ADR-0007. *("On `query`" above was realized as a separate `aggregate`
function returning `raw-query-result` (D1, ADR-0007 amendment) — `query`'s
fixed record shape cannot represent a grouped/projected result. Targets
physical collections only; aggregating over init-defined logical views is
deferred (D3) — see `status.md`'s B4 section.)*

#### Slice B5: Privileged `query-raw` Escape Hatch
**Depends on:** B0 (Admin UCAN capability type). **Requirement:** `[PLT-DAT]`;
closes gate item #3.
Implement `query-raw`/`sql-value` per
[ADR-0011](../../../decisions/0011-privileged-raw-sql-query.md), gated by the
Admin UCAN capability from B0 instead of `is_init_context`.

#### Slice B6: Per-SynApp-Instance KEK Narrowing
**Depends on:** B0. **Requirement:** `[FND-SEC]`; closes gate item #5.
Wire `inject_kek`'s `_scope` param (`key_store.rs:46`) to derive per-app-instance
KEKs, gated on the caller's verified app-instance identity. Specify + test the DEK
re-wrap path (Migration Strategy).

#### Slice B7: Substrate & Service Ownership (Deploy Authorization + Ownership Attribution) — split into **B7a ✅ (2026-07-18)** / **B7b ✅ (2026-07-18)** ([plans/B7.md](plans/B7.md))
**Depends on:** B0 (done — substrate-owner resolution now sources from
`ControllerAgreement`, see status.md addendum). **Interacts with:** B1 (a
real capability-delegation chain is the likely mechanism for item 1 below).
**Requirement:** `[FND-IAM]`.

Surfaced via design discussion (2026-07-14), prompted by a concrete gap: today
`crates/app_orchestration/src/catalog.rs` records no owner/creator for a
deployed app at all, and `orchestrator`'s `list` method
(`crates/control_plane/src/service.rs:250`) returns every deployed app to any
caller — there is no "list only my apps" or "substrate owner sees everything"
enforcement, and no data to enforce it against even if there were.

Agreed shape (not yet designed in code):
1. **Service-owner permission is a grant, not a mutual agreement** — unlike
   substrate ownership (`ControllerAgreement`, two-way signed), the substrate
   owner unilaterally hands specific DIDs permission to deploy/undeploy/
   status-check on this substrate (a pre-negotiated, ongoing, **revocable**
   grant — not a one-time setup step; substrates may eventually be offered in
   a marketplace to arbitrary grantees). Likely realized as a UCAN capability
   once B1's real chain-verification exists; B0's `admin_ucan_root`-style
   allowlist is not expressive enough (no revocation, no per-grantee scoping).
2. ~~**App catalog needs an owner field.** `catalog.rs` must record which
   DID's deploy call created each app.~~ **Corrected per plan F1 (B7a ✅):**
   `app_orchestration/src/catalog.rs` is a client-side (`roymctl`)
   blueprint→manifest resolver with no deployed-app records and no
   `control_plane` caller. The substrate's actual deployed-service record is
   `EndpointRegistry` (`crates/core/src/local_registry.rs`, persisted via
   `endpoints.db`'s new `service_owners` table) — that is what `list` reads
   and where the owner field now lives.
3. **Attribution must resolve through one hop of delegation to the real
   owner**, not the immediate signing key — covers both key rotation and a
   distinct team-member's own key equally (same mechanism, `master_did`
   resolution already used by `build_caller`, see `io.rs:63`). **Multi-hop
   delegation (a delegate re-delegating further) is not resolvable with
   today's one-hop `DelegationCertificate` format and is explicitly deferred**
   until real UCAN proof chains (B1) exist — flag, don't silently misattribute.
4. **The substrate publishes registry/DHT entries on behalf of the owner**,
   not the owner itself — avoids a gap between "owner believes it's deployed"
   and "it's actually discoverable." Must attribute to the resolved owner
   (item 3), which today's `publish_to_community_registry`
   (`crates/substrate/src/runtime.rs`) does not yet do.
5. **`list` gates on caller identity**: substrate owner (resolved
   `ControllerAgreement` controller) sees every app; a service owner sees only
   apps whose recorded owner (item 2) matches their resolved identity (item
   3). Multiple recognized substrate owners (multiple independent
   `ControllerAgreement`s, or a rotated owner key) are all equally privileged
   — no partial/limited owner tier.

> **Planned 2026-07-17 — see [plans/B7.md](plans/B7.md), which supersedes the
> anchors below where they conflict.** The plan recommends splitting B7 into
> **B7a** (attribution: items 2–5) and **B7b** (the deploy grant: item 1), and
> flags three things this section gets wrong or predates:
> - **Item 2 names the wrong file.** `app_orchestration/src/catalog.rs` is a
>   client-side (`roymctl`) blueprint→manifest *resolver* with no deployed-app
>   records; `control_plane` never calls it. The substrate's deployed-service
>   record is `EndpointRegistry` (`crates/core/src/local_registry.rs`,
>   persisted via `endpoints.db`) — that is what `list` reads and where the
>   owner field belongs (plan F1).
> - **Item 3 is already satisfied** by B0's `build_caller`
>   (`caller_did = DelegationCertificate.master_did`); only the multi-hop
>   *flagging* is outstanding (plan F11, §2.6).
> - **Item 4 is not implementable as written, and its premise is unsound.** The
>   community registry verifies every record against its own `service_id`'s key
>   (`SignedEndpointInfo::verify`), so the substrate *cannot* sign a `Service`
>   record for an app — by design, since that is what stops a hostile substrate
>   publishing for services it doesn't host. The owner attribution item 4 wants
>   already exists and is unused (`EndpointInfo.delegation`). Item 4's
>   justification — closing a gap between "owner believes it's deployed" and
>   "it's actually discoverable" — also does not hold as stated: a client *can*
>   reach an unpublished service if given the substrate address out of band
>   (`SyneroymClient::new_with_mechanisms` bypasses registry lookup), and the
>   `Service` record only maps service → substrate anyway. Item 4 is **dropped
>   from B7** (plan F9). What the situation actually needs is the opposite of
>   item 4 — **declared service visibility** (below), not more substrate-side
>   publishing.
> - **New scope from `f95206b`:** ADR-0017's Open list assigns the
>   mis-addressed Tier-1 `TODO(M04B/FDAE)` (`route_handler/dispatch.rs`) to B7
>   — "today any verified identity reaches any native service" (plan F3). B7b
>   closes it for `orchestrator` only; the `security` interface and the five
>   data native-capability interfaces stay open (plan F3.1).
>
> **Decided 2026-07-17:** ship as **B7a** (attribution) then **B7b** (the deploy
> grant). An **unowned substrate** — no verified `ControllerAgreement` and no
> `[iam].admin_ucan_root`, i.e. every deployment today — issues every verified
> caller `orchestrator/{deploy,undeploy,status}` on the node, logged loudly at
> boot; deliberately *not* `substrate/admin`, which entails `data-layer/admin`
> and would open `execute-ddl`/`query-raw` to everyone.

**Open questions — all resolved 2026-07-17** (plan §6 records them in full):
- *Exact shape of the deploy/undeploy/status-check grant?* →
  `orchestrator/{deploy,undeploy,status}` as UCAN capabilities, **flat** (no
  entailment tier), scoped by an `app/<name>` selector. B1 has shipped, so no
  interim pre-UCAN mechanism is needed.
- *Posture of a substrate with no owner?* → it **issues every verified caller
  the three orchestrator abilities**, logged loudly at boot — never
  `substrate/admin`, which entails `data-layer/admin` and would open
  `execute-ddl`/`query-raw` to everyone. Consequence: **B7b's gate is inert
  until something can create a `ControllerAgreement`** (the natural next slice);
  B7 must not be reported as "deploy is authorized".
- *Does "list apps filtered by owner" belong here or in FDAE (M04B)?* → **here.**
  The catalog is `EndpointRegistry`/`endpoints.db` — substrate plumbing, not a
  service's data-layer DB — so FDAE has no policy document to attach to it and
  no service resource to name; ADR-0017 §2.1's default-*absent* rule agrees
  (a resource with no `definitions:` entry is grant-only). The two milestones do
  not duplicate the mechanism.

**B7a delivered (2026-07-18)** — see `status.md`'s B7a section for full
evidence: owner recorded per deployed service (`EndpointRegistry`/
`endpoints.db`'s `service_owners` table), survives restart, cleared on
undeploy; `list` filtered per item 5 (node-wide orchestrator authority sees
everything, an ordinary caller sees only their own, an unattributed
pre-B7a app is hidden); redeploy/undeploy from a non-owner rejected (F7);
the unowned-substrate posture (F4) expressed as an issued capability, logged
at boot; `roymctl --as` operator identity (F5); both Tier-1 TODOs retargeted
off `M04B/FDAE` (F3/§2.8).

**B7b delivered (2026-07-18)** — see `status.md`'s B7b section for full
evidence: ADR-0015 A1 selectors + segment-wise prefix cover
(`ResourceUri::covers_resource`), `is_substrate_scope` narrowed to the bare
form (F2), `A3`'s `can_delegate` caveat enforced at attenuation, `A6`'s
resource-scoped `is_trusted_root` (owner-rooted trust per service) wired
into `build_caller`, `A7`'s revocation confirmed to already cover it (F11),
`F6`'s cross-node wildcard closed at the chain-rooting predicate. The Tier-1
`orchestrator/{deploy,undeploy,status}` gate now runs on every
`deploy`/`undeploy`/per-service `readyz` call (§3.2/§2.4.1), independent of
ownership — item 1 is closed. `roymctl identity issue-grant` + the global
`--ucan` flag let an operator mint and present a real grant. **Item 1 is the
only thing B7b closes; nothing else in B7's scope changes.** As before B7b,
the gate is inert in practice on today's every-substrate-is-unowned reality
(F4) — every verified caller still holds the bare orchestrator abilities and
passes the gate trivially — but it is now real code, not "not started", and
is exercised end to end by real signed `CapabilityToken`s in the test suite
(not just hand-built `CallerContext`s). `execute-ddl`/`query-raw` remain
denied (F4's over-grant trap, still tested, unaffected by B7b).

**Spun out of B7** (plan §6.2, which has the detail):
- **Declared service visibility** — designed in
  [ADR-0018](../../../decisions/0018-service-record-visibility.md) (*Proposed*,
  awaiting agreement). Publication is currently *incidental* (a service is
  published iff a pre-signed certificate happened to be supplied). ADR-0018 adds
  a three-valued `visibility` to the manifest + WIT `service-config` (default
  `private`), makes `SignedEndpointInfo` the export/import artifact for private
  records, adds a verified local known-records store so a private service stays
  reachable cross-node, and keeps `EndpointInfo.is_private` as the `internal`
  tier's wire encoding.
- `roymctl svc deploy` validating `--identity` against `--svc-id` (today a
  mismatch silently builds a certificate the registry rejects forever).
- A registry-trust-model ADR (item 4 as literally written — needs a real
  consumer first), a `ControllerAgreement` creation tool, multiple substrate
  owners, and Tier 1 for the five data native-capability interfaces.

---

## Reference Scenario (M04A subset)

Continues the "Professional Services Guild" walking skeleton from M03B (step 19):

20. ✅ Two services on different physical nodes exchange a typed call through the
    Universal Proxy (A1) — JSON-RPC transport with full WIT⇄JSON conversion (A0′)
    — routed transparently to the remote instance. Proven by
    `crates/coordinator_iroh/tests/multi_hop_relay.rs::test_cross_node_proxy_call`.
21. ✅ A client presents a UCAN; the gateway verifies the chain and normalizes
    claims/capabilities into a SessionContext (B1). Proven by
    `crates/router/tests/ucan_context.rs::verified_ucan_capability_reaches_native_dispatch`.
24. ✅ An admin-scoped caller runs `query-raw` for a report needing a join beyond
    the JSON filter DSL (B5). Proven by
    `crates/router/tests/native_dispatch_identity.rs::admin_caller_admitted_query_raw`
    and `::query_raw_binds_params_no_injection`.
25. ✅ A peer with no valid delegation attempts a `data-layer` write over a raw Iroh
    connection; now rejected at the router (B0) — the interim gap is closed.
    Proven by
    `crates/router/tests/native_dispatch_identity.rs::anonymous_caller_rejected_before_native_dispatch_for_every_interface`.

*(Steps 22–23 — FDAE row filtering and federated fetch — belong to
[M04B](../M04B-fdae-policy/task.md).)*

---

## Failure and Security Tests

| Test | Expected Outcome |
|---|---|
| Peer opens Iroh connection with no `preamble.delegation`, attempts `data-layer::put` | Rejected at `handle_stream` before native dispatch |
| Same via the HTTP bridge (`http.rs`) with no verified identity | Rejected on the same call path (A.3) |
| Peer presents a delegation cert whose `temporary_did` does not match the preamble pubkey | Rejected by `verify_preamble` (existing check, unchanged) |
| `query-raw` without Admin UCAN capability | `data-layer-error::permission-denied`, same shape as `execute-ddl` today |
| `query-raw` with SQL injection via `params` | Bound as a parameterized value; no injection |
| Caller declares a protocol scheme the callee does not support | Typed *unsupported-protocol/version* error (A.7) |
| WIT⇄JSON round-trip of a `u64 > 2^53` / `char` / nested `option` value | Documented lossy-edge behavior (A0′) — no silent corruption |
| Per-app-instance KEK requested by a caller without that app-instance's identity | `permission-denied`; another instance's KEK never returned |

---

## Performance Budgets

| Metric | Budget | Method |
|---|---|---|
| UCAN chain verification (cache-cold) | < 5 ms p99 | `criterion` micro-bench |
| Universal Proxy call (JSON-RPC, same-node) | < 5 ms p99 round-trip incl. WIT⇄JSON both ways | `criterion` micro-bench |
| WIT⇄JSON conversion (typical record, round-trip) | Document measured; must not dominate same-node call latency | `criterion` micro-bench |
| Service DB open with per-app KEK | Establish budget on Raspberry Pi 4 (per `M03-sss/task.md` deferred item) | Integration test |
| `[PLT-DAP-05]` stream throughput (local, 1 MB batches, QUIC-native flow control) | Document measured *(skip if A3 moves to M5)* | Integration test |

---

## Tests Summary

- **Unit:** WIT⇄JSON conversion round-trip across the full WIT type set incl.
  documented lossy edges (A0′); typed result serialization (`wasm_results_to_json`,
  A1); `ProxyRouter` local dispatch, guest native-capability gate, retry, and
  proof forwarding (A1); UCAN verification + claim/capability normalization
  (B1); `AggregationPipeline` stage translation (B4).
- **Integration:** **Native-dispatch identity threading end-to-end** —
  unauthenticated caller rejected; authenticated caller's identity reaches
  `dispatch_data_layer` (B0) — *the single most important test in this
  milestone*; guest-to-guest same-node proxy call + cross-service
  native-capability denial through the proxy, and the typed
  unsupported-protocol error (A1); `query-raw` permission-denied +
  injection-resistance (B5); per-app-instance KEK isolation + DEK re-wrap
  (B6).
- **Benchmarks (`criterion`):** UCAN verification, Universal Proxy same-node call
  (A1 — `proxy_local_native`/`proxy_local_wasm`), WIT⇄JSON conversion.
- **E2E (`mise run test:e2e`):** reference scenario steps 20, 21, 24, 25 in a live
  substrate, ≥2 substrates for the cross-node proxy case (A1 —
  `test_cross_node_proxy_call`, in-process via `coordinator_iroh`'s own test
  harness rather than Playwright; see plan.md Flag F10).

---

## Measurable Exit Criteria

- [x] `cargo +nightly fmt --all` clean; `cargo clippy --workspace --all-targets --all-features` zero warnings; `cargo test --workspace` green; `mise run test:e2e` green (no M0–M3C regression); `wasm32-wasip2` unbroken after every slice. *(True as of A0′+B0+A1+B1+B5+B4; re-verify after each subsequent slice.)*
- [x] ADRs D-04-01, D-04-05 exist in `docs/decisions/`. *([0015-ucan-capability-model.md](../../../decisions/0015-ucan-capability-model.md) and [0016-native-dispatch-identity-threading.md](../../../decisions/0016-native-dispatch-identity-threading.md), both Status: Accepted.)*
- [x] Full WIT⇄JSON conversion replaces the `conversions.rs` stub; round-trip tested across the full WIT type set; JSON fidelity limitations documented. *(A0′ delivered the encode/decode primitives; A1 closes the deferred half — typing the JSON-RPC `result` field itself via `wasm_results_to_json`/`execute_wasm_json` — see `status.md`'s A1 section.)*
- [x] **Gate item #1 verified with a real test** (not code inspection): an unauthenticated peer's `data-layer`/`messaging`/`blob-store`/`vault`/`app-config` call and HTTP-bridge request are all rejected. *(B0 — see `crates/router/tests/native_dispatch_identity.rs`.)*
- [x] `AggregationPipeline` implemented and tested. *(B4 — `crates/data_db/src/aggregate.rs`'s `compile` (whitelisted `$match`/`$group`/`$having`/`$project`/`$sort`/`$limit`/`$skip` document compiler, all field paths/values bound as `?`); `do_aggregate` in `crates/data_db/src/sqlite.rs` reuses B5's `run_query_raw`; guest impl in `crates/sandbox_wasm/src/host_capabilities.rs` (no capability gate, same trust level as `query`); native arm in `crates/control_plane/src/synsvc_native.rs`'s `dispatch_data_layer`; ADR-0007 amended in place — see `status.md`'s B4 section.)*
- [x] `query-raw` implemented, gated by Admin UCAN capability (not `is_init_context`). *(B5 — `crates/data_db/src/sqlite.rs`'s `do_query_raw` (read-only enforced two-layer: `Statement::readonly()` plus an authorizer denying `ATTACH`/`DETACH`/`BEGIN`/pragma-set, post-commit review S1; compute additionally bounded by a `progress_handler`, S1); guest gate in `crates/sandbox_wasm/src/host_capabilities.rs`; native gate in `crates/control_plane/src/synsvc_native.rs`'s `dispatch_data_layer` (request/response `sql-value` JSON encoding made symmetric, post-commit review C1); ADR-0011 amended in place — see `status.md`'s B5 section.)*
- [x] Both `TODO(M4)` sites (`host_capabilities.rs:452-463`, `synsvc_native.rs:309-316`) removed. *(B0 — both replaced by the `data-layer/admin` capability gate.)*
- [ ] Per-app-instance KEK narrowing implemented; `_scope` actually used; DEK re-wrap path tested.
- [x] Universal Proxy handles ≥1 real cross-node typed call over JSON-RPC (full WIT⇄JSON conversion) in an e2e test; the transport-agnostic seam for later wRPC is in place. *(A1 — `crates/router/src/proxy.rs`'s `ProxyRouter`/`RemoteHop`/`IrohHop`; cross-node proof in `crates/coordinator_iroh/tests/multi_hop_relay.rs::test_cross_node_proxy_call`.)*
- [x] A caller declaring an unsupported protocol receives a typed error (negotiation deferred, A.7). *(A1 — `ServiceStage::UnsupportedProtocol`, `-32091`; see `crates/router/tests/unsupported_protocol.rs`.)*
- [x] `[PLT-DAP-05]` either ships as a QUIC-flow-control-backed framing spike or is explicitly deferred to M5 with rationale in `status.md`. *(Deferred wholesale to M5, per A3's own stated fallback — no code exists, and validating the framing choice has no real signal without M5's actual consumer. Rationale recorded in `status.md`'s "Slice A3 — DEFERRED TO M5" section.)*
- [x] Reference scenario steps 20, 21, 24, 25 execute end-to-end. *(All four now marked ✅ in the Reference Scenario section above, each with its own dedicated integration-test proof, matching the convention step 20 already established rather than requiring one continuous chained run: A1 closes step 20 — `test_cross_node_proxy_call`. B1 closes step 21 — `ucan_context.rs::verified_ucan_capability_reaches_native_dispatch` plus `io.rs`'s `build_caller` unit tests (chain verify + revocation wiring). B5 closes step 24 — `native_dispatch_identity.rs`'s `admin_caller_admitted_query_raw`/`query_raw_binds_params_no_injection`; the live-substrate e2e assertion remains a milestone-close activity per B5.md §9, not re-run here. B0 closes step 25 — `native_dispatch_identity.rs::anonymous_caller_rejected_before_native_dispatch_for_every_interface`.)*
- [ ] Performance budgets verified; `criterion` output in `status.md`. *(A1 delivers the "Universal Proxy call (JSON-RPC, same-node)" row — see `status.md`'s A1 section; B1 delivers the "UCAN chain verification (cache-cold)" row — see `status.md`'s B1 section. A3's row no longer applies — deferred wholesale to M5, no transport to benchmark. The remaining row belongs to B6.)*
- [x] `traceability-matrix.md` updated with M04A evidence for `[PLT-DAT]` (Universal Proxy + conversion + aggregation + `query-raw`), `[FND-IAM]` (foundation: identity threading + UCAN context + Admin capability), `[FND-SEC]` (per-app KEK); `[PLT-DAP-05]` marked spike/M5; `[LFC-VER]` protocol-negotiation retargeted out; `[FND-FDA]`→`[FND-IAM]` citation fixed (A.2). *(`[PLT-DAT]`/`[FND-IAM]` (M4A) rows flipped to Complete with evidence; `[FND-SEC]` (per-app KEK) correctly left Planned — B6 not started; `[PLT-DAP-05]` evidence points at the new deferral rationale. `[FND-FDA]` citation fixed at its two sources, `system-requirements-spec.md`'s Appendix and `meta-implementation-plan.md` — it was never present in `traceability-matrix.md` itself.)*
- [x] `system-architecture.md:1892` interim-security-posture note updated to record the native-dispatch gap as closed. *(B0 — see the "Gap closed (M04A Slice B0)" note at that anchor.)*

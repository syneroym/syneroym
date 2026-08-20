# M06B Slice B4 — Durable Messaging: Interface and 1:1 Delivery — Implementation Plan

> **Revision 2 (2026-08-20)** — incorporates a plan review. Ten defects were
> raised; **all ten were verified against the tree and all ten are correct**.
> One (§14.8, the dead-letter alert) is accepted with a narrowed remedy and a
> stated reason. What changed materially: the identity model (§1 F16, `D-B4-5`,
> `D-B4-22`), which was broken and is now rebuilt around signed envelopes;
> the `syneroym-async-queue` API work (§1 F18, §5.6), which was unbudgeted;
> and the wiring mechanism for `ConversationHost` (§1 F19, `D-B4-24`), which
> the tree already answers with a precedent the first revision missed.
>
> **Revision 2a** — a follow-up review asked for one undefined helper to be
> pinned down. `svc_address(svc)` is now defined in §5.1, together with the
> **one-string invariant** it rests on and a test that asserts it; closing it
> surfaced a live hole (a registry alias and a DID name one service but derive
> two conversation ids), now `D-B4-29`. Also: `D-B4-28` was referenced three
> times in revision 2 and never defined — it is now a real decision row.
>
> **Scope.** Gaps **G1 part 1** and **G2** of the experience spec, as slice
> **B4** of [M06B](task.md): the `syneroym:conversation` host interface
> (conversations, direct delivery, delivery state, history) with outbox
> `pending`/`delivered`/`failed` folded in (`D-06B-2`), plus the Layer 3
> machinery underneath it — X3DH + Double Ratchet direct exchange, the
> sender's own outbox, strict direct delivery with no third-party buffering
> (`D4`, [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §3).
> Durable content never touches the pub/sub broker (ADR-0013 §6).
>
> **Not in B4.** The Gossip DAG, group conversations, the owner-distributed
> per-epoch group key, epidemic routing, and offline catch-up — all **B5**.
> Multi-device / Primary-substrate reconciliation, third-party mailboxes,
> MLS, and attachments are milestone-level non-goals ([task.md](task.md)
> §"Explicit non-goals").
>
> **Status:** plan only. Nothing implemented. Written against the tree at
> commit `3e57f69` **plus the uncommitted B3 follow-on work in the working
> tree** — see §0.1.
>
> **Decision-id note.** `D-B4-n` in this document means *M06B slice B4*.
> M05B's own slice-B4 plan uses the same prefix for unrelated decisions;
> the two namespaces are scoped by document, exactly as `D-B1-n`/`D-B2-n`
> already are across M05B and M06B.

---

## §0 What B1, B2, and B3 hand to B4

### 0.1 The working tree is not the same as `HEAD`

`git status` shows thirteen modified files that are B3 follow-on work **not
yet committed**, including two this plan depends on directly:

| File | Change B4 depends on |
|---|---|
| `crates/wit_interfaces/wit/messaging/messaging.wit` | adds `world messaging-import` (import-only view, no `guest-api`/`stream-types` export requirement) |
| `crates/wit_interfaces/src/messaging.rs` | retargets the guest bindgen at `messaging-import` |

**Step 0 of §11 is therefore: commit or otherwise settle the working tree
before starting B4.** B4 copies the `*-import` world pattern verbatim for
`syneroym:conversation`; starting from `HEAD` would reproduce the same
component-encoding failure B3 already diagnosed and fixed.

There is also an untracked stray, `crates/control_plane/test_fdae_redeploy_policy_45803.json`,
which looks like pid-suffixed test debris. Confirm and delete or ignore it.

### 0.2 B3's five constraints (its §13, "What B3 owes B4")

Hard inputs, not suggestions:

1. **The host implementation lands as `impl … for HostState`** in
   [`crates/sandbox_wasm/src/host_capabilities.rs`](../../../../crates/sandbox_wasm/src/host_capabilities.rs),
   beside `store::Host` and `blob_store::Host`. That is the only place
   `syneroym-app-host-native` can reach it from (`D-B3-5`).
2. **Every function needs three edits**: a trait method in
   `syneroym-app-host`, a guest impl in `syneroym-app-host::guest`, and a
   shim delegation in `syneroym-app-host-native`.
3. **The host→app direction follows `MessageSink`'s shape**, not
   `handle-message`'s. B3 names this "the half that has no automatic parity,
   so it is the half to design first."
4. **Types must be expressible in the guest vocabulary** (`D-B3-2`).
5. **Resources are per-invocation on both builds** (`D-B3-6`). This settles
   [task.md](task.md)'s first open design point — see `D-B4-2`.

### 0.3 B1 and B2

- **B1** gives the gateway a *person's* DID under an owner→node delegation.
  **B4 uses this less than revision 1 assumed** — see F16: the DID a message
  is attributed to is not the DID of whoever invoked the guest. B4 adds no
  gateway code.
- **B2** gives `ServiceConfig.visibility` and `topology_visibility`. B4's
  delivery path needs the recipient's Conversation service to be resolvable
  by a caller its operator never met, which is B2's `visibility =
  internal|public` plus a published `EndpointInfo`. **`D-B2-15`'s migration
  rule applies to every manifest B4's tests deploy** — see §10.2.

---

## §1 Findings from reading the tree

Every line reference is to the working tree at 2026-08-20.

### F1 — the outbox already exists as a library, a worker, and a per-service database, and none of it is about messages

[`crates/async_queue`](../../../../crates/async_queue/src/lib.rs) is a
complete SQLite-backed queue: `Queue::open_encrypted`
([lib.rs:241](../../../../crates/async_queue/src/lib.rs#L241)), `enqueue`,
`claim_due`, `complete`, `fail` → `FailOutcome::{Retrying, DeadLettered}`,
`dead_letters`, `replay`, `all`, `has_pending`, `pending_count`.

`ProxyOutbox` ([`crates/router/src/proxy_outbox.rs`](../../../../crates/router/src/proxy_outbox.rs))
is the precedent for owning one per service, and
`ProxyRouter::drain_outboxes_once` / `drain_one_outbox`
([proxy.rs:1361](../../../../crates/router/src/proxy.rs#L1361)) is the
precedent for the worker loop, including three subtleties B4 must copy:

- a claim that never resolves is bounded by `item.claim_count`, not by
  `attempts` (a crashed worker never calls `fail`);
- an unreadable payload is terminal, not retried;
- a target that no longer resolves is terminal on its own terms, and does
  **not** go through the retry classification.

The worker is spawned once in `runtime.rs`
([runtime.rs:289](../../../../crates/substrate/src/runtime.rs#L289)) off
`connection_router.proxy()`, ticking at `roles.app_sandbox.queue_tick_secs`.

**One thing the ADR asks for that this precedent does not do.** ADR-0023 §5
requires a terminal failure to *"raise an alert through the existing
`AlertStore` path"*. `drain_one_outbox` does not: on `FailOutcome::DeadLettered`
it increments `substrate.proxy.outbox.dead_lettered` and logs
([proxy.rs:1453](../../../../crates/router/src/proxy.rs#L1453)), and `rg`
for `AlertStore` under `crates/router/` returns nothing. `AlertStore` lives
in `syneroym-app-orchestration` and is written only by the **supervisor**
(`app_supervisor/src/{service,store}.rs`), keyed by `AppInstanceId`. So the
alerting half of §5 is unimplemented for the existing outbox as well. See
§14.8 and `D-B4-25`.

### F2 — `proxy.enqueue` is not the outbox G2 is asking for

`syneroym:proxy/proxy.enqueue` gives a guest durable fire-and-forget of an
*RPC call*, deliberately opaque about outcome, with state readable only by
an operator through `ProxyQueueInspector`
([rpc/src/proxy.rs:268](../../../../crates/rpc/src/proxy.rs#L268)). A guest
cannot read `pending`/`delivered`/`failed` for anything, and the items are
calls, not messages. B4 builds a second, parallel owner of an
`async_queue::Queue`, keyed by message id.

### F3 — there is a fourth-per-service-store trigger already written down, and B4 fires it

[deferred-backlog.md](../../deferred-backlog.md) row 240's pickup trigger is
literally *"a fourth per-service store is added and the duplication becomes
four copies instead of three"*, pointing at
[`crates/router/src/service_async_db.rs`](../../../../crates/router/src/service_async_db.rs).
`async_db_location(...)` is `pub(crate)` to `syneroym-router`; B4 needs it
from outside. See `D-B4-12` and §13.

### F4 — the recipient of a message has no address today

`MasterAnchorPayload`
([dht_registry.rs:763](../../../../crates/core/src/dht_registry.rs#L763))
carries `{schema, revoked_keys, revoke_list_registry, timestamp}` — key
revocation, nothing else. **Nothing in the tree maps a person's Master DID
to the substrates that may receive on their behalf.** ADR-0013 §2's Primary
Substrate designation is designed and unbuilt; multi-device is an explicit
M06B non-goal. `EndpointInfo` maps a **service** id to `mechanisms`, and
[task.md](task.md) states: *"**No wire-format change** to endpoint records,
topology documents, or gateway hostnames."*

### F5 — X3DH's asynchrony assumes a server, and this architecture has none

X3DH exists so Alice can start a session with an *offline* Bob, by fetching
a prekey bundle a **server** holds. ADR-0013 §3 forbids third-party
buffering. But the constraint is weaker than it looks: delivery is already
gated on the same reachability, so the bundle can be served by **Bob's own
substrate**, on the same connection the delivery would use, and nothing is
lost — the failure mode is identical to the one D4 already accepts.

### F6 — the spec names a crate whose licence does not fit this repository

[roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md)
§Messaging says *"1:1: X3DH plus Double Ratchet, through
`libsignal-protocol-rust`."* The workspace is
`MPL-2.0 OR LicenseRef-Commercial` ([Cargo.toml:36](../../../../Cargo.toml#L36));
libsignal is AGPL-3.0. **Blocking input-document problem** — §14.1, `D-B4-7`.

Available today: `ed25519-dalek` 2.2, `p256` 0.13 (`ecdh`), `aes-gcm` 0.10,
`hkdf` 0.12, `sha2` 0.10, `blake3` 1.8, `rand` 0.9, `zeroize` 1.8. No
`x25519-dalek`, no ratchet of any kind. The house precedent
([`handshake.rs`](../../../../crates/router/src/handshake.rs)) is a
*handshake*, not a ratchet: no chain keys, no per-message key derivation,
no skipped-message handling. Not a starting point.

### F7 — the linker adds every host interface unconditionally, so a new package costs one line

`AppSandboxEngine::build_wasm_linker`
([engine.rs:741](../../../../crates/sandbox_wasm/src/engine.rs#L741))
already links `syneroym_wit_interfaces::http::syneroym::http::websocket`,
which lives in its own world (`http-host`) and is **not** in
`host-environment`. A component that does not import an interface is
unaffected by its presence in the linker.

### F8 — the host calls a guest export by name

`AppSandboxEngine::deliver_message`
([engine.rs:1547](../../../../crates/sandbox_wasm/src/engine.rs#L1547))
looks up `"syneroym:messaging/guest-api@0.1.0"` as a *string* through
`get_wasm_func`, instantiates a fresh store with
`CallerContext::service_system(service_id)` and `self.dispatch_epoch_ticks`,
and retries instantiation up to 4 times at 50 ms. It logs and gives up
rather than propagating. Note `service_system`, **never** `local_elevated`.

### F9 — the native-capability dispatch table is five arms over a six-name array, and the mismatch is deliberate

`SynSvcNativeService::dispatch`
([synsvc_native.rs:1403](../../../../crates/control_plane/src/synsvc_native.rs#L1403))
has **five** arms: `data-layer`, `vault`, `app-config`, `blob-store`,
`messaging`. `NATIVE_CAPABILITY_INTERFACES`
([local_registry.rs:40](../../../../crates/core/src/local_registry.rs#L40))
has **six** names — the sixth, `http-native`, is the M3C HTTP bridge's
reserved name and deliberately has no dispatch arm; that constant's own doc
comment explains why. *(Revision 1 called these "matching"; they are not, and
an implementer counting arms against the array would be misled.)*

`NODE_NATIVE_INTERFACES = ["orchestrator", "security"]`
([local_registry.rs:47](../../../../crates/core/src/local_registry.rs#L47))
is registered under the node's own DID and denied to guests outright.

### F10 — `invoke_remote_at` builds the right identity, but only under conditions that must be met deliberately

[proxy.rs:809](../../../../crates/router/src/proxy.rs#L809)'s
instance-certificate branch is
`(None, CallOrigin::Native { service_id: Some(sid) })` — it requires **all
three** of:

1. `req.caller.proof` is **`None`**;
2. `registry.instance_cert(sid)` exists and is unexpired;
3. `registry.owner_of(sid)` is recorded.

If `proof` is `Some`, the **first** arm matches instead and forwards the
original caller's proof verbatim — so a delivery worker that reuses a
`CallerContext` carrying a person's proof would present *that person* to the
peer, not the service. If (2) or (3) fail, it falls back to the node's own
key. Both `D-B4-4`'s security argument and the e2e setup depend on hitting
branch 2 exactly. See `D-B4-23`.

### F11 — the broker supports wildcard subscription, so failure-matrix row 6 is testable

`MqttBroker::subscribe(topic_filter)`
([mqtt_broker/src/lib.rs:181](../../../../crates/mqtt_broker/src/lib.rs#L181))
passes the filter straight to `rumqttd`, so `#`/`+` work.  `MqttBroker::new`
opens no listener, so a second instance in one process is free.

### F12 — every guest entry point is bounded by 5 seconds

`dispatch_epoch_timeout_secs` defaults to 5
([config.rs:504](../../../../crates/core/src/config.rs#L504)), its own doc
calling it *"tight by design"*. So `conversation::send` must never wait on
the network. See `D-B4-9`.

### F13 — the parity harness is the template and already knows the traps

[`dual_build_parity.rs`](../../../../crates/app_host_native/tests/dual_build_parity.rs)
stands up two independent host stacks sharing **one service id**
(`D-B3-17`), drives both under one real `CallerContext`, compares verbatim,
and proves the comparison can fail. B4 extends this file. But conversation
delivery is cross-node, so the parity suite proves only the **local** half;
the cross-node half needs `multi_substrate_placement_e2e.rs`'s `Node`
harness, **not** `tests/common`'s `SubstrateTestContext` (whose module doc
says two live nodes deadlock on its setup lock).

### F14 — an `open` service's resolve bypass has an unbounded-cost note addressed to B4 by name

[deferred-backlog.md](../../deferred-backlog.md) §3: *"B4/B5 should not
inherit this silently if they reuse the same unauthenticated-`resolve`
shape."* B4 does not reuse it, but it creates a comparable surface —
`prekey-bundle` — and must bound it (`D-B4-15`).

### F15 — the fixture is a workspace member with file symlinks five levels up

[`test-components/dual-build-fixture/`](../../../../test-components/dual-build-fixture/).
`wit/deps/<pkg>/` is a **real directory** containing a **file** symlink:

```
wit/deps/data-layer/data-layer.wit
    -> ../../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit
```

Five `..` levels, to the `.wit` file — *not* a directory symlink four levels
up, as revision 1 wrote. Its `Cargo.toml` narrows `syneroym-wit-interfaces`
to `default-features = false, features = ["data-layer","blob-store","messaging"]`.

### F16 — three different DID namespaces are in play, and none of them is the one a message should be attributed to *(revision 2, blocking)*

This is the finding revision 1 missed, and it invalidated three separate
pieces of that plan's §5.1.

| Value | What it actually is |
|---|---|
| `ProxyRequest.target_service` / registry lookup | the peer's **routing service id** |
| `CallerContext.caller_did` at the receiver | the **owner's Master DID**, read out of the delegation certificate |
| `CallerContext.session.subject_did` | the same Master DID (`build_caller` sets both from `id.master_did`) |
| the presented key | `derive_service_identity(owner_did, service_id)` — an HKDF child key whose `did:key` has **no relation** to the service id string |

Verified, not inferred:

- `verify_preamble` returns `VerifiedIdentity { master_did, temporary_did }`
  and sets `master_did = cert.master_did` — the certificate's **issuer**
  ([handshake.rs:44](../../../../crates/router/src/handshake.rs#L44)).
- `build_caller` sets `session.subject_did = id.master_did.clone()`
  ([io.rs:169](../../../../crates/router/src/route_handler/io.rs#L169)) and
  `caller_did: id.master_did.clone()`
  ([io.rs:252](../../../../crates/router/src/route_handler/io.rs#L252)).
  The test `build_caller_uses_master_did_not_temporary_did_as_caller_did`
  ([io.rs:1338](../../../../crates/router/src/route_handler/io.rs#L1338))
  pins it.
- `derive_service_identity` is HKDF over
  `syneroym:identity:v1:{len}:{owner_did}:{service_id}`
  ([keys.rs:240](../../../../crates/identity/src/keys.rs#L240)), and is
  **node-private**: only the hosting node can derive it.
- **`CallerContext` has no `subject_did` field at all**
  ([native.rs:22](../../../../crates/rpc/src/native.rs#L22)) — it is
  `caller_did`; `subject_did` lives on `SessionContext`. Revision 1 wrote
  `caller.subject_did` in five places.

**Three consequences, all fatal to revision 1's §5.1:**

1. `derive_conversation_id(svc_did, caller.subject_did)` at the receiver
   mixes a *service id* with a *Master DID*, while the sender derives over
   a service-id pair. The two sides mint **different conversation ids**.
2. The attribution check `payload.sender_did != caller.subject_did`
   compares a guest/person DID against a Master DID. It can **never pass**.
   Revision 1 called this check load-bearing, which it is.
3. `(sender_did, id)` dedup and `(sender_timestamp, sender_did, id)`
   ordering therefore hold **different values on the two sides** — which B5
   would inherit for its byte-identical-transcript requirement.

Worse than the review states, in one respect: because `caller_did` is the
*owner's* Master DID, two different services owned by the same person are
**indistinguishable** at the receiver. The transport can never answer "which
service is calling".

**What is recoverable.** `CallerProof.pubkey_hex` is carried verbatim into
the `CallerContext` ([io.rs:257](../../../../crates/router/src/route_handler/io.rs#L257)),
so the receiver *can* compute the sender's service-instance `did:key`. That
is a stable per-`(owner, service)` value — but it is still not the routing
service id, and the sender cannot verify a peer's copy of it offline. See
`D-B4-5` and `D-B4-22` for the model that replaces revision 1's.

### F17 — `check_native_capability_gate` has a same-service exemption, so "impossible by construction" holds only cross-service *(revision 2)*

[proxy.rs:650](../../../../crates/router/src/proxy.rs#L650):

```rust
if service_id == &req.target_service {
    return Ok(());
}
```

with the test `guest_reaching_its_own_native_capability_is_allowed`
([proxy.rs:3273](../../../../crates/router/src/proxy.rs#L3273)). So a guest
**can** reach `conversation/deliver` and `conversation/prekey-bundle` on its
**own** service id through `syneroym:proxy`.

`NativeInvocation` carries `{interface, method, params, caller}` and **no
origin** ([native.rs:182](../../../../crates/rpc/src/native.rs#L182)), so
`dispatch_conversation` cannot tell a self-proxy call from a peer's. See
`D-B4-26` for why the signed envelope closes this and what is left over.

### F18 — `async_queue` cannot do three things B4 needs, and none is a one-liner *(revision 2)*

1. **The atomic `send` is impossible with today's API.** `Queue` holds
   `Arc<Mutex<Connection>>` and `enqueue(&self, …)` locks it internally
   ([lib.rs:312](../../../../crates/async_queue/src/lib.rs#L312)). A caller
   holding a `rusqlite::Transaction` on that same connection already holds
   the lock; `std::sync::Mutex` is not reentrant, so calling `enqueue`
   inside the transaction **self-deadlocks**. Revision 1 said "confirm
   whether `enqueue` can run inside a caller-held transaction" — the answer
   is no, and `enqueue_in_tx(&Transaction)` does not fit the
   `Arc<Mutex<Connection>>` shape without restructuring.
2. **`D-B4-20` has no API to build on.** `enqueue` hardcodes
   `visible_at = now` ([lib.rs:322](../../../../crates/async_queue/src/lib.rs#L322))
   and there is no `defer`. The good news: `claim_due` already selects on
   `WHERE visible_at <= ?1` and there is an index for it
   ([lib.rs:285](../../../../crates/async_queue/src/lib.rs#L285),
   [lib.rs:459](../../../../crates/async_queue/src/lib.rs#L459)), so a
   `defer` is a small, well-fitting addition rather than a redesign.
3. **`enqueue` takes a `group_key` revision 1 never specified.** It scopes
   the dead-letter cap (`dlq_max_rows` prunes *within* one `group_key`), so
   it directly decides whether `D-B4-16` actually holds.

§5.6 specifies all three as in-scope `syneroym-async-queue` work, and
`crates/async_queue/src/lib.rs` is now in §9's table.

### F19 — the engine already has the exact wiring mechanism B4 needs, and using it avoids 98 call-site edits *(revision 2)*

Revision 1's §8.2 "gestures at a `set_*` method" without deciding, while §9
listed neither constructor. The real counts:

- **`HostState::new` — 26 call sites in 7 files**: `host_capabilities.rs`
  (12, its own tests), `engine.rs` (2), `benches/wasm_engine.rs` (3),
  `tests/lifecycle_hooks.rs` (5), `tests/abac_integration.rs` (2),
  `tests/blob_store_integration.rs` (1), `app_host_native/src/factory.rs` (1).
  It already takes 15 parameters under `#[allow(clippy::too_many_arguments)]`.
- **`AppSandboxEngine::init` — 72 call sites across 23 files**, 8 positional
  arguments, mostly in `control_plane/src/service.rs` and
  `service/orchestration.rs`.

**The tree already answers this.** The engine holds
`pub service_proxy: OnceLock<Weak<dyn ServiceProxy>>`
([engine.rs:200](../../../../crates/sandbox_wasm/src/engine.rs#L200)),
initialised empty at construction
([engine.rs:595](../../../../crates/sandbox_wasm/src/engine.rs#L595)), set
after the fact, and read with a fallback at instantiation time
([engine.rs:1256](../../../../crates/sandbox_wasm/src/engine.rs#L1256)):

```rust
let service_proxy = self.service_proxy.get().cloned()
    .unwrap_or_else(crate::host_capabilities::empty_service_proxy);
```

Copying this costs **zero** changes to `init`'s 72 sites. See `D-B4-24`.

### F20 — a multi-test two-node e2e binary needs a per-binary serialization lock *(revision 2)*

[deferred-backlog.md](../../deferred-backlog.md) row 39 root-causes the
`"no viable network path exists: last path abandoned by peer"` CI failures:
files booting 2–3 full substrates *per test* with no serialization ran up to
`num_cpus` concurrently, starving a 4-vCPU runner badly enough that iroh's
QUIC path-validation timers expire. The fix adopted by `saga_e2e.rs`,
`proxy_outbox_e2e.rs`, `durable_outbox_e2e.rs`, `binding_push_e2e.rs`,
`cert_renewal_e2e.rs`, `health_monitoring_e2e.rs`, and
`multi_substrate_placement_e2e.rs` is a **per-binary static lock acquired
once per test before either node boots** — not per-node, which would
deadlock a two-node test.

§10.2 proposes eleven two-node tests in one binary. Without this lock it
would reintroduce exactly the failure that row documents.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-B4-1** | **B4 ships as two commits behind one plan: B4a (interface, storage, outbox, delivery, state, history) and B4b (the real X3DH + Double Ratchet).** B4a lands with the key agreement behind a trait and one clearly-named non-production implementation; B4b replaces that module. **B4 is not complete until B4b lands.** | ADR-0013 §6 names the seam: *"the key agreement sits behind an interface the DAG, ordering, sync, and storage do not depend on."* F6 says the crypto dependency is unresolved at plan time; blocking every other part of B4 on a licence question is the wrong order, and hiding the split is worse. |
| **D-B4-2** | **Plain functions over an opaque `conversation-id: string`. No WIT resources.** | `D-B3-6`: resources are per-invocation on both builds, so a conversation handle could not survive a call on either. Settles [task.md](task.md)'s first open design point. |
| **D-B4-3** | **One WIT package with three worlds**: `conversation-guest`, `conversation-import` (import-only), `conversation-host` (wasmtime `bindgen!`). **Not** added to `host-environment`. | F7 and the M06A `syneroym:http` precedent. `conversation-import` is required by B3's component-encoding finding. |
| **D-B4-4** | **Inbound delivery arrives on a new native-capability interface, `conversation`**, added to `NATIVE_CAPABILITY_INTERFACES` and given a sixth `SynSvcNativeService::dispatch` arm. Its verbs are transport-only and never appear in the guest-facing WIT. | F9. Cross-service, a guest cannot reach it at all. **Same-service, it can** (F17) — which `D-B4-26` addresses rather than papers over. |
| **D-B4-5** | **Three names, never conflated** *(rewritten in revision 2; F16)*: <br>• **`address`** — a routing service id. What `open-direct` takes, what the outbox stores, what `ProxyRequest.target_service` gets. <br>• **`author`** — the DID a message is attributed to, ordered by, and deduped on. **Defined to be the sender's own `address`**, carried *inside the signed envelope*, never read from the transport. <br>• **transport caller** — `CallerContext.caller_did`, which is the **owner's Master DID** (F16). Used only as a coarse "is this a verified peer at all" gate. | One namespace (`address`) then serves addressing, ordering, and dedup, and both sides compute the same conversation id. Revision 1 used all three interchangeably and broke on every one of F16's three consequences. |
| **D-B4-6** | **The prekey bundle is served by the recipient's own substrate**, over the same authenticated `conversation` interface as delivery, never published in a registry record. Full X3DH with one-time prekeys. | F5. Delivery is already gated on the recipient's substrate being reachable, so this costs nothing D4 has not accepted, and keeps [task.md](task.md)'s no-wire-format-change promise. |
| **D-B4-7** | **`libsignal-protocol-rust` is not adopted.** The key agreement sits behind `SessionCrypto`; the crate is chosen in B4b after a licence and maintenance review, from (a) an Apache-2.0/MIT Olm/Double-Ratchet crate (candidate `vodozemac` — **licence and API to be verified against the crate source**, not from memory or a docs summary), or (b) an in-house X3DH + Double Ratchet over `x25519-dalek`/`hkdf`/`aes-gcm`. | F6: AGPL is incompatible with `MPL-2.0 OR LicenseRef-Commercial`. (b) is the fallback, not the preference. |
| **D-B4-8** | **A service's X25519 identity key is a fresh key, not a transform of its ed25519 identity.** Generated per Conversation service, stored in that service's own encrypted store, bound to the service by an ed25519 signature over the prekey bundle. | Cross-protocol key reuse makes the signing key and the DH key share a scalar for no benefit. A signed bundle gives the same authentication with none of the coupling, matching how Signal separates identity key from signed prekey. |
| **D-B4-9** | **`send` is queue-always, not try-then-queue.** It writes to the store and outbox and returns `pending`, with no inline network attempt. | F12: 5-second epoch. ADR-0023 §2's try-then-queue was reasoned about `write_bindings`, where *"an implementation that enqueues and returns immediately has no outcome to return"*. `send`'s outcome **is** `pending` — R1's *"never shown as delivered while pending"* — so the reasoning does not transfer. Recorded on the ADR (§13). |
| **D-B4-10** | **`delivered` means the recipient's substrate durably committed the message and said so.** The receiving handler commits before returning `Ok`; the sender flips `pending → delivered` only on that `Ok`. | Failure-matrix row 3 becomes true by construction, not by timing. |
| **D-B4-11** | **Dedup is `(author, message-id)`** — both taken from the *verified* envelope, never from the transport (`D-B4-5`) — plus an RPC-layer `idempotency_key` equal to the message id. | ADR-0023 §1: at-least-once needs a fence the caller already holds. Two fences at two layers is not redundancy: the store-level one survives a dedup-window expiry, the RPC-level one does not. |
| **D-B4-12** | **`service_async_db::async_db_location` moves into `syneroym-data-db` as `pub async fn service_db_location`**; the three router callers are updated. | F3. This is the cheap half of backlog row 240; §13 re-scopes the row rather than closing it. |
| **D-B4-13** | **Conversation state lives in its own per-service `conversation.db`**, holding the content tables **and** its own `async_queue::Queue`, on **one connection** (`Queue::from_connection`). Not more tables on `async.db`. | `AlertStore`'s precedent — *"deliberately its own store rather than more tables on `DeploymentJournal`"*. One connection is what makes `D-B4-27`'s atomic `send` possible at all (F18). |
| **D-B4-14** | **Host→guest is two optional exports on `syneroym:conversation/guest-api` — `on-message`, `on-delivery-state` — invoked by name as `deliver_message` invokes `handle-message`, under `CallerContext::service_system`.** The native equivalent is a `ConversationSink` trait mirroring `MessageSink`. | B3 §13 item 3. `service_system` not `local_elevated`, for the reason F8 gives. Optional, so a component that exports neither still receives durably. |
| **D-B4-15** | **`prekey-bundle` refuses an unauthenticated caller and is rate-limited per calling identity**, with a bounded one-time-prekey pool and a documented fallback to the signed prekey when empty (X3DH §3.3's variant). | F14. Otherwise a peer drains the pool at the cost of one keygen per request, and every request is a store write. |
| **D-B4-16** | **Per-conversation bounds in the store**: max outbox items, max stored messages, max body bytes. Exceeding returns `quota-exceeded` and degrades that conversation only. The queue's `group_key` is the **conversation id** (`D-B4-27`), so `dlq_max_rows` prunes per conversation too. | Failure-matrix row 12. `DEFAULT_MAX_PENDING_ROWS` bounds the queue per *service*, not per conversation, so it does not answer the row alone. |
| **D-B4-17** | **Ordering is `(sender_timestamp, author, message_id)`.** | ADR-0013 §5's rule is not total — one sender can produce two messages in one millisecond. A third component costs nothing and makes B5's byte-identical transcripts achievable. Recorded as an implementation note on §5 (§13). |
| **D-B4-18** | **The outbox holds the plaintext body under the service's own DEK; the ratchet ciphertext is produced per delivery attempt.** | The ratchet advances per message key. A ciphertext produced once and retried for hours can outlive the session it was bound to. Encrypting at delivery keeps session state and ciphertext consistent, and the outbox is already DEK-encrypted. |
| **D-B4-19** | **No new `RolesConfig` role.** Knobs go on `AppSandboxRole` beside the existing `queue_*` fields; the worker is spawned in `runtime.rs` beside `proxy_outbox_join`. | F1: that is where `ProxyOutbox`'s knobs and worker already live, with the same lifecycle and tick. |
| **D-B4-20** | **A "peer not reachable" failure re-defers without consuming the attempt budget**, bounded by `conversation_max_pending_age_secs` (default 30 days) from `created_at`, after which the message goes `failed` and `retry` can re-arm it. Only a *refusal* (the peer answered and said no) consumes the budget. | Failure-matrix row 5 says a message to an unreachable peer stays `pending`; the queue's budget dead-letters in ~10 hours. The budget exists to stop retrying something broken, and an offline peer is not broken. Needs `Queue::defer` (F18, §5.6). |
| **D-B4-21** *(new)* | **A message whose `sender_timestamp` is more than `conversation_max_clock_skew_secs` (default 24 h) *ahead* of the receiver's clock is rejected at `peer_deliver`.** Past timestamps are accepted unbounded. | ADR-0013's Open Questions leaves the "coarse sanity bound" for a follow-up decision, and `sender_timestamp` is both attacker-chosen and the primary sort key — a far-future value pins a message to the top of every participant's history permanently. Asymmetric on purpose: a future value is an ordering attack, a past one only sorts old. B4 takes the decision and records the amendment (§13). |
| **D-B4-22** *(new)* | **Every message carries an ed25519 signature by the sender's per-service conversation signing key**, over a canonical byte string covering `(message_id, conversation_id, author, sender_timestamp, content_type, body)`. The key is published in the prekey bundle and bound by the same X3DH exchange. Attribution requires **both** layers to agree: the envelope decrypts under the session pinned to `author`, **and** the signature verifies under the key pinned for `author`. | ADR-0013 §5 requires the timestamp be *"signed as part of the message"* — that is what makes ordering trustworthy across relay hops, and revision 1 had no signature anywhere (its §15 item 5 conceded as much by telling B5 to add one). It also replaces the attribution check F16 proved can never pass, and is what B5's relayed entries need. |
| **D-B4-23** *(new)* | **The delivery worker builds its outbound `CallerContext` with `proof: None` and `CallOrigin::Native { service_id: Some(<sender service id>) }`, and refuses to send if `instance_cert`/`owner_of` are absent** — returning a *terminal* error naming the missing certificate. | F10: with `proof: Some(..)` the first arm forwards a person's proof to the peer instead of the service's; with no certificate it silently falls back to the node key. Both are wrong and both are silent. Failing loudly matches `proxy.enqueue`'s own precedent, which refuses a call from a service holding no unexpired instance certificate. |
| **D-B4-24** *(new)* | **Wiring is by `OnceLock`, not by constructor parameter.** `AppSandboxEngine` gains `pub conversation: OnceLock<Weak<dyn ConversationHost>>` beside `service_proxy`; `HostState` gains a `conversation` field defaulted to `Weak::new()` in `new` plus a `#[must_use] with_conversation(self, …) -> Self` builder. `HostState::new`'s signature and `AppSandboxEngine::init`'s do not change. | F19: 26 + 72 = 98 call sites otherwise, and `HostState::new` already carries 15 parameters under a `too_many_arguments` allow. The engine's own `service_proxy` is exactly this pattern, fallback included. `Weak::new()` also makes "this node has no conversation service" representable, which is the correct answer for a node that runs none. |
| **D-B4-25** *(new)* | **A conversation dead letter emits a metric and a `warn!` — the same treatment `drain_one_outbox` gives a proxy dead letter — and does not write to `AlertStore`.** One backlog row covers ADR-0023 §5's alerting half for **both** queues. | F1: `AlertStore` is supervisor-scoped (`AppInstanceId`-keyed, in `supervisor.db`) and the existing proxy outbox raises no alert either. Building a conversation-only alert path would (a) not work on a node with no supervisor role, which is exactly the deployment a conversation service runs on, and (b) leave the older, larger gap open while claiming the ADR is satisfied. See §14.8. |
| **D-B4-26** *(new)* | **Self-injection through the same-service exemption is permitted and tested, and the signed envelope is what bounds it.** A guest calling `conversation/deliver` on its own service id cannot produce a *validly signed* message from a peer, because it holds no peer key — so the worst it achieves is writing a self-signed message into its own store, which it can already do through `data-layer`. `dispatch_conversation` additionally rejects any envelope whose `author` equals this service's own address. | F17, and `NativeInvocation` carries no origin, so an origin check is not available without widening a shared type. Making the *signature* the invariant is stronger than an origin check anyway: it holds for a relayed message in B5, where origin never will. |
| **D-B4-27** *(new)* | **`syneroym-async-queue` gains three things**, in scope for B4 and budgeted in §5.6: `Queue::transaction(...)` (one lock, one `rusqlite::Transaction`, enqueue through the same transaction), `Queue::defer(id, visible_at)`, and a specified `group_key` — the **conversation id** — for every conversation enqueue. | F18. Without the first, `send` cannot be atomic and `enqueue` inside a caller-held transaction self-deadlocks. Without the second, `D-B4-20` has no mechanism. Without the third, `D-B4-16`'s per-conversation bound does not reach the dead-letter cap. |
| **D-B4-28** *(new)* | **A peer's conversation signing key is pinned trust-on-first-use, per address, at session creation.** A later bundle presenting a *different* key for a pinned address is a **hard failure**, never a silent re-pin, and never a prompt. The residual gap is recorded rather than papered over. | Nothing in the tree independently binds a service address to a long-term key: `derive_service_identity` is node-private ([keys.rs:240](../../../../crates/identity/src/keys.rs#L240)), so a sender cannot verify a peer's key offline, and no registry record carries a conversation key — which [task.md](task.md) forbids changing here anyway. TOFU is therefore the honest ceiling, and it is the same trust the whole proxy layer already rests on. Hard-failing on a changed key is what keeps it from silently degrading to no authentication at all. Backlog row 2, with a concrete pickup trigger. |
| **D-B4-29** *(new)* | **`open-direct` canonicalizes `peer-address` to the peer's own `EndpointInfo.service_id` before deriving anything**, and refuses an address that does not resolve. The canonical string is what the conversation row, the outbox item, and every `author` comparison store. | A call target may be *"the target's DID **or a registry alias**"* ([proxy.wit](../../../../crates/wit_interfaces/wit/proxy/proxy.wit)), and `RegistryClient::lookup(id, resolve)` accepts either ([dht_registry.rs:375](../../../../crates/core/src/dht_registry.rs#L375)). Two peers naming one service by different strings would derive **different conversation ids** for the same conversation — F16's first consequence again, by a different route. Refusing an unresolvable address at `open-direct` also moves a guaranteed delivery failure from hours later to the call that caused it. |

---

## §3 The WIT package

**New file:** `crates/wit_interfaces/wit/conversation/conversation.wit`.

Names are normative. Doc comments are abbreviated here and must be written
out in full, in the house style (why, not what).

```wit
package syneroym:conversation@0.1.0;

/// Durable, ordered, end-to-end-encrypted conversation. Distinct from
/// `syneroym:messaging` by design: ADR-0013 section 6 forbids durable
/// message content from depending on the pub/sub broker.
interface conversation {
    /// Host-minted, opaque, never parsed by a guest.
    type conversation-id = string;
    /// Content-derived and stable across delivery attempts: it is the
    /// at-least-once fence both the receiver's store and the RPC dedup
    /// layer use.
    type message-id = string;

    variant conversation-error {
        permission-denied,
        not-found,
        invalid-argument(string),
        /// No session could be established with the peer. A `send` never
        /// returns this -- `send` does not touch the network.
        unreachable(string),
        quota-exceeded,
        internal(string),
    }

    enum delivery-state { pending, delivered, failed }

    /// `group` is reserved for the group slice and is never returned by
    /// this version. Present so a guest's match is not broken by its
    /// arrival.
    enum conversation-kind { direct, group }

    record conversation-summary {
        id: conversation-id,
        kind: conversation-kind,
        /// Routing service ids ("addresses"), sorted. Exactly two for
        /// `direct`. This is the same namespace `open-direct` takes and
        /// `message.author` reports -- see the note on `author`.
        participants: list<string>,
        created-at: s64,
        last-activity-at: s64,
    }

    record message {
        id: message-id,
        conversation: conversation-id,
        /// The sender's own routing service id. It is **not** read from
        /// the transport: the transport reports the owner's master DID,
        /// which cannot distinguish two services of one owner. `author`
        /// travels inside the signed, session-authenticated envelope, and
        /// `verified` below reports whether both checks passed.
        author: string,
        /// The sender's own raw clock in Unix milliseconds, signed by the
        /// sender and taken at face value (ADR-0013 section 5). Primary
        /// sort key. Not corrected, and it may be wrong -- though a value
        /// implausibly far in the future is refused on arrival.
        sender-timestamp: s64,
        /// When this substrate learned of the message. Local, never
        /// sorted on, never sent.
        received-at: s64,
        content-type: string,
        body: list<u8>,
        state: delivery-state,
        /// The host's own verdict: the envelope decrypted under the
        /// session pinned to `author` **and** the signature verified under
        /// the key pinned for `author`. Always true for a message this
        /// service sent itself. A guest may show provenance from this
        /// without holding any key material -- the same honesty the
        /// `caller-auth` label already applies on the HTTP path.
        verified: bool,
        /// Set only on `failed`.
        last-error: option<string>,
    }

    record history-page {
        /// Oldest first within the page, ordered by
        /// (sender-timestamp, author, id).
        messages: list<message>,
        next-cursor: option<string>,
    }

    /// Returns the existing 1:1 conversation with `peer-address`, or
    /// creates one. `peer-address` is the peer's Conversation *service*
    /// id -- the same string a proxy call would target. Idempotent.
    open-direct: func(peer-address: string) -> result<conversation-id, conversation-error>;

    conversations: func() -> result<list<conversation-summary>, conversation-error>;

    /// Writes the message durably and returns immediately with state
    /// `pending`. Never touches the network.
    send: func(
        conversation: conversation-id,
        content-type: string,
        body: list<u8>,
    ) -> result<message-id, conversation-error>;

    history: func(
        conversation: conversation-id,
        limit: u32,
        cursor: option<string>,
    ) -> result<history-page, conversation-error>;

    delivery-status: func(message: message-id) -> result<delivery-state, conversation-error>;

    /// Every message this service still owes delivery for, plus every one
    /// that gave up. The outbox surface, folded in here rather than given
    /// a package of its own.
    outbox: func() -> result<list<message>, conversation-error>;

    /// Re-arms a `failed` message. A `pending` or `delivered` message is
    /// `invalid-argument`.
    retry: func(message: message-id) -> result<_, conversation-error>;
}

/// Optional guest exports. A component that exports neither still
/// receives messages durably -- they are in the store either way.
interface guest-api {
    use conversation.{message-id, message, delivery-state};

    on-message: func(msg: message) -> result<_, string>;
    on-delivery-state: func(msg: message-id, state: delivery-state) -> result<_, string>;
}

world conversation-guest { import conversation; export guest-api; }

/// Import-only view. Without this, every consumer inherits `guest-api`'s
/// export requirement into its own component-type section and encoding
/// fails -- the finding that produced `data-layer-import` and
/// `messaging-import`.
world conversation-import { import conversation; }

world conversation-host { import conversation; export guest-api; }
```

**Seven functions** — `open-direct`, `conversations`, `send`, `history`,
`delivery-status`, `outbox`, `retry`. §5.4 and §6.2 say seven. *(Revision 1's
§5.4 said eight; it was wrong.)*

**Type-vocabulary check (`D-B3-2`):** every type is `string`, `list<u8>`,
`s64`, `u32`, `bool`, `option`, `enum`, `record`, or `variant`. No
resources, no wasmtime-only types.

**The signature is deliberately not in the guest-facing `message` record.**
It is a Layer 3 concern (B5's relays verify it; a guest cannot re-derive the
pinned key), so the guest gets the host's verdict as `verified: bool`
instead. The raw signature lives in `DeliveryPayload` and in the `messages`
table.

---

## §4 Bindings modules

### 4.1 `crates/wit_interfaces/src/conversation.rs` (new, guest)

```rust
//! Guest-side bindings. `conversation-import`, not `conversation-guest`:
//! this module exists to be *called into* by other components' own worlds,
//! and `guest-api`'s export requirement would otherwise become an unmet
//! requirement of every consumer's linked component.

wit_bindgen::generate!({
    world: "conversation-import",
    path: "wit/conversation/conversation.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
```

### 4.2 `crates/wit_interfaces/src/conversation_host.rs` (new, host)

Mirrors `src/http.rs`:

```rust
wasmtime::component::bindgen!({
    path: "wit/conversation",
    world: "conversation-host",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
    exports: { default: async },
});
```

### 4.3 `crates/wit_interfaces/src/lib.rs`

`#[cfg(feature = "conversation")] pub mod conversation;` beside the other
guest modules, and `pub mod conversation_host;` inside the existing
`#[cfg(not(target_arch = "wasm32"))]` block beside `http` — host-only, and
deliberately **not** feature-gated, since gating it would change every
host-only consumer's build.

### 4.4 `crates/wit_interfaces/Cargo.toml`

`conversation = []` added to `[features]` **and** to `default = [...]`.

---

## §5 Host implementation

### 5.1 The Layer 3 crate — `crates/conversation/` → `syneroym-conversation` (new)

Naming per AGENTS.md: directory snake_case, package `syneroym-<kebab>`.

**Why a crate and not more of `sandbox_wasm`:** the store, the ratchet, the
outbox, and the delivery worker need no wasmtime and are driven from three
places (the `HostState` impl, the shim's delegation, the substrate's worker
loop). `syneroym-sandbox-wasm` would drag wasmtime into all three.

Dependencies: `syneroym-data-db`, `syneroym-data-keystore`,
`syneroym-async-queue`, `syneroym-rpc`, `syneroym-identity`,
`syneroym-core`, `rusqlite`, `tokio`, `tracing`, `metrics`, `serde`,
`serde_json`, `blake3`, `ed25519-dalek`, `zeroize`, plus B4b's crypto crate.
**Not** `syneroym-sandbox-wasm`, **not** `wasmtime`.

```
src/lib.rs        // ConversationService, ConversationError
src/store.rs      // conversation.db: schema, queries, the shared Queue
src/outbox.rs     // the delivery worker
src/transport.rs  // the peer-facing verbs + the outbound call
src/crypto.rs     // SessionCrypto, the session store, the signing key
src/envelope.rs   // DeliveryPayload, canonical bytes, sign/verify
src/ids.rs        // conversation-id / message-id derivation
```

#### The identity model in one place (`D-B4-5`, `D-B4-22`)

Read this before any of the pseudo-code below.

| Name | Value | Where it comes from |
|---|---|---|
| `address` | a routing **service id** | the guest (`open-direct`), the store, `ProxyRequest.target_service` |
| `author` | the sender's own `address` | **inside** the signed envelope |
| transport caller | the **owner's Master DID** | `CallerContext.caller_did` — coarse gate only |

Two independent checks make `author` trustworthy, and **both** must pass:

1. the envelope decrypts under the X3DH/ratchet session pinned to `author`;
2. the ed25519 signature verifies under the conversation signing key pinned
   for `author`.

Pinning is trust-on-first-use, established when the session is created
against the prekey bundle returned by a routed call to that `address`. A
later bundle presenting a **different** key for a pinned `address` is a hard
failure, never a silent re-pin. `D-B4-28` and the backlog row in §13 record
the residual gap: nothing in the tree independently binds a service address
to a long-term key, because `derive_service_identity` is node-private and no
registry record carries a conversation key. This is the same trust the whole
proxy layer already rests on; it is now written down rather than assumed.

#### `svc_address(svc)` — the one string, and why all three roles must be it

The pseudo-code below calls `svc_address(svc)` four times. It is not a
lookup and not a derivation. **It is the identity function**, and saying so
is the point:

```
svc_address(svc) == svc == HostState.component_id
                        == the deployed service id
                        == EndpointInfo.service_id in this service's own record
                        == the string a peer passes as ProxyRequest.target_service
```

Verified, since the whole identity model rests on it:

- `HostState.component_id` is the deployed service id — it is what
  `open_store` namespaces the SQLite store by, what
  `namespace_topic`/`namespace_topic_for_publish` namespace broker topics
  by, and what the `data-layer/admin` gate builds its `ResourceUri` from
  ([host_capabilities.rs:649](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L649)).
- `member_registry_record` writes `EndpointInfo.service_id = service_id`
  verbatim ([deploy.rs:698](../../../../crates/sdk/src/deploy.rs#L698)), and
  `certify_placed_members` keys by the member master `ServiceId` — so for an
  app-deployed member this string is itself a `did:key`.
- `resolve_iroh_addr(registry_client, service_id)` looks the peer up by
  exactly that string
  ([net_iroh.rs:133](../../../../crates/router/src/net_iroh.rs#L133)).

**If those ever stop being one string, the identity model quietly reopens** —
`derive_conversation_id(svc_address(svc), author)` would no longer agree
across the two sides, which is `D-B4-5`'s whole load-bearing property. So
this is an invariant to assert, not a coincidence to rely on. §10.3 gets a
test that pins `component_id`, the store namespace, and the published
`EndpointInfo.service_id` to the same value for a deployed service.

**One live way they can differ, and it must be closed.** `proxy.wit` says a
call target is *"the target's DID **or a registry alias**"*, and
`RegistryClient::lookup(id, resolve)`
([dht_registry.rs:375](../../../../crates/core/src/dht_registry.rs#L375))
accepts either. Two peers naming one service by different strings — its DID
and its nickname alias — would derive **different conversation ids** for the
same conversation, reopening F16's first consequence by a different route.
`D-B4-29` closes it: `open-direct` canonicalizes before it derives.

**One caveat about the fixture.** `NativeHostFactory.service_id` is supplied
by the embedder (`D-B3-21`), and `init_dual_build_fixture` supplies the
literal `DUAL_BUILD_FIXTURE_DISPATCH_ID` (`"dual-build-fixture"`), which is
a `native_dispatch` key and **not** a registry-resolvable id. So the
fixture's `svc_address` is not routable. That is fine and does not weaken
anything: the parity suite (§10.1) is entirely in-process and never
delivers over the wire. The cross-node suite (§10.2) must therefore use
**really deployed services with DID service ids**, not the linked fixture —
which it already does, since it needs instance certificates and published
records anyway (`D-B4-23`, §10.2 setup requirement 2).

#### `conversation.db` schema (`store.rs`)

```sql
CREATE TABLE IF NOT EXISTS conversations (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,             -- 'direct' | 'group' (unused in B4)
    peer_address  TEXT,                      -- NOT NULL for 'direct'
    created_at    INTEGER NOT NULL,
    last_activity INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_conversations_direct_peer
    ON conversations(peer_address) WHERE kind = 'direct';

CREATE TABLE IF NOT EXISTS messages (
    id               TEXT PRIMARY KEY,
    conversation_id  TEXT NOT NULL REFERENCES conversations(id),
    author           TEXT NOT NULL,          -- a routing service id, verified
    sender_timestamp INTEGER NOT NULL,
    received_at      INTEGER NOT NULL,
    content_type     TEXT NOT NULL,
    body             BLOB NOT NULL,
    signature        BLOB NOT NULL,          -- ed25519 over canonical_bytes()
    outgoing         INTEGER NOT NULL,       -- 1 = we sent it
    verified         INTEGER NOT NULL,
    state            TEXT NOT NULL,          -- 'pending' | 'delivered' | 'failed'
    last_error       TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_order
    ON messages(conversation_id, sender_timestamp, author, id);
-- D-B4-11's store-level fence, over verified values only.
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup ON messages(author, id);

CREATE TABLE IF NOT EXISTS sessions (
    peer_address  TEXT PRIMARY KEY,
    pinned_sig_key BLOB NOT NULL,            -- TOFU; a change is a hard failure
    state         BLOB NOT NULL,             -- opaque; SessionCrypto owns the shape
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS local_identity (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    dh_secret  BLOB NOT NULL,                -- X25519, D-B4-8
    sig_secret BLOB NOT NULL,                -- ed25519, D-B4-22
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS prekeys (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,               -- 'signed' | 'one-time'
    secret      BLOB NOT NULL,
    public      BLOB NOT NULL,
    consumed_at INTEGER,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS prekey_requests (   -- D-B4-15's rate limit
    caller_did  TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    count       INTEGER NOT NULL,
    PRIMARY KEY (caller_did, window_start)
);
```

Plus `async_queue`'s own `outbox` and `dead_letters` in the **same file on
the same connection** (`D-B4-13`, `Queue::from_connection`). Every `BLOB` is
inside a DEK-opened database.

#### Derivations (`ids.rs`, `envelope.rs`)

```
conversation_id = "conv:" || hex(blake3("direct" || 0x00 || min(a,b) || 0x00 || max(a,b)))
      where a, b are the two *addresses*  -- both sides compute this identically,
      the receiver using the `author` from the verified envelope

message_id      = "msg:" || hex(blake3(
                      author || 0x00 || conversation_id || 0x00 ||
                      sender_timestamp_be || 0x00 || content_type || 0x00 ||
                      body || nonce_16))

canonical_bytes(m) = b"syneroym:conversation:v1" || 0x00 ||
                     m.message_id || 0x00 || m.conversation_id || 0x00 ||
                     m.author || 0x00 || m.sender_timestamp_be || 0x00 ||
                     m.content_type || 0x00 || len_be(m.body) || m.body
```

`len_be(body)` and not a bare separator, so no field can be reassigned
across a boundary — the same reasoning `derive_service_identity`'s own
comment gives for length-prefixing `owner_did`.

#### `send` (pseudo-code)

```
send(svc, caller, conv_id, content_type, body):
    if host.read_only:                       return PermissionDenied
    if body.len() > cfg.max_body_bytes:      return QuotaExceeded
    store = store_for(svc)
    conv  = store.get_conversation(conv_id) or return NotFound
    if store.pending_count(conv_id) >= cfg.max_pending_per_conversation:
        return QuotaExceeded
    if store.message_count(conv_id) >= cfg.max_messages_per_conversation:
        return QuotaExceeded

    author = svc_address(svc)               # THIS service's own routing address
    now_ms = wall_clock_ms()
    id     = derive_message_id(author, conv_id, now_ms, content_type, body, random_nonce())
    sig    = ed25519_sign(store.sig_secret(), canonical_bytes(...))

    # ONE transaction over both tables. D-B4-27's `Queue::transaction` is
    # what makes this expressible: `Queue::enqueue` locks the shared
    # connection internally, so calling it inside a caller-held
    # `rusqlite::Transaction` on that same connection self-deadlocks
    # (`std::sync::Mutex` is not reentrant).
    store.queue.transaction(|tx, q| {
        tx.execute(INSERT INTO messages (..., outgoing=1, verified=1, state='pending'))
        q.enqueue(tx,
                  group_key = conv_id,        # D-B4-16: the DLQ cap is per conversation
                  queue_key = id,             # also the RPC idempotency key
                  payload   = serde_json(OutboxItem { svc, conv_id, id, peer_address }),
                  now       = now_ms)
    })
    return id
```

#### The delivery worker (`outbox.rs`)

```
drain_once(registry):
    services = open_stores ∪ { deployed s | conversation.db exists for s }
    for svc in services:
        drain_one(svc, queue_for(svc), still_deployed = registry.has_endpoints(svc))

drain_one(svc, queue, still_deployed):
    now = now_ms()
    for item in queue.claim_due(now, CLAIM_LIMIT_PER_TICK):
        if not still_deployed:                       queue.complete(item.id); continue
        if item.claim_count > queue.max_attempts():  # the crashed-worker bound
            settle_failed(item, "claimed repeatedly without ever completing"); continue
        parsed = serde_json::from_slice::<OutboxItem>(item.payload)
            or { queue.fail(item.id, now, "queued payload is unreadable", true); continue }
        msg = store.get_message(parsed.message_id)
        if msg is None or msg.state != 'pending': queue.complete(item.id); continue

        # D-B4-20's outer bound, checked before the attempt, not after.
        if now - item_created_at(item) > cfg.max_pending_age_secs * 1000:
            settle_failed(item, "recipient never became reachable"); continue

        match deliver(svc, parsed, msg):
            Ok(_):
                store.set_state(msg.id, 'delivered')
                notify_state(svc, msg.id, 'delivered')
                queue.complete(item.id)

            Err(e) if unreachable(e):
                # NOT queue.fail: an offline peer is not a broken target,
                # and failure-matrix row 5 says the message stays pending.
                # `defer` also un-counts this claim, or the claim_count
                # bound above would dead-letter a legitimately-waiting item.
                queue.defer(item.id, now + backoff(item.attempts))

            Err(e) if terminal(e):     # unknown address, refused, malformed, no instance cert
                settle_failed(item, e)

            Err(e):                    # a real transport failure: budget applies
                if queue.fail(item.id, now, e, false) == DeadLettered:
                    settle_failed_already_dead_lettered(item, e)

settle_failed(item, e):
    queue.fail(item.id, now, e, terminal = true)
    store.set_state(msg.id, 'failed', last_error = e)
    notify_state(svc, msg.id, 'failed')
    metrics::counter!("substrate.conversation.outbox.dead_lettered").increment(1)
    warn!(service = svc, message = msg.id, error = %e, "conversation delivery gave up")
    # No AlertStore write -- D-B4-25 and section 14.8.
```

`unreachable(e)` vs `terminal(e)` classification reuses
`proxy_outbox::disposition_of`'s shape; it does not re-derive one.

#### `deliver` (`transport.rs`)

```
deliver(svc, item, msg):
    session = crypto.session_for(store, item.peer_address)
    if session is None:
        bundle = call_peer(svc, item.peer_address, "prekey-bundle", {})   # retryable
        verify bundle.self_signature under bundle.sig_key                 # else terminal
        session = crypto.begin_session(store, item.peer_address, bundle)  # X3DH; pins sig_key
    env = crypto.encrypt(session, DeliveryPayload {
              message_id, conversation_id, author: svc_address(svc),
              sender_timestamp, content_type, body, signature: msg.signature })
    ack = call_peer(svc, item.peer_address, "deliver", env)
    crypto.commit(store, session)      # ONLY after a real Ok -- see below
    return ack

call_peer(svc, peer_address, method, params):
    # D-B4-23. Getting any of this wrong is silent, so it is checked, not assumed.
    cert  = registry.instance_cert(svc)  or return Terminal("no instance certificate")
    if cert.is_expired():                   return Terminal("instance certificate expired")
    owner = registry.owner_of(svc)       or return Terminal("no recorded owner")

    proxy.invoke(ProxyRequest {
        target_service:  peer_address,
        interface:       "conversation",
        method,
        params:          serde_json(params),
        caller:          CallerContext { proof: None, .. service context for svc },
        origin:          CallOrigin::Native { service_id: Some(svc) },
        idempotency_key: Some(message_id) for "deliver", None otherwise,
    })
```

`proof: None` is load-bearing: with `Some(..)`, `invoke_remote_at`'s **first**
match arm fires and forwards the original caller's proof — presenting a
*person* to the peer instead of the service (F10).

**Ratchet-commit ordering.** The ratchet advances on encrypt. Persisting
before the `Ok` means a failed call leaves the sender a step ahead and the
conversation permanently broken. Persisting after means a delivery that
succeeded at the receiver but lost its `Ok` re-encrypts under the *same*
message key next attempt — which the receiver's `(author, id)` dedup catches
before the ratchet ever sees it. Test exactly this (§10.2 test 4).

#### `peer_deliver` (the receiving side)

```
peer_deliver(svc, caller, env):
    if caller is anonymous:                    return PermissionDenied
    session = crypto.session_for_envelope(store, env)      # existing, or X3DH responder
    payload = crypto.decrypt(session, env)                 # terminal on failure

    author = payload.author
    if author == svc_address(svc):             return PermissionDenied   # D-B4-26
    if session is newly created:
        store.pin(author, payload.sig_key)                 # TOFU
    else if store.pinned_sig_key(author) != session.sig_key:
        return PermissionDenied                            # never silently re-pin
    if not ed25519_verify(pinned_sig_key(author), payload.signature, canonical_bytes(payload)):
        return PermissionDenied
    if payload.sender_timestamp > now_ms() + cfg.max_clock_skew_secs*1000:
        return InvalidArgument("sender timestamp implausibly far in the future")   # D-B4-21

    conv_id = derive_conversation_id(svc_address(svc), author)
    if payload.conversation_id != conv_id:     return InvalidArgument
    store.queue.transaction(|tx, _| {
        ensure conversation (kind='direct', peer_address=author)
        INSERT OR IGNORE INTO messages (..., outgoing=0, verified=1, state='delivered')
        crypto.commit_in(tx, session)
    })
    notify_message(svc, msg)     # best-effort, after commit
    return DeliveryAck { message_id: payload.message_id }
```

The `INSERT OR IGNORE` plus the unique `(author, id)` index is the whole of
receiver-side dedup; a repeat returns the same `Ok`, which is what makes
`D-B4-10` safe.

**Note what is *not* here.** Revision 1 checked
`payload.sender_did != caller.subject_did`. That check compares a service
DID against an owner's Master DID and can never pass (F16), and
`CallerContext` has no `subject_did` field in the first place. The two
checks above replace it and are strictly stronger — they survive a relayed
message, which the transport-identity check never could (§15 item 5).

### 5.2 The crypto seam (`crypto.rs`)

```rust
#[async_trait::async_trait]
pub trait SessionCrypto: Send + Sync + fmt::Debug {
    async fn prekey_bundle(&self, store: &ConversationStore)
        -> Result<PrekeyBundle, CryptoError>;
    async fn begin_session(&self, store: &ConversationStore, peer_address: &str,
                           bundle: &PrekeyBundle) -> Result<Session, CryptoError>;
    async fn session_for_envelope(&self, store: &ConversationStore, env: &Envelope)
        -> Result<Session, CryptoError>;
    async fn session_for(&self, store: &ConversationStore, peer_address: &str)
        -> Result<Option<Session>, CryptoError>;
    fn encrypt(&self, s: &mut Session, p: &DeliveryPayload) -> Result<Envelope, CryptoError>;
    fn decrypt(&self, s: &mut Session, e: &Envelope) -> Result<DeliveryPayload, CryptoError>;
    async fn commit(&self, store: &ConversationStore, s: &Session) -> Result<(), CryptoError>;
    fn commit_in(&self, tx: &Transaction<'_>, s: &Session) -> Result<(), CryptoError>;
}
```

`PrekeyBundle { address, sig_key, dh_identity, signed_prekey, signed_prekey_sig,
one_time_prekey: Option<..>, self_signature }`. `self_signature` is over the
whole bundle under `sig_key`, so a bundle is self-consistent before any of
it is trusted; `sig_key` itself is pinned TOFU.

**B4a** ships one implementation, refused at startup unless a test-only flag
is set:

```rust
/// NOT A RATCHET. One static ECDH-P256 + AES-GCM session per peer, no
/// forward secrecy, no rotation, so the storage, ordering, outbox, and
/// transport can be built and tested before the real key agreement lands.
/// `ConversationService::new` refuses it unless
/// `ConversationConfig::allow_insecure_crypto` is set, which no config file
/// can set and only this workspace's own tests do.
pub struct StaticEcdhSessionCrypto { … }
```

**B4b** replaces it with `X3dhDoubleRatchet` and **deletes**
`StaticEcdhSessionCrypto` and `allow_insecure_crypto`.

### 5.3 Authorization

Every guest-facing method reaches **its own** store, keyed by
`HostState.component_id`, never by anything the guest passes. No
cross-service conversation access exists and no interface for one.

`read_only` hard-denies `send`, `open_direct`, and `retry`
(`permission-denied`), exactly as `store::Host::put` does at
[host_capabilities.rs:621](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L621).
Reads are allowed.

**No new capability `Ability` is introduced.** `data-layer/admin`'s
precedent does not apply: no conversation operation destroys another
principal's policy-protected rows.

### 5.4 The `HostState` impl (`crates/sandbox_wasm/src/host_capabilities.rs`)

Per B3 §13 item 1, this is where it must live.

- New field `pub conversation: Weak<dyn ConversationHost>`, where
  `ConversationHost` is an object-safe trait in **`syneroym-rpc`** (beside
  `ServiceProxy`) that `ConversationService` implements. `Weak` for the
  Slice-6B cycle reason; the trait in `syneroym-rpc` because
  `syneroym-sandbox-wasm` must not depend on `syneroym-conversation`.
- **`HostState::new`'s signature does not change** (`D-B4-24`, F19). The
  field defaults to `Weak::new()`; a `#[must_use] pub fn with_conversation(mut self, …) -> Self`
  builder sets it. Only the two real construction sites change, not the 24
  test/bench ones — and an unset `Weak` upgrade-fails to
  `internal("no conversation capability on this node")`, which is the
  correct answer for a node that runs none.
- `impl conversation::Host for HostState` — **seven** methods, each:
  `read_only` check where it mutates, upgrade the `Weak`, delegate with
  `self.component_id.clone()` and `self.caller.clone()`, map the error.
- One line in `build_wasm_linker`:
  `syneroym_wit_interfaces::conversation_host::syneroym::conversation::conversation::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |s| s)?;`

### 5.5 The engine (`crates/sandbox_wasm/src/engine.rs`)

- `pub conversation: OnceLock<Weak<dyn ConversationHost>>` beside
  `service_proxy` ([engine.rs:200](../../../../crates/sandbox_wasm/src/engine.rs#L200)),
  `OnceLock::new()` at every construction site, and read with the same
  fallback shape at
  [engine.rs:1256](../../../../crates/sandbox_wasm/src/engine.rs#L1256):
  `self.conversation.get().cloned().unwrap_or_default()` feeding
  `HostState::with_conversation`. **`AppSandboxEngine::init`'s 8 parameters
  and 72 call sites are untouched** (`D-B4-24`).
- `notify_guest_message` / `notify_guest_state`, copied from
  `deliver_message` ([engine.rs:1547](../../../../crates/sandbox_wasm/src/engine.rs#L1547)),
  targeting `"syneroym:conversation/guest-api@0.1.0"`, under
  `CallerContext::service_system`, with the same 4-attempt instantiation
  retry and the same log-and-give-up ending.
- `impl ConversationNotifier for AppSandboxEngine` (the second object-safe
  trait in `syneroym-rpc`), held `Weak` by `ConversationService`.

### 5.6 `syneroym-async-queue` changes (`D-B4-27`, F18)

Three additions, all inside `crates/async_queue/src/lib.rs`:

```rust
impl Queue {
    /// Runs `f` inside one transaction on this queue's own connection,
    /// handing the caller both the transaction and a queue handle that
    /// writes through it. For an owner whose own tables live in the same
    /// file and must commit atomically with the enqueue -- `enqueue`
    /// itself takes this connection's lock, so an owner cannot hold a
    /// `Transaction` on it and call `enqueue` too.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>, &TxQueue<'_>) -> Result<T>,
    ) -> Result<T>;

    /// Pushes a claimed item back to `visible_at` **without** charging the
    /// attempt budget, and un-counts this claim so the poison-pill bound
    /// (`claim_count > max_attempts`) does not fire on an item that is
    /// deliberately waiting. For a target that is absent rather than
    /// broken; a caller using this owes its own outer bound.
    pub fn defer(&self, id: i64, visible_at: i64) -> Result<()>;
}

/// The transaction-scoped half of `Queue`. Same SQL, borrowed connection.
pub struct TxQueue<'a> { … }
impl TxQueue<'_> {
    pub fn enqueue(&self, tx: &Transaction<'_>, group_key: &str, queue_key: &str,
                   payload: &[u8], now: i64) -> Result<i64>;
}
```

`defer` is `UPDATE outbox SET visible_at = ?1, claim_count = claim_count - 1
WHERE id = ?2`. `claim_due` already selects on `WHERE visible_at <= ?1` with
`idx_outbox_visible_at` behind it
([lib.rs:459](../../../../crates/async_queue/src/lib.rs#L459),
[lib.rs:285](../../../../crates/async_queue/src/lib.rs#L285)), so no query
changes.

Each needs its own unit test in that crate, including one proving `defer`
does **not** advance `attempts` and one proving a `transaction` that returns
`Err` rolls back both the caller's row and the enqueue.

### 5.7 The native-dispatch arm (`crates/control_plane/src/synsvc_native.rs`)

- Add `"conversation" => self.dispatch_conversation(invocation).await` — the
  **sixth** arm over a now-**seven**-name array (F9: `http-native` still has
  no arm, by design).
- `dispatch_conversation` handles **two** methods: `prekey-bundle` and
  `deliver`. Everything else is `MethodNotFound`. *(Revision 1 listed a
  third, `ack`; `deliver` returns `DeliveryAck` synchronously and nothing
  ever called it.)*
- It exposes **none** of the guest-facing verbs: a peer must not be able to
  call `history` or `send` on this node.
- Add `"conversation"` to `NATIVE_CAPABILITY_INTERFACES`
  ([local_registry.rs:40](../../../../crates/core/src/local_registry.rs#L40)),
  widening `[&str; 6]` → `[&str; 7]`, and update the enumeration in
  [preamble.rs](../../../../crates/router/src/preamble.rs)'s
  "Reserved native-capability interfaces" bullet.

---

## §6 The dual-build surface

### 6.1 `crates/app_host/src/types.rs`

```rust
pub mod conversation {
    pub use syneroym_wit_interfaces::conversation::syneroym::conversation::conversation::{
        ConversationError, ConversationKind, ConversationSummary, DeliveryState, HistoryPage,
        Message,
    };
}
```

### 6.2 `crates/app_host/src/lib.rs`

```rust
/// Mirrors `syneroym:conversation/conversation@0.1.0`, function for
/// function.
pub trait AppConversation {
    fn open_direct(&self, peer_address: String)
        -> impl Future<Output = Result<String, ConversationError>> + Send;
    fn conversations(&self)
        -> impl Future<Output = Result<Vec<ConversationSummary>, ConversationError>> + Send;
    fn send(&self, conversation: String, content_type: String, body: Vec<u8>)
        -> impl Future<Output = Result<String, ConversationError>> + Send;
    fn history(&self, conversation: String, limit: u32, cursor: Option<String>)
        -> impl Future<Output = Result<HistoryPage, ConversationError>> + Send;
    fn delivery_status(&self, message: String)
        -> impl Future<Output = Result<DeliveryState, ConversationError>> + Send;
    fn outbox(&self)
        -> impl Future<Output = Result<Vec<Message>, ConversationError>> + Send;
    fn retry(&self, message: String)
        -> impl Future<Output = Result<(), ConversationError>> + Send;
}

/// The host -> app direction for conversations. `MessageSink`'s shape
/// (B3 section 4.1), for the same reason: part of the app-facing contract,
/// and used as `dyn`.
#[async_trait::async_trait]
pub trait ConversationSink: Send + Sync + core::fmt::Debug {
    async fn on_message(&self, msg: Message) -> Result<(), String>;
    async fn on_delivery_state(&self, message: String, state: DeliveryState)
        -> Result<(), String>;
}
```

Seven methods, matching §3 and §5.4.

`AppHost` widens to `AppDataLayer + AppBlobStore + AppMessaging + AppConversation + Send + Sync`.
Breaking, with two implementors (`GuestHost`, `NativeAppHost`) and one
consumer (the fixture), all in this plan's scope. See §14.5 for the
compatibility question this raises for a *third* app.

### 6.3 `crates/app_host/src/guest.rs`

`impl AppConversation for GuestHost` — seven `async fn` bodies calling
`conv::<fn>(...)` where
`conv = syneroym_wit_interfaces::conversation::syneroym::conversation::conversation`.

### 6.4 `crates/app_host/Cargo.toml`

Add `"conversation"` to the narrowed feature list.

### 6.5 `crates/app_host_native/`

- `src/host.rs`: `impl AppConversation for NativeAppHost` — seven
  delegations to `HostState`'s `conversation::Host` impl, in the shape the
  `AppDataLayer` delegations already have.
- `src/convert.rs`: six new types (`ConversationError`, `DeliveryState`,
  `ConversationKind`, `ConversationSummary`, `Message`, `HistoryPage`), one
  round-trip unit test each, matching the existing thirteen.
- `src/factory.rs`: `NativeHostFactory::new` gains an
  `Arc<ConversationService>`; `host_with` calls
  `.with_conversation(Arc::downgrade(&self.conversation) as Weak<dyn ConversationHost>)`
  on the `HostState` it builds. `set_conversation_sink(Weak<dyn ConversationSink>)`
  beside `set_sink`, and the factory registers itself with the service's
  sink map so the worker can wake a natively-linked app.
- `src/lib.rs`: re-export `ConversationSink`.

**Permitted difference, stated in code as well as here.** The WASM build is
woken by instantiate-and-call with a 4-attempt retry; the native build by a
`Weak<dyn ConversationSink>` with none. The *timing* differs and the retry
differs — the same permitted difference B3 documented for `MessageSink`.
What must be identical is the **store contents** afterwards, and that is
what the parity suite compares.

---

## §7 The fixture

`test-components/dual-build-fixture/` — **six files**:

1. **`Cargo.toml`** — `"conversation"` added to the narrowed
   `syneroym-wit-interfaces` feature list, and
   `"syneroym:conversation" = { path = "wit/deps/conversation" }` added to
   `[package.metadata.component.target.dependencies]`.
2. **`wit/deps/conversation/`** — a **real directory** containing a **file**
   symlink, matching the existing three exactly:
   ```
   wit/deps/conversation/conversation.wit
       -> ../../../../../crates/wit_interfaces/wit/conversation/conversation.wit
   ```
   Five `..` levels, to the `.wit` file — *not* a directory symlink.
3. **`wit/world.wit`** — `import syneroym:conversation/conversation@0.1.0;`
   and `export syneroym:conversation/guest-api@0.1.0;`.
4. **`src/guest.rs`** — add
   `"syneroym:conversation/conversation@0.1.0": syneroym_wit_interfaces::conversation::syneroym::conversation::conversation`
   to the `with:` map, and `impl` the conversation `guest-api` forwarding to
   `crate::app::on_conversation_message` / `on_conversation_state`.
5. **`src/native.rs`** — `impl ConversationSink for NativeFixture`,
   forwarding to the same two `app.rs` functions.
6. **`src/app.rs`** — new `Request` variants, JSON in/out per `D-B3-10`:

   | Verb | What it proves |
   |---|---|
   | `open-conversation { peer_address }` | id is stable and idempotent |
   | `send-message { conversation, body }` | returns an id, state `pending` |
   | `read-history { conversation, limit }` | ordering, paging, content, `verified` |
   | `delivery-status { message }` | the state a user sees |
   | `read-outbox` | G2's `pending`/`failed` surface |
   | `retry-message { message }` | re-arming a `failed` message |
   | `read-conversation-inbox` | what `on_message` stored — through `data-layer`, never in-process state (`D-B3-12`) |
   | `read-state-log` | what `on_delivery_state` stored, same rule |

`app.rs` must still name no substrate crate — `grep`-checkable, and exit
criterion 3 of B3's own "done" list.

---

## §8 Substrate wiring

### 8.1 `crates/core/src/config.rs`

New `AppSandboxRole` fields beside the existing `queue_*` ones (`D-B4-19`):

| Field | Default | Meaning |
|---|---|---|
| `conversation_tick_secs: u64` | 5 | worker tick, min-clamped to 1 like `queue_tick_secs` |
| `conversation_max_body_bytes: u32` | 262_144 | per-message body cap |
| `conversation_max_pending_per_conversation: u32` | 1_000 | failure-matrix row 12 |
| `conversation_max_messages_per_conversation: u32` | 100_000 | ditto |
| `conversation_max_pending_age_secs: u64` | 2_592_000 (30 d) | `D-B4-20`'s outer bound |
| `conversation_max_clock_skew_secs: u64` | 86_400 (24 h) | `D-B4-21` |
| `conversation_prekey_pool_size: u32` | 100 | one-time prekeys held |
| `conversation_prekey_requests_per_peer_per_hour: u32` | 20 | `D-B4-15` |

Each needs a `default_*` fn and a doc comment saying *why* that number, in
the style the `queue_*` fields use.

### 8.2 `crates/substrate/src/runtime.rs`

- Build `ConversationService` in `init`, where `build_route_handler_deps`
  builds the blob provider and logical resolver; add it to
  `SharedNodeHandles` so `init_dual_build_fixture` can reach it.
- Wire both directions **after** both objects exist, via the `OnceLock`
  setters (`D-B4-24`): `engine.conversation.set(Arc::downgrade(&conv))` and
  `conv.set_notifier(Arc::downgrade(&engine))`. This is the
  `ControlPlaneService.service_proxy` precedent, and the Slice-6B `Arc`-cycle
  reason for `Weak` on both sides.
- Spawn the worker beside `proxy_outbox_join`
  ([runtime.rs:289](../../../../crates/substrate/src/runtime.rs#L289)):

  ```rust
  self.conversation_worker_join = self.conversation.clone().map(|svc| {
      let tick = Duration::from_secs(
          config.roles.app_sandbox.as_ref().map_or(5, |r| r.conversation_tick_secs).max(1),
      );
      let cancel = self.conversation_worker_cancel.clone();
      tokio::spawn(async move { svc.run_worker(tick, cancel).await })
  });
  ```

- Three places, mirroring `proxy_outbox_join` exactly: the struct field, the
  `tokio::select!` arm, and the `drop(...take())` shutdown block.
- `init_dual_build_fixture`
  ([runtime.rs:917](../../../../crates/substrate/src/runtime.rs#L917))
  changes because `NativeHostFactory::new` gains an argument.

---

## §9 Every call site that changes

| File | Change |
|---|---|
| `crates/wit_interfaces/wit/conversation/conversation.wit` | **new** (§3) |
| `crates/wit_interfaces/src/conversation.rs` | **new** — guest bindgen, `conversation-import` |
| `crates/wit_interfaces/src/conversation_host.rs` | **new** — wasmtime bindgen |
| `crates/wit_interfaces/src/lib.rs` | two `pub mod` lines |
| `crates/wit_interfaces/Cargo.toml` | `conversation = []`, added to `default` |
| `crates/conversation/**` | **new crate** `syneroym-conversation` (§5.1) |
| `Cargo.toml` (workspace) | new member + workspace dependency |
| **`crates/async_queue/src/lib.rs`** | **`Queue::transaction`, `Queue::defer`, `TxQueue` (§5.6) — `D-B4-27`** |
| `crates/rpc/src/conversation.rs` (new) + `lib.rs` | `ConversationHost`, `ConversationNotifier` object-safe traits |
| `crates/sandbox_wasm/src/host_capabilities.rs` | `HostState.conversation` field (default `Weak::new()`), `with_conversation` builder, `impl conversation::Host` |
| `crates/sandbox_wasm/src/engine.rs` | `conversation: OnceLock<..>` field + 3 construction sites, read-with-fallback at instantiation, one `add_to_linker` line, `notify_guest_message`/`notify_guest_state`, `impl ConversationNotifier` |
| **`crates/sandbox_wasm/benches/wasm_engine.rs`** | none — `HostState::new`'s signature is unchanged (`D-B4-24`) |
| **`crates/sandbox_wasm/tests/{lifecycle_hooks,abac_integration,blob_store_integration}.rs`** | none, same reason. Listed so the grep hits are explained rather than chased |
| `crates/core/src/local_registry.rs` | `NATIVE_CAPABILITY_INTERFACES` `[&str; 6]` → `[&str; 7]` |
| `crates/router/src/preamble.rs` | the reserved-interface enumeration in the module doc |
| `crates/control_plane/src/synsvc_native.rs` | sixth `dispatch` arm + `dispatch_conversation` (two verbs) |
| `crates/control_plane/Cargo.toml` | `syneroym-conversation` dependency |
| `crates/router/src/service_async_db.rs` | **deleted**, moved (`D-B4-12`) |
| `crates/data_db/src/service_db.rs` (new) + `lib.rs` | `pub async fn service_db_location` + its error enum |
| `crates/router/src/{proxy_outbox,call_dedup,saga}.rs` | three import updates for the moved function |
| `crates/core/src/config.rs` | eight `AppSandboxRole` fields + defaults |
| `crates/substrate/src/runtime.rs` | construct, `OnceLock`-wire both ways, spawn, select, shutdown; `init_dual_build_fixture`'s new argument |
| `crates/substrate/Cargo.toml` | `syneroym-conversation` dependency |
| `crates/app_host/src/{lib,types,guest}.rs`, `Cargo.toml` | §6.1–6.4 |
| `crates/app_host_native/src/{lib,host,factory,convert}.rs` | §6.5 |
| `test-components/dual-build-fixture/**` | §7, six files |
| `crates/app_host_native/tests/dual_build_parity.rs` | §10.1 |
| **`crates/substrate/tests/dual_build_fixture_e2e.rs`** | `NativeHostFactory::new`'s new argument |
| `crates/substrate/tests/conversation_e2e.rs` | **new** (§10.2), with the per-binary lock |
| `crates/substrate/tests/reference_scenario_e2e.rs` | scenario steps 6–8, if that file is the right home — check its current scope first |
| `apps/roymctl/src/commands/` | **§14.4** — decide explicitly; not planned here |

**Grep list to run before declaring §9 complete:**

```bash
rg -n 'NATIVE_CAPABILITY_INTERFACES' crates apps docs
rg -n 'HostState::new\(' crates                      # expect 26 hits, 0 edits (D-B4-24)
rg -n 'AppSandboxEngine::init\(' crates apps         # expect 72 hits, 0 edits (D-B4-24)
rg -n 'async_db_location' crates
rg -n 'impl AppHost|dyn AppHost|: AppHost' crates test-components
rg -n 'NativeHostFactory::new' crates test-components
rg -n 'caller\.subject_did|session\.subject_did' crates   # F16: know which you mean
```

---

## §10 Tests

### 10.1 The parity suite (`dual_build_parity.rs`)

The local half — everything needing no second node. Extend `SCENARIOS`:

- `open-conversation` twice → same id, identical across builds.
- `send-message` → `delivery-status` is `pending`; `read-outbox` shows it.
- three sends with controlled timestamps → `read-history` in
  `(sender_timestamp, author, id)` order, byte-identical across builds.
- a body over `conversation_max_body_bytes` → `quota-exceeded` on both.
- a send past `max_pending_per_conversation` → `quota-exceeded` on both.
- `retry` on a `pending` message → `invalid-argument` on both.
- `read-history` with a `limit` below the message count → same page, same
  `next-cursor` on both.
- a message whose `author` is this service's own address, injected through
  `peer_deliver` → refused on both (`D-B4-26`).

Host→app direction, driven by injecting straight into each stack's
`ConversationService::peer_deliver` (no network in this suite):

- `read-conversation-inbox` shows it on both, with `verified: true`;
- a payload with a corrupted signature → not stored, on both;
- `read-state-log` shows a `delivered` transition on both.

Add a `permitted_differences` entry for the wake-mechanism difference
(§6.5), with a test asserting the **store** converges though the mechanism
differs. Extend `the_parity_comparison_detects_a_divergence` to a
conversation field — otherwise the new scenarios' green is not evidence.

### 10.2 Cross-node e2e (`crates/substrate/tests/conversation_e2e.rs`, new)

Two real substrates via `multi_substrate_placement_e2e.rs`'s `Node`
harness, **not** `SubstrateTestContext` (F13).

**Two setup requirements the first revision omitted:**

1. **A per-binary static lock**, acquired once per test **before either node
   boots** — not per-node, which deadlocks a two-node test. Eleven two-node
   tests in one binary without it reintroduces exactly the CPU-starvation
   failure [deferred-backlog.md](../../deferred-backlog.md) row 39
   root-causes (F20). Copy the lock from
   `multi_substrate_placement_e2e.rs`, the same harness this file borrows.
2. **Every deployed conversation service declares `visibility = "internal"`**
   in its `ServiceConfig`. B2's `D-B2-15` makes this mandatory for anything
   needing cross-node resolution, and the failure mode is a late, misleading
   *"No valid Iroh mechanism found"* rather than a deploy error. The harness
   already imports `Visibility` from `syneroym_app_orchestration`.
   Additionally, each service needs an **installed, unexpired instance
   certificate and a recorded owner**, or `D-B4-23` refuses every send with
   a terminal error.

| # | Test | Matrix row / criterion |
|---|---|---|
| 1 | A sends while B is down → `pending`; nothing at B | Row 5, scenario step 6 |
| 2 | Restart both with a message in flight → still `pending`, exactly one outbox row, no double-send | **Row 4**, criterion 4, step 7 |
| 3 | B comes up → delivered; A reads `delivered`; B's `history` has it, `verified: true` | Row 3, criterion 4, step 8 |
| 4 | B is up but its ack is dropped → A retries; B stores exactly one copy | `D-B4-11`, the ratchet-commit ordering |
| 5 | A's `delivery-status` never reads `delivered` while B is held down, polled across the retry window | **Row 3**, criterion 5 |
| 6 | Subscribe `#` on both brokers for the whole run; assert no durable body bytes appear | **Row 6**, criterion 6, F11 |
| 7 | Node C delivers an envelope whose `author` claims A's address | Refused — the signature does not verify under A's pinned key (`D-B4-22`) |
| 8 | A peer re-presents a *different* signing key for a pinned address | Refused; never silently re-pinned |
| 9 | A guest calls `conversation` on **another** service through `syneroym:proxy` | Refused by `check_native_capability_gate` |
| 10 | A guest calls `conversation/deliver` on **its own** service id (the same-service exemption, F17) | Reaches the arm, and is refused because it cannot sign as a peer — `D-B4-26`'s invariant, tested rather than assumed |
| 11 | `prekey-bundle` past the per-peer hourly limit | Refused; the pool is not drained (`D-B4-15`) |
| 12 | A conversation exceeding `max_pending_per_conversation` | That conversation gets `quota-exceeded`; a second conversation on the same node still sends | **Row 12** |
| 13 | A message arrives with `sender_timestamp` a year in the future | Refused (`D-B4-21`); one a year in the past is accepted |
| 14 | A peer offline past `max_pending_age_secs` (clock injected) | `failed`, and `retry` re-arms it (`D-B4-20`) |
| 15 | A send from a service with no installed instance certificate | Terminal failure naming the certificate, not a silent node-key fallback (`D-B4-23`) |
| 16 | A opens a conversation by B's **registry alias** while B opens one by A's **DID** | Both derive the *same* conversation id, because `open-direct` canonicalized first (`D-B4-29`) |
| 17 | `open-direct` on an address that resolves to nothing | Refused at the call, not hours later in the outbox (`D-B4-29`) |

### 10.3 Unit tests inside `syneroym-conversation`

- `ids.rs`: conversation id is order-independent over the address pair and
  identical on both sides; message id changes with the nonce, stable
  otherwise.
- **the one-string invariant** (§5.1): for a deployed service, `component_id`,
  the conversation store's namespace, and the published
  `EndpointInfo.service_id` are the same string. Asserted, not assumed —
  `derive_conversation_id` agreeing across two nodes is `D-B4-5`'s
  load-bearing property, and it holds only while these three coincide.
- `envelope.rs`: `canonical_bytes` cannot be made ambiguous by moving bytes
  across the `body` length prefix; a one-bit change anywhere fails verify.
- `store.rs`: the `send` transaction is atomic (injected failure between the
  two writes → neither row exists); the `(author, id)` index rejects a
  repeat; the ordering index returns the documented order under a skewed
  clock.
- `crypto.rs`: `commit` is not called on a failed delivery, and the next
  attempt derives the same message key (the dropped-ack case).
- `outbox.rs`: the three `ProxyRouter` subtleties (F1), plus `defer` not
  advancing `attempts` and not tripping the `claim_count` bound.

### 10.4 `syneroym-async-queue` unit tests (§5.6)

`transaction` rolls back both writes on `Err`; `defer` leaves `attempts`
untouched and decrements `claim_count`; a deferred item is invisible to
`claim_due` until `visible_at`.

### 10.5 Failure-and-security matrix ledger

Complete, so `status.md` inherits an accurate one (criterion 11):

| Rows | Owner | Evidence |
|---|---|---|
| 1, 2 | **B1** | `gateway_session_e2e.rs` |
| 3, 4, 5, 6, 12 | **B4** | §10.2 |
| 7, 8, 9, 10 | **B5** | — |
| **11** | **B2** | `service_visibility_e2e.rs` |
| 13 | **B3**, extended by **B4** | `dual_build_parity.rs`, §10.1 |

*(Revision 1's ledger left row 11 unassigned.)*

---

## §11 Ordering constraint inside B4

1. **Step 0** — settle the working tree (§0.1).
2. WIT package + three bindgen modules + the feature. Build only.
3. `syneroym-async-queue`'s three additions (§5.6) with their tests — first,
   because step 4 cannot write atomically without them.
4. `syneroym-conversation`: `ids`, `envelope`, `store`. No network, no
   real crypto.
5. `crypto.rs`: `SessionCrypto` + `StaticEcdhSessionCrypto` (B4a's
   placeholder, refused unless `allow_insecure_crypto`).
6. `transport.rs` + the native-dispatch arm + `NATIVE_CAPABILITY_INTERFACES`.
7. `outbox.rs` + the worker, wired in `runtime.rs`.
8. `HostState` impl + engine `OnceLock` + linker line + the two notifiers.
9. The three dual-build edits (§6) and the fixture (§7).
10. The parity suite (§10.1).
11. The cross-node e2e (§10.2), lock and visibility first.
12. `D-B4-12`'s move of `async_db_location` — late, so it is one isolated
    refactor commit.
13. **B4b**: replace `StaticEcdhSessionCrypto` with the real X3DH + Double
    Ratchet; delete `allow_insecure_crypto`; re-run §10 **unchanged**. If
    §10 needs changing for B4b, the seam in step 5 was drawn wrong.

The B4a/B4b boundary is between 12 and 13.

---

## §12 The completion pass (AGENTS.md)

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
mise run test:e2e
cargo component build --release --target wasm32-wasip2 -p syneroym-test-dual-build-fixture
wasm-tools validate --features component-model target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
wasm-tools component wit target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
cargo build -p syneroym-substrate --features dual_build_fixture
cargo check -p syneroym-app-host --target wasm32-wasip2
mise run build:test-components
```

`wasm-tools component wit` is not optional: it is the only check that the
fixture's *actual* import/export set is what this plan says, independent of
the linker's claims. B3 found two real toolchain problems that way.

Run `cargo test --workspace` with the sandbox **on**; the five
socket-binding crates fail with `Operation not permitted (os error 1)` and
must be re-run with the sandbox off to confirm.

---

## §13 Documents and backlog owed

| Document | Edit |
|---|---|
| [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) | §5 gets an implementation note for the third tiebreak (`D-B4-17`); §3 a note that a peer is addressed by service id in the first implementation (`D-B4-5`); **the Open Question on raw-timestamp plausibility bounds is answered** by `D-B4-21` and marked resolved |
| [ADR-0023](../../../decisions/0023-durable-async-primitives.md) | A dated note on §2 recording queue-always and why §2's reasoning does not transfer (`D-B4-9`) |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | §Messaging's `libsignal-protocol-rust` sentence corrected when `D-B4-7` resolves (§14.1) |
| [task.md](task.md) | B4's row → Complete; open design points 1 and 4 resolved. **Also: the "Owed as slices land" table says "the durable-messaging and outbox rows move to Recently resolved" — correct it**, since both rows also name group delivery, which is B5's (§13 note below) |
| [status.md](status.md) | `B4 — What shipped` / `B4 — Verification Evidence`, plus §10.5's complete ledger. Note that reference-scenario step 13 is B2's, so it is not double-counted (§14.6) |
| [meta-implementation-plan.md](../../meta-implementation-plan.md) | M6 item 3's "relative-clock deterministic ordering" is stale; correct to raw sender timestamps |

**Backlog rows to edit, not close:**

- The **durable-messaging** row and the **outbox** row: split each, moving
  the 1:1/interface/state half to "Recently resolved" and leaving the group
  half targeted at B5. `task.md`'s owed-edits table is corrected to match
  (above), rather than left disagreeing with what actually happened.
- **Row 240** (`service_async_db.rs`, "a fourth per-service store is
  added"): its pickup trigger has now **fired**, and `D-B4-12` resolves only
  its cheap half — the shared `(dir, dek)` resolver. The expensive half
  (three, now four, separate caches, single-flights, and error mappings)
  stands. Re-scope the row to name the remaining duplication and record that
  the trigger fired, rather than leaving a fired trigger sitting unactioned.
  *(Revision 1 said nothing about this row; AGENTS.md's mandatory-backlog
  rule requires the edit either way.)*

**New backlog rows B4 owes:**

1. **A conversation peer is addressed by service id, never by a person**
   (`D-B4-5`). Multi-device delivery, ADR-0013 §2's Primary Substrate
   designation, and person→substrate resolution are all unbuilt.
2. **A service address is bound to its conversation key only by
   trust-on-first-use** (`D-B4-28`). Nothing in the tree independently binds
   one to the other: `derive_service_identity` is node-private and no
   registry record carries a conversation key. Pickup trigger: a registry
   record gains a key field, or ADR-0020 grows a published service key.
3. **ADR-0023 §5's alerting half is unimplemented for *both* durable queues.**
   The proxy outbox emits a metric and a log on dead-letter and writes no
   alert; the conversation outbox now matches it. `AlertStore` is
   supervisor-scoped (`AppInstanceId`-keyed, in `supervisor.db`), so a node
   running a conversation service and no supervisor has nothing to write to.
   Closing this needs a node-scoped alert sink, which is new machinery.
   (`D-B4-25`, §14.8.)
4. **No operator surface for conversation dead letters** — `roymctl` has
   `proxy-dlq`/`sagas` and nothing for this queue (§14.4).
5. **The native build's conversation wake has no retry; the WASM build's has
   four** — the same shape as B3's `MessageSink` row.
6. **`retry` re-arms one message; no bulk or per-conversation re-arm** — the
   all-or-nothing shape `Queue::replay` already has.
7. **Conversation history has no retention policy or export path.**
   `max_messages_per_conversation` bounds it; bounding is not retention, and
   R2's export goal needs a real one.
8. **The prekey pool refills lazily and the signed prekey has no rotation
   schedule.** X3DH assumes periodic rotation; B4 generates once.
9. Whatever `D-B4-7` does not choose — if B4b lands the in-house
   implementation, a "we rolled our own ratchet" row with a pickup trigger
   of "an acceptably-licensed crate becomes available".

---

## §14 Ambiguities and staleness in the input documents

### 14.1 The spec names an AGPL crate — **blocking**

F6. `D-B4-7` structures the work so it does not block B4a, but **B4 is not
done until it is answered**, and the spec sentence must be corrected.

### 14.2 "A person" is not addressable, and three documents assume it is

The spec's Conversation service and D7; ADR-0013 §3's *"any of Bob's known
substrates"*; the reference scenario's step 3. None is satisfiable: F4 and
F16 together show that neither a routing address nor the transport identity
names a person, and [task.md](task.md) forbids changing the endpoint-record
format. `D-B4-5` narrows to a service address. **Confirm before
implementation**; the alternative is building person→substrate resolution
here, a milestone-sized addition.

### 14.3 Failure-matrix row 5 and the queue's attempt budget contradict each other

Row 5 says "indefinitely"; the budget dead-letters in ~10 hours; row 12 in
the same table forbids unbounded growth. `D-B4-20` resolves it at 30 days
with an offline peer not charged against the budget. **Confirm 30 days, or
name a different number.**

### 14.4 The DLQ operator surface is specified by ADR-0023 and unscoped by task.md

ADR-0023 §5: *"the dead letter is listable and replayable by an operator
through `roymctl`."* B4 creates a second dead-letter table; task.md says
nothing about verbs for it. Either build `roymctl conv dlq` / `conv replay`
(a real §9 addition) or take backlog row 4. **Do not leave it implicit.**

### 14.5 Widening `AppHost` is a compatibility question the docs do not address

§6.2 requires every dual-build app to implement `AppConversation`, even one
that never uses conversations. B3's own backlog row anticipates this for
`syneroym:http`, `app-config`, `vault`, and `proxy`. §6.2 picks "widen it"
because the only implementors are the two in this plan. **If a second
dual-build app exists by the time B4 runs, revisit.**

### 14.6 `task.md`'s reference-scenario step 13 belongs to B2

Step 13 is B2's, shipped and covered by `service_visibility_e2e.rs`. It sits
in the same numbered list as B4's steps 6–8, which reads as if B4 owes it.
Note it in `status.md` so it is not double-counted.

### 14.7 "Group key handling" appears in G1's description but is B5's

The spec's G1 asks for *"…and group key handling"*; [task.md](task.md) puts
the group key in B5. task.md wins — it is the normative scope statement. B4
only reserves `conversation-kind::group` so B5's arrival does not break a
guest's `match`.

### 14.8 ADR-0023 §5's alerting half — accepted, with a narrowed remedy *(revision 2)*

The review is right that revision 1 dropped it silently. Two facts change
what the right remedy is, and both were checked:

- `drain_one_outbox` raises **no** alert on `FailOutcome::DeadLettered` — it
  increments `substrate.proxy.outbox.dead_lettered` and logs
  ([proxy.rs:1453](../../../../crates/router/src/proxy.rs#L1453)); `rg` for
  `AlertStore` under `crates/router/` returns nothing. The alerting half is
  already unimplemented for the queue ADR-0023 was written about.
- `AlertStore::raise` is keyed by `AppInstanceId` with supervisor-scoped
  arguments ([alerts.rs:258](../../../../crates/app_orchestration/src/alerts.rs#L258))
  and is written only from `app_supervisor`. A node running a conversation
  service and no supervisor role — the ordinary participant deployment —
  has no `AlertStore` to write to at all.

So B4 matches the existing precedent exactly (metric + `warn!`, `D-B4-25`)
and files **one** backlog row covering both queues (row 3 above). Inventing
a conversation-only alert path would not work on the deployment that needs
it, and would let the older, larger gap stay open while appearing to satisfy
the ADR.

---

## §15 What B4 owes B5

1. **`SessionCrypto` is the 1:1 channel the group key rides on** (ADR-0013
   Amendment 1, task.md's fourth open design point). B5 must decide
   explicitly whether key material shares the ratchet with content;
   `DeliveryPayload.content_type` exists so a key-distribution payload needs
   no format change.
2. **Ordering is `(sender_timestamp, author, message_id)`** (`D-B4-17`),
   already an index. B5's DAG sorts on the same key or transcripts will not
   be byte-identical.
3. **Attribution is already signature-based, not transport-based**
   (`D-B4-22`). *This is the single most important thing B4 hands B5*, and
   revision 1 got it backwards: it used a transport check and told B5 to
   replace it. A relay forwards a message it did not write, so a
   transport-identity check could never have survived into B5. It does not
   have to now — `verify(pinned_sig_key(author), signature, canonical_bytes)`
   works identically for a direct delivery and a relayed DAG entry.
4. **`conversation-kind::group` and `conversation-summary.participants` are
   already in the WIT.** B5 adds verbs, not new records.
5. **`messages` has no DAG parent column.** B5 adds one plus a `dag_entries`
   table — an additive schema change to a pre-release product.
6. **`D-B4-21`'s clock-skew bound applies per received entry**, so B5 gets it
   for relayed entries too — but B5 must decide what a *relay* does with an
   entry it must forward and cannot accept.
7. **Pinning is trust-on-first-use, and a group multiplies the parties it
   applies to** (`D-B4-22`, `D-B4-28`, backlog row 2). In 1:1 the assumption
   is bounded: two parties, one pinning each, and a wrong pin breaks exactly
   that one conversation. A group entry is authored by one member and
   verified by every other, so each member independently TOFU-pins each
   author — and an owner adding a member the others have never contacted
   means N first-contact pinnings with no prior session to lean on. **B5 must
   state explicitly** whether a membership event carries the joiner's signing
   key (letting the owner's already-pinned identity vouch for it, narrowing
   the trust window to the owner alone — which is consistent with Amendment
   1 already making the owner the single point of trust for key
   distribution), or whether every member pins independently on first entry.
   Do not inherit this silently.

---

## §16 What "done" means for B4

1. `syneroym:conversation@0.1.0` is imported by the fixture's component and
   appears in `wasm-tools component wit`'s output — proving it links.
2. The parity suite passes over the new scenarios **and**
   `the_parity_comparison_detects_a_divergence` fails on a corrupted
   conversation field. (Criterion 2, row 13.)
3. A 1:1 message survives a restart on both sides, and
   `pending`/`delivered`/`failed` is readable by guest code through the host
   interface. (Criterion 4, row 4.)
4. A test holding the recipient offline proves a message is never reported
   `delivered` before acknowledgement. (Criterion 5, row 3.)
5. A `#` broker subscription proves no durable content traverses
   `syneroym:messaging`. (Criterion 6, row 6.)
6. Exhausting one conversation's bounds degrades that conversation, not the
   node. (Row 12.)
7. **Every message is signed, and an unverifiable or wrongly-attributed
   message is refused** — tested for a third-party forgery, a re-pin
   attempt, and a self-injection through the same-service exemption
   (`D-B4-22`, `D-B4-26`).
8. `D-B4-7` is resolved and the real X3DH + Double Ratchet is in:
   `StaticEcdhSessionCrypto` and `allow_insecure_crypto` are **deleted**,
   not merely unused. The spec's `libsignal-protocol-rust` sentence is
   corrected.
9. §12's command list is clean, in that order.
10. No `D-B4`/`D-06B`/slice id appears in any code comment in the diff.
11. §13's documents and backlog rows are updated in the same change,
    including row 240's re-scope and `task.md`'s owed-edits correction.
12. **The three ADR-0013 edits have actually landed in the ADR file**,
    checked by opening it rather than by intending to: §5's third-tiebreak
    note (`D-B4-17`), §3's service-address note (`D-B4-5`), and — the one
    most at risk — the **Open Question on raw-timestamp plausibility bounds
    marked resolved** by `D-B4-21`, with the 24-hour bound written there.
    That bound is a security property, not a note: a far-future timestamp
    pins a message to the top of every participant's history permanently.
    A security property that lives only in a slice plan does not survive the
    plan being archived.

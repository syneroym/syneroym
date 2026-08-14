# M06A Slice A2 — Guest HTTP Route Target: Implementation Plan

> **Milestone:** [task.md](task.md) · **Slice:** A2 · **Status:** Planned, not
> started. **Revision 4** (2026-08-14), after three reviews.
>
> Decision ids are `D-A2-n`. Milestone-level decisions (`D-06A-n`) live in
> [task.md](task.md) and are inputs here. Slice A1's decisions (`D-A1-n`) live
> in [slice-a1-implementation-plan.md](slice-a1-implementation-plan.md); A1 is
> **shipped**, so its decisions are facts on the ground here, not proposals.
>
> **Scope, from task.md's slice table:** "A fourth `dispatch_route` target that
> hands method, path, params, headers, and body to the component and turns its
> return into an HTTP response; error and status mapping; body size caps;
> behaviour within the existing 5s `dispatch_epoch_timeout_secs` bound."

---

> **§0–§0b are a historical review log, not the design.** Each records what one
> review found and what that revision did about it, so a later revision can
> reverse an earlier one and both rows stay. **`§2 Decisions` is always the
> current rule**; where a later revision overturned an earlier disposition, the
> earlier row says so inline. Skip to §1 if you only need the design.

## §0 Review response (revision 2)

Every finding from the 2026-08-14 review was verified against the tree before
disposition. All twelve are accepted. **Two reversed a revision-1 decision**
(findings 8+9 and finding 10); the rest are gaps, not errors.

| # | Finding | Disposition |
|---|---|---|
| 1 | `--custom-config` cannot reach the substrate: the SDK helper hardcodes `custom_config: None` | **Accepted.** Verified [sdk/src/lib.rs:727](../../../../crates/sdk/src/lib.rs#L727). Fixed **differently** than A1 did: `deploy_svc_wasm_with_assets` has exactly **one** call site, so it is *replaced* by an options-struct form rather than a third `deploy_svc_wasm_*` variant being added beside it. `D-A2-9`, call site 22 |
| 2 | Two more module docs enumerate the three targets | **Accepted.** Call sites 17, 19 |
| 3 | `docs/system-architecture.md`'s "HTTP Passthrough" bullet becomes wrong | **Accepted, and it is already half-wrong today** — the `stream` target has never gone through `dispatch_native`, so "bridged routes return HTTP 401" was already untrue for it. §10.7 |
| 4 | The classifier refactor under-specifies `authorize_rows`' memory-fault behaviour | **Accepted — this would have been a silent behaviour change.** Verified: [engine.rs:1855-1868](../../../../crates/sandbox_wasm/src/engine.rs#L1855) has **no** memory arm, so a memory fault falls to `AbacError::Trap`. Pinned explicitly in call site 14 |
| 5 | The fixture has no `--interfaces` answer | **Accepted, on a different reason than revision 2 gave** — see revision 3's finding 2: `parse_interfaces` does *not* reject a blank value, it defaults it. The fixture still exports a `test-driver` interface, now justified solely as a second assertion channel. §6 |
| 6 | Nothing bounds concurrent guest HTTP instantiations; pool exhaustion is a hard error, not a wait | **Accepted.** Verified the accounting: default pool 10, `STREAM_INSTANCE_POOL_HEADROOM = 2`, `stream_budget = 8`, `abac_budget * 2 = 2` — **8 + 2 = 10, exactly saturated, so "ordinary calls" have zero dedicated slots today**. A browser is the first workload to drive this. `D-A2-11` |
| 7 | The new path bypasses `active_instances`/`execution_ms` | **Accepted.** Call site 12 |
| 8 | The guest cannot see who called it, so "the guest owns its own authorization" is unreachable | **Accepted — the decisive finding.** Verified: no guest-facing WIT carries a caller identity (`authorizer.wit`'s `subject-did` is the stage-4 after-step only). `http-request` gains `caller: option<caller-identity>` (`D-A2-12`) |
| 9 | Unreconciled with A1's opposite answer in the same milestone | **Accepted — revision 1's `D-A2-7` is reversed.** A guest route is now **authenticated by default** with an explicit `public: bool` opt-in, mirroring `D-A1-1` exactly. Argued in `D-A2-7` |
| 10 | The WIT world placement contradicts the closer precedent | **Accepted.** Verified [engine.rs:671-675](../../../../crates/sandbox_wasm/src/engine.rs#L671): `AUTHORIZER_INTERFACE` is "deliberately not part of the `host-environment` world". `incoming-handler` is in the identical position. No `host.wit` change, no symlink |
| 11 | A3 gets no SPA deep-link mechanism from A1+A2 | **Accepted.** Verified `match_path` requires equal segment counts and has no wildcard. §9.8 |
| 12 | Matrix row 5 is half-tested | **Accepted, taking the "say so plainly" option** — there is no guest-reachable host function that blocks unboundedly today, so the other half cannot be provoked. §10.5 |
| — | `1862-1871` is a line off | **Accepted.** Now `:1863` |

**Two consequences the review did not name, which follow from its own
findings:**

**R2-A — findings 8 and 9 are the same finding.** Revision 1's argument for
admitting anonymous callers rested on "the guest is the application logic and
owns its own authorization." Finding 8 shows the guest is identity-blind, so
that sentence described something the code could not do. Adding the caller
field (finding 8) would make the sentence true — but it does not make it
*sufficient*, because the failure mode of forgetting is still "my write
endpoint is open to the world," silently. A1 faced the same asymmetry from the
other side (revision-3 finding 6: private-by-default fails silently *for the
operator*) and answered it with **safe default plus a loud signal**, not with a
permissive default. A2 now gives the same answer. Both changes ship: the opt-in
because the default must be safe, the caller field because a `public: true`
route still needs to tell a signed-in caller from a stranger.

**R2-B — `stream` becomes the inconsistent one.** With `D-A2-7` reversed, the
`stream` target is the only HTTP-bridge target that reaches guest code with no
caller check and no declaration ([http.rs:1016-1021](../../../../crates/router/src/route_handler/http.rs#L1016)).
That is pre-existing and A2 does not change it — extending `public` to the
`stream` target touches M3B behaviour for a slice that has no consumer needing
it. Backlog row owed; §9.5.

## §0a Review response (revision 3)

Ten findings closed cleanly. Two did not, and both are accepted: one was closed
on a **false premise**, and revision 2's own reversal **introduced a hole it
could not see**, because the plan never mentioned the client gateway once.

| # | Finding | Disposition |
|---|---|---|
| 1 | **The reversal has a hole: the client gateway.** `caller` means two different things depending on transport, so `public: false` gates nothing on the path browsers actually use, and `D-A2-12` hands the guest the *node's* DID labelled `delegated` | **Accepted — the most important finding in either review.** Verified end to end (F5a below). Three changes: `D-A2-7` now states exactly what it protects and what it does not; `D-A2-12` gains a `self-asserted` `caller-auth` variant and derives `auth` from what the connection actually presented rather than from `AuthLevel` alone; the fixture's `/whoami` asserts both transport shapes. Plus the A3 knock-on, §9.10. **Superseded in part by §0b finding 1:** revision 3 read *all* of `auth` off the preamble, which made the `ucan` label caller-controllable. The current rule is the mixed one in `D-A2-12` — read that, not this row |
| 2 | Finding 5 was closed on a false premise: `parse_interfaces` **defaults** a blank value, it does not reject one | **Accepted — revision 2 was simply wrong.** Verified [svc.rs:516-519](../../../../apps/roymctl/src/commands/svc.rs#L516): `if interfaces.trim().is_empty() { return Ok(vec![DEFAULT_INTERFACE_NAME.to_string()]) }`, and the doc comment above it records the defaulting as a deliberate fix. Only a blank *segment* is refused. §0's row and §6's justification corrected; the conclusion stands on the assertion-channel reason alone |
| 3 | F13's "No A1 code changes" is now false | **Accepted.** Verified six `HttpRoute { .. }` literals: five in A1's own [assets.rs:483, :507, :528, :551, :570](../../../../crates/control_plane/src/assets.rs#L483) and one at [native_dispatch_identity.rs:760](../../../../crates/router/tests/native_dispatch_identity.rs#L760). F13 corrected; call site 29 |
| 4 | `guest_http_permits` has no teardown | **Accepted.** Verified the established pattern: `unsubscribe_all` ([engine.rs:1263](../../../../crates/sandbox_wasm/src/engine.rs#L1263), called from [orchestration.rs:2499](../../../../crates/control_plane/src/service/orchestration.rs#L2499)) and `abort_streams` ([:1271](../../../../crates/sandbox_wasm/src/engine.rs#L1271), called from **`stop_wasm`** at [:1421](../../../../crates/sandbox_wasm/src/engine.rs#L1421)). §3.6 gains `forget_guest_http_permits`; call site 9a |
| 5 | §5.3 holds a `DashMap` guard across an `await` | **Accepted — a real deadlock-shaped bug in the pseudo-code**, and exactly the kind this plan pins elsewhere. §5.3 rewritten to clone the `Arc` out and drop the guard in its own scope, with the hazard named |
| 6 | `max_concurrent_guest_http_per_service` and `GUEST_HTTP_ADMISSION_TIMEOUT` are used but never declared | **Accepted.** Both declared in §3.6 |
| 7 | `ActiveInstanceGuard` is function-local | **Accepted.** Verified: declared inside `execute_wasm_vals` ([engine.rs:870-882](../../../../crates/sandbox_wasm/src/engine.rs#L870)). Hoisting it to module scope is now its own call site (12a) rather than an implied one |
| 8 | The config knob is on the wrong struct | **Accepted.** Verified `StreamingConfig`'s doc is "M3B Slice 6B bidirectional streaming (ADR-0014)" and it holds exactly one field ([config.rs:1166-1174](../../../../crates/core/src/config.rs#L1166)); every instance-budget knob the engine reads lives on `AppSandboxRole`. Moved, with the same `Option`-and-default-pair handling the abac knobs already get |
| 9 | The security default has no unit test | **Accepted.** §8 gains the deserialization test |
| 10 | Citation drift | **Accepted.** `AUTHORIZER_INTERFACE` is at `engine.rs:685`; the SDK's `custom_config: None` is at `sdk/src/lib.rs:725` |

**R3-A — the gateway's own "harmless today" note is now A2's problem, not
M06B's.** [gateway.rs:52-56](../../../../crates/client_gateway/src/gateway.rs#L52)
records that proxying under the node DID is "harmless today only because it
proxies to deployed services, never to `orchestrator`/`security` (flagged, not
fixed; see the deferred backlog)". That argument holds only while "deployed
service" means storage-bridging routes that ignore the caller. A2 is precisely
what makes it mean *guest logic that may branch on `caller.did`*. The
harmlessness claim does not survive this slice unchanged, so §10.8 owes that
comment and its backlog row a correction — in A2, not in M06B.

## §0b Review response (revision 4)

Both findings accepted. The first is a **security defect revision 3 introduced
while fixing one** — the same class of error, one layer along.

| # | Finding | Disposition |
|---|---|---|
| 1 | **The auth derivation swaps one lie for another.** `preamble.ucan.is_some()` means a UCAN was *attached*, not that it verified | **Accepted — attacker-controllable, and worse than what it replaced.** Verified `build_caller` fails open on a bad chain ([io.rs:241-247](../../../../crates/router/src/route_handler/io.rs#L241)): expired, revoked, wrong-audience or garbage logs a warning, leaves `auth` at `Delegated`, grants nothing — while `preamble.ucan` stays `Some`. So any caller could attach a junk string and be labelled `ucan`, the strongest value in the enum. Derivation is now **mixed**, per finding: `Ucan` from `CallerContext.auth`, `Delegated` from `preamble.delegation`, else `SelfAsserted`. `D-A2-12`, §5.2, F5b |
| 2 | **F5a's WebRTC mechanism is described wrong.** The conclusion holds; the stated reason does not | **Accepted.** Verified both sub-paths, and they fail for two *different* reasons, neither being the literal placeholder: the direct data channel sends `preamble.split('?')[0]` ([peer-proxy.js:310-312](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L310)) so there is **no `pubkey` field at all** → "Missing client public key" ([handshake.rs:47-50](../../../../crates/router/src/handshake.rs#L47)); the fallback tunnel substitutes a **real raw P-256 ECDH key** ([peer-proxy.js:369-372](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L369)), 65 bytes, which `verify_preamble` rejects decoding into a 32-byte ed25519 key ([handshake.rs:55-58](../../../../crates/router/src/handshake.rs#L55)). F5a rewritten |
| 3 | §3.7's `guest_caller_identity` signature never gained the `preamble` parameter §5.2 passes it | **Accepted.** §3.7 corrected |
| 4 | `abort_streams` is called from `stop_wasm`, not `remove_wasm` | **Accepted.** §0a corrected; the line number was right |

**R4-A — the `ucan` label still does not mean "proven caller", and the WIT must
not imply it.** Two things the fix does not buy, both now in the WIT doc rather
than left for a guest to assume. First, `AuthLevel::Ucan` is set only when the
verified chain is **non-empty** ([io.rs:232-234](../../../../crates/router/src/route_handler/io.rs#L232)),
so a chain that verifies but carries no capabilities correctly falls to
`Delegated` — the label means "verified, unrevoked, and actually bearing
capabilities", which is stronger than "a UCAN was involved" and worth saying.
Second, a UCAN presented **without** a delegation certificate is audienced to a
self-asserted pubkey nobody challenged, so the chain's signatures are sound
while possession of the audience key is not proven. That is the pre-existing
gap the SDK's own field doc names ("an assertion, not proof-of-possession") and
B1/M04B's to close — but a guest branching on `ucan` would otherwise read it as
proof. One doc sentence, no code: the same discipline `self-asserted` exists
for.

---

## §1 Findings from reading the tree

Every line reference was checked against the current working tree (`d63c9dd`,
A1 merged).

### F1 — there are exactly three targets, and the unknown-target arm is clean

`dispatch_route` ([http.rs:775](../../../../crates/router/src/route_handler/http.rs#L775))
matches `route.target.as_str()` over `"data-layer"`, `"messaging"`, `"stream"`,
with `other =>` returning a 500 naming the unknown target
([:785-789](../../../../crates/router/src/route_handler/http.rs#L785)). task.md's
"Migration impact" claim that an existing route table keeps working is
therefore correct and needs no defensive work.

### F2 — the HTTP bridge's pipeline is `NativeService`, so the guest is **not** reachable through `dispatch_json_rpc_once`

An HTTP-bridge connection carries `interface: "http-native"`
([static_assets_e2e.rs:125](../../../../crates/substrate/tests/static_assets_e2e.rs#L125)),
which every deployed service registers as
`SubstrateEndpoint::NativeHostChannel` regardless of service type
(`NATIVE_CAPABILITY_INTERFACES`,
[local_registry.rs:40](../../../../crates/core/src/local_registry.rs#L40);
registration loop at
[orchestration.rs:1960](../../../../crates/control_plane/src/service/orchestration.rs#L1960)).
`plan_pipeline` resolves `(JsonRpc, NativeHostChannel)` to
`(AdaptationStage::None, ServiceStage::NativeService)`
([dispatch.rs:321-324](../../../../crates/router/src/route_handler/dispatch.rs#L321)),
so `dispatch_json_rpc_unfenced` takes the **native** arm
([dispatch.rs:192](../../../../crates/router/src/route_handler/dispatch.rs#L192)).
Overriding `preamble.interface` (what `dispatch_native` does) changes which
*native* capability answers; it can never reach the `JsonRpcToWasm` arm.

**Consequence:** A2 must call `app_sandbox_engine` directly, exactly as
`handle_stream_route` already does
([http.rs:1001](../../../../crates/router/src/route_handler/http.rs#L1001)).
This is the single most important structural fact in this plan.

### F3 — `stream` is the working precedent for reaching a guest over the bridge

`handle_stream_route` takes `app_sandbox_engine` from `RouteHandlerInner`,
falls back to `"unknown-peer"` when the preamble carries no delegation
([http.rs:1016-1021](../../../../crates/router/src/route_handler/http.rs#L1016)),
and invokes the guest. It is the mechanical model for A2. It is **not** the
model for A2's authorization posture — see F4a and `D-A2-7`.

### F4 — the 401 on bridged routes is doctrine about *native* interfaces

`dispatch_native`'s `caller.is_none()` guard
([http.rs:181-186](../../../../crates/router/src/route_handler/http.rs#L181))
and `handle_json_rpc_bridge`'s mirror
([http.rs:473-484](../../../../crates/router/src/route_handler/http.rs#L473))
gate only the `NativeService` path. `dispatch.rs` states the rule: "native
interfaces reject anonymous callers, WASM guests admit them"
([dispatch.rs:207-209](../../../../crates/router/src/route_handler/dispatch.rs#L207),
citing design §6.1.2 and ADR-0016 §3).

### F4a — but "guests admit them" is about the *dispatch layer*, not about publication

That doctrine says a guest invocation does not *require* an identity to
proceed. It does not say every declared HTTP path must be world-reachable.
Those are different questions, and A1 already answered the second one for this
milestone: `D-A1-1` made public reach an explicit declaration with a
private default. `D-A2-7` follows A1, not F3's silence.

### F5 — a guest has no way to learn who called it

No guest-facing WIT in the tree carries a caller identity. `authorizer.wit`'s
`auth-context.subject-did` is the stage-4 ABAC after-step's own argument, not
the ordinary dispatch path, and nothing exposes `HostState.caller`. A guest
HTTP handler would therefore be identity-blind even when the connection
carried a verified delegation. `D-A2-12`.

### F5a — "the caller" means two different things, and neither is the end user

Browser traffic reaches a service by one of two transports, and they produce
opposite `caller` values. Verified end to end.

**Through the client gateway** (`localhost:7960`, the ordinary local path):
`GatewayState.identity` is the **node's own** identity, "presented as the caller
DID for every proxied request"
([gateway.rs:70-74](../../../../crates/client_gateway/src/gateway.rs#L70)).
`passthrough_with_conn` puts that key on the preamble as
`pubkey: Some(..), delegation: None`
([sdk/src/lib.rs:1049-1058](../../../../crates/sdk/src/lib.rs#L1049)), and
`build_caller` starts every verified preamble at `AuthLevel::Delegated`
([io.rs:173](../../../../crates/router/src/route_handler/io.rs#L173)). So
`self.caller` is **always `Some`** on this path:

- `D-A2-7`'s 401 never fires, so `public: false` gates nothing here.
- `caller.did` is the substrate's own key — **identical for every visitor on
  earth**, and carrying no information about who is at the keyboard.
- `auth` derived from `AuthLevel` would read `delegated` for a pubkey that was
  never challenged. The SDK's own field doc says so: "a self-asserted pubkey is
  an assertion, not proof-of-possession (the no-delegation handshake path does
  not challenge it)" ([sdk/src/lib.rs:172-180](../../../../crates/sdk/src/lib.rs#L172)).

**Direct WebRTC** (task.md exit criterion 2's configuration): `caller = None`,
so the 401 *does* fire and only `public: true` reaches the guest. **But not for
the reason the template string suggests, and the two sub-paths differ** — this
matters because `D-A2-7`'s only real protection rests on it, so anyone
maintaining that property needs the actual mechanism, not the apparent one.

The page builds
`http://<iface>|<svc>?enc=ecdh-p256&pubkey=placeholder`
([peer-proxy.js:865](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L865)),
but that literal never reaches the wire on either path:

- **Direct data channel:** the whole query string is stripped before sending —
  `preamble.split('?')[0]`, because "WebRTC data channels are already DTLS
  encrypted, no ECDH is needed"
  ([peer-proxy.js:310-312](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L310)).
  So there is **no `pubkey` field at all**, and `verify_preamble` fails at its
  first check, "Missing client public key (pubkey) in preamble"
  ([handshake.rs:47-50](../../../../crates/router/src/handshake.rs#L47)).
- **Fallback blind tunnel:** the placeholder *is* replaced, with a **real raw
  P-256 ECDH public key**
  ([peer-proxy.js:369-372](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L369)).
  Verification fails on a length mismatch, not on garbage: `verify_preamble`
  hex-decodes that field into a 32-byte ed25519 `VerifyingKey`, and a raw P-256
  point is 65 bytes → "Invalid client pubkey length"
  ([handshake.rs:55-58](../../../../crates/router/src/handshake.rs#L55)).

Either way the failure is not a hard reject — with no delegation present it
resolves to **anonymous**, `caller = None`
([io.rs:345-358](../../../../crates/router/src/route_handler/io.rs#L345)).

**The fragility this exposes is the point.** Anonymity here is a by-product of
that field being unusable, not of an intent to stay anonymous: it holds because
one path omits the field and the other fills it with a key of the wrong
*type*. "Fixing the placeholder" changes nothing on the direct path, but any
change that puts an ed25519 identity key in that field on the fallback path
flips `D-A2-7`'s protection off silently — every WebRTC caller would become
`Some`, and non-`public` routes would start admitting them. §8's e2e test pins
the behaviour; this finding is what tells whoever trips it *why* it broke.

### F5b — a rejected UCAN leaves `preamble.ucan` set

`build_caller` **fails open** on a bad authorization token: an expired,
revoked, wrong-audience or malformed chain logs a warning and leaves `auth` at
`Delegated` with no capabilities granted, deliberately — "a bad *authorization*
token does not sink an otherwise-verified *transport* identity"
([io.rs:241-247](../../../../crates/router/src/route_handler/io.rs#L241)).
`preamble.ucan` stays `Some` throughout.

So `preamble.ucan.is_some()` is caller-controlled and says nothing about
verification. `AuthLevel::Ucan` is the honest signal: it is set only inside the
verified-and-unrevoked arm, and only when the chain actually carries
capabilities ([io.rs:232-234](../../../../crates/router/src/route_handler/io.rs#L232)).

The delegation half is the opposite way round: a malformed certificate **is** a
hard reject before `build_caller` runs
([io.rs:346-355](../../../../crates/router/src/route_handler/io.rs#L346)), so
reaching here with `preamble.delegation.is_some()` does imply it verified —
which is what makes the preamble the right source for that half, and
`CallerContext.auth` the wrong one (F5a). `D-A2-12`'s derivation is therefore
**mixed**, not uniformly one or the other.

**Three consequences, all folded in below.** `public: false` protects the
direct anonymous transport and nothing else (`D-A2-7`). `caller-auth` cannot be
a projection of `AuthLevel`, whose `Delegated` covers the gateway's
unchallenged pubkey — nor, as revision 3 wrongly generalised, a projection of
the preamble alone; F5b shows why it must be **mixed** (`D-A2-12`, §5.2). And
every route A3's demo declares needs `public: true`, or exit criterion 2 fails
(§9.10).

### F6 — guest export calls are dynamic, and an *optional* export stays out of the world

`bindgen!`'s generated `host-environment` world type is **never used**: a
workspace grep for `HostEnvironment` finds only its own definition in
[crates/wit_interfaces/src/host.rs](../../../../crates/wit_interfaces/src/host.rs).
Every guest call goes through `AppSandboxEngine::get_wasm_func`'s dynamic
`Instance::get_export` + `Func::call_async(&[Val])` path
([engine.rs:620](../../../../crates/sandbox_wasm/src/engine.rs#L620)), with
`stream.rs` marshalling `Val`s by hand
([stream.rs:122-278](../../../../crates/sandbox_wasm/src/stream.rs#L122)).
A component's **exports** need no linker registration — `build_wasm_linker`
([engine.rs:572](../../../../crates/sandbox_wasm/src/engine.rs#L572)) only adds
imports.

The governing precedent for *where the WIT lives* is `AUTHORIZER_INTERFACE`
([engine.rs:685](../../../../crates/sandbox_wasm/src/engine.rs#L685)):
"Deliberately not part of the `host-environment` world -- a component only
needs to implement it when a deployed policy opts in." `incoming-handler` is
in exactly that position (optional, dynamically looked up, gated at deploy by
`D-A2-10b`), and `authorizer.wit` already sits beside its package's main file
without being in the host world.

### F7 — the epoch bound is already applied by the shared instantiation helper

`build_store_and_instantiate(service_id, caller, epoch_deadline_ticks, opts)`
([engine.rs:1010](../../../../crates/sandbox_wasm/src/engine.rs#L1010)) sets
`store.epoch_deadline_trap()` + `set_epoch_deadline(ticks)` at
[:1101-1102](../../../../crates/sandbox_wasm/src/engine.rs#L1101) and bumps A1's
instantiation counter at [:1112-1113](../../../../crates/sandbox_wasm/src/engine.rs#L1112).
Passing `self.dispatch_epoch_ticks` gives A2 task.md's "existing 5s
`dispatch_epoch_timeout_secs` bound"
([config.rs:449-451](../../../../crates/core/src/config.rs#L449), default 5)
with no new knob.

**Caveat that must be stated, not assumed away:** the epoch deadline only
interrupts guest *wasm* execution. A guest blocked inside a host call is not
interrupted by it, and nothing in this tree wraps a guest dispatch in a
wall-clock `tokio::time::timeout` — the only `time::timeout` uses in the router
are pre-auth preamble/frame reads
([io.rs:308, :533](../../../../crates/router/src/route_handler/io.rs#L308)).
A2 does not change that; §9.2, §10.5.

### F8 — nothing bounds ordinary-call instantiations, and the pool is exactly saturated

`stream_instance_permits`, `abac_instance_permits` and `probe_instance_permits`
all exist because exhausting the wasmtime pool is a hard
`PoolConcurrencyLimitError` **at instantiation, not a wait**
([engine.rs:190-215](../../../../crates/sandbox_wasm/src/engine.rs#L190)).
Ordinary calls hold no permit, and `probe_instance_permits`' own doc calls that
"out of scope here" ([engine.rs:228-231](../../../../crates/sandbox_wasm/src/engine.rs#L228)).

The arithmetic, at the default `max_concurrent_instances = 10`
([config.rs:420-422](../../../../crates/core/src/config.rs#L420)):

```
STREAM_INSTANCE_POOL_HEADROOM = 2                       (engine.rs:272)
stream_instance_budget = 10 - 2          = 8            (engine.rs:~418)
abac_instance_budget   = max(2/2, 1) = 1, ×2 slots = 2  (engine.rs:~409)
                                          8 + 2 = 10    -- exactly saturated
```

So "slots reserved for ordinary calls" is, in practice, **zero**: ordinary
calls run in whatever the streams and after-steps are not using at that
instant. That has been survivable because ordinary callers are RPC clients and
message deliveries. A2 is the first path where **one browser page issues six
or more parallel requests**, each needing its own instance. `D-A2-11`.

### F9 — the trap taxonomy is already duplicated twice, and the copies disagree

`execute_wasm_vals` classifies a call failure at
[engine.rs:915-929](../../../../crates/sandbox_wasm/src/engine.rs#L915)
(`Trap::OutOfFuel` / `"all fuel consumed"` / `"out of fuel"` → QuotaExceeded;
`"exceeded its memory limits"` / `"MemoryFault"` → MemoryFault; everything
else, **including an epoch deadline**, → the raw error). `authorize_rows`
re-implements it at
[engine.rs:1851-1868](../../../../crates/sandbox_wasm/src/engine.rs#L1851) —
its own comment claims it is "deliberately not a second, independently-drifting
implementation", which is textually false: it is a second copy, it treats
`"epoch"`/`"deadline"` as a budget overrun where `execute_wasm_vals` does not,
and **it has no memory-fault arm at all**, so a memory fault there becomes
`AbacError::Trap`. A2 would be the third copy. `D-A2-6`.

### F10 — `match_path` keeps the **last** capture, exposes no name, and has no wildcard

`match_path` ([http_routes.rs:51](../../../../crates/core/src/http_routes.rs#L51))
requires equal segment counts, overwrites `captured` on every `{...}` segment,
and returns only the value. Two consequences: A2 needs a second helper to name
the capture, and that helper must pick the **last** `{...}` segment
(`D-A2-4`); and no pattern can match a path of unknown depth, which is why A3's
SPA deep links are unreachable today (§9.8).

### F11 — nothing declares `http_routes` from the single-service CLI, and the SDK blocks it too

`roymctl svc deploy` has no `--custom-config` flag
([svc.rs:37-119](../../../../apps/roymctl/src/commands/svc.rs#L37)), and the
SDK helper it calls hardcodes `custom_config: None`
([sdk/src/lib.rs:725](../../../../crates/sdk/src/lib.rs#L725)) — so even adding
the flag alone would not reach the substrate. `http_routes` is read only out of
`ServiceConfig.custom_config`
([orchestration.rs:1580-1583](../../../../crates/control_plane/src/service/orchestration.rs#L1580)),
which today only `roymctl supervisor submit` ([mapper.rs:207](../../../../crates/sdk/src/mapper.rs#L207))
or a direct JSON-RPC deploy can populate. The Playwright harness deploys with
`svc deploy` ([global-setup.ts:164](../../../../crates/substrate/tests/e2e/global-setup.ts#L164)).
A3 and A4 are blocked on this. `D-A2-9`.

### F12 — the deploy path already has the export-existence check pattern A2 needs

`deploy_wasm_service`
([orchestration.rs:764](../../../../crates/control_plane/src/service/orchestration.rs#L764))
runs `exports_authorize_rows` / `exports_function` / `exported_functions`
against the just-compiled component and fails the deploy with a rollback of the
config generation and FDAE policy
([:779-823](../../../../crates/control_plane/src/service/orchestration.rs#L779)).
Its caller adds A1's asset-bundle rollback on top
([:1877-1884](../../../../crates/control_plane/src/service/orchestration.rs#L1877)).

### F13 — A1's asset/route collision *logic* needs no change; its test literals do

`unpack_asset_bundle` filters `declared_routes` by **method only**
([assets.rs:84-86](../../../../crates/control_plane/src/assets.rs#L84)), then
`match_path`es each pattern. A new `guest` route answering `GET` already
participates in D-A1-4's deploy-time collision refusal, so **no A1 behaviour
changes**.

Revision 2 wrote "no A1 code changes", which is false: `HttpRoute` gaining a
field breaks every struct literal, and six exist — five in A1's own collision
tests ([assets.rs:483, :507, :528, :551, :570](../../../../crates/control_plane/src/assets.rs#L483))
and one at [native_dispatch_identity.rs:760](../../../../crates/router/tests/native_dispatch_identity.rs#L760).
Mechanical (`public: false`, or `..Default::default()` if a `Default` impl is
added), and `cargo check` finds them all — but they are edits, and §4's "do not
hand-search" only works if the plan says they exist.

### F14 — dependency states

A2 adds **no workspace dependency**. The router already has `hyper`, `bytes`,
`http-body-util` (`Limited`, [http.rs:681-699](../../../../crates/router/src/route_handler/http.rs#L681));
`syneroym-sandbox-wasm` already depends on `syneroym-core` and `syneroym-rpc`.
`wasmtime::PoolConcurrencyLimitError` is public in 46.0.2 (confirmed in the
vendored source, `wasmtime-46.0.2/src/runtime.rs:112`), so `D-A2-11`'s
detection is a `downcast_ref`, not a string match.

---

## §2 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A2-1** | **A dedicated WIT export**, `syneroym:http/incoming-handler@0.1.0`'s `handle-request`, in a new standalone package at `crates/wit_interfaces/wit/http/http.wit` — **not** added to the `host-environment` world, and no `wit/host/deps` symlink. | task.md's open design point asks A2 to choose the shape. The convention route (a JSON-RPC method by name) would have to reuse `execute_wasm_json`, forcing the request through `serde_json::Value` in both directions — a 1 MiB body becomes ~1M `Value::Number`s before it becomes `Val`s — and would give the boundary no declared shape at all. On *placement*, F6: `AUTHORIZER_INTERFACE` is the exact precedent (optional, dynamic, deploy-gated) and is deliberately outside the world; `messaging/guest-api` is inside it only because the messaging world predates that reasoning. Staying out of the world also means A2 touches neither `host.wit` nor the generated bindings. |
| **D-A2-2** | **Dynamic `Val` marshalling**, in a new `crates/sandbox_wasm/src/http.rs`, reusing `stream.rs`'s `bytes_to_val_list`/`val_list_to_bytes`/`extract_result` (promoted to `pub(crate)`). No `bindgen!` typed accessor. | F6: every guest call in the tree is already dynamic and the generated world type is dead code. Introducing the first typed-export call path for this one interface is new machinery with its own failure modes, for a body capped at 1 MiB. The honest cost is recorded: a `list<u8>` materialises as `Vec<Val::U8>` at this size either way (`stream.rs` pays exactly this per 64 KiB chunk today); the typed path avoids it and is the fix if the cap rises. §9.1. |
| **D-A2-3** | **`target: "guest"`, `operation: "handle-request"`.** The matched route *pattern* travels to the guest as `route`. | `HttpRoute.operation` is non-optional, so a guest route must carry some value; making it name the export keeps the table self-describing and leaves room for a second export later. |
| **D-A2-4** | The guest gets **both** kinds of "params", named separately: `query: string` (raw, undecoded, matching `parse_query`'s existing permissive style) and `path-params: list<tuple<string, string>>`. The name comes from a new `syneroym_core::http_routes::param_name`, returning the **last** `{...}` segment to agree with `match_path`'s last-wins capture (F10). | task.md says "params" without saying which; a browser-facing route needs both. A list of tuples rather than an `option<string>` so a future multi-capture `match_path` widens without a WIT change. |
| **D-A2-5** | **The host owns HTTP framing; the guest owns semantics.** `status` and `body` are taken verbatim; framing headers (`content-length`, `transfer-encoding`, `connection`, `keep-alive`, `upgrade`, `proxy-connection`, `te`, `trailer`) are **stripped** with a `debug!`; an invalid `HeaderName`/`HeaderValue`, a status outside 200–599, or an over-cap body **rejects the whole response with a 500**. `Content-Length` is always the host's. `nosniff` is added only when the guest did not set it. | Failure-matrix row 6 says a malformed response is "bounded and **rejected**, not streamed" — a bad header fails the response rather than being silently dropped, which would change what the guest thought it sent. Framing headers are different: they are not the guest's to set, and a guest `Content-Length` disagreeing with the real body is a connection-desync bug, so those are stripped rather than treated as a fixture-level error. `HeaderValue::from_str` rejects CR/LF, which closes header injection. `nosniff` mirrors `try_handle_asset`'s own reasoning ([http.rs:584-594](../../../../crates/router/src/route_handler/http.rs#L584)). |
| **D-A2-6** | **One trap classifier**, `classify_call_failure(&anyhow::Error) -> CallFailure`, replacing F9's two copies and serving A2 as the third consumer — **with each existing call site's observable behaviour preserved exactly**, enumerated per variant per site in call sites 13 and 14. | F9: adding a third hand-rolled copy of a taxonomy whose two existing copies already disagree is how it becomes four. The review's finding 4 is why the preservation table is per-variant and not a sentence: `authorize_rows` has no memory arm today, so the "obvious" mapping of `MemoryFault` to a budget error would be a silent behaviour change. |
| **D-A2-7** | **A `guest` route is authenticated by default.** `HttpRoute` gains `#[serde(default)] pub public: bool`. `false` (the default) → an anonymous caller gets **401** before the component is instantiated, using `dispatch_native`'s existing `UNAUTHENTICATED_RPC_CODE` shape. `true` → the request reaches the guest with `caller: none`, and deploy emits an `info!` naming every such route. `public: true` on a non-`guest` target is **refused at deploy** (it would do nothing there). **What this buys, stated exactly (F5a):** it gates a **direct anonymous connection** — a WebRTC browser, or a raw QUIC client presenting no usable pubkey and no delegation. It gates **nothing** reached through the local client gateway, nor through any `SyneroymClient`: both self-assert a pubkey, so `caller` is always `Some` there and the 401 never fires. `public: false` therefore separates "no identity at all" from "some self-asserted identity". **It is not authentication**, and nothing in A2 makes it one — a self-asserted pubkey goes unchallenged until B1/M04B. | **This reverses revision 1.** F4a and R2-A: the dispatch-layer doctrine ("guests admit anonymous callers") is about whether an invocation may proceed, not about which paths are published, and `D-A1-1` already answered the publication question for this milestone with private-by-default plus a loud signal. The failure mode of forgetting the flag is a 401 an author debugs in a minute; the failure mode of a permissive default is an open write endpoint nobody notices. The narrow scope is worth keeping rather than dropping the flag: direct WebRTC is how a **remote** browser reaches a service, and is the configuration task.md's exit criterion 2 names — the default does real work exactly where untrusted traffic arrives. Overclaiming it would be worse than not having it, which is why the scope is in the decision and not a footnote. `public: bool` rather than reusing ADR-0018's `visibility` enum: this is neither endpoint-record publication nor byte readability, `syneroym-core` cannot see `app_orchestration::Visibility` (the same constraint that produced `ServiceAssets.public`, backlog row 128), and A2 has no middle tier to express. |
| **D-A2-8** | **Two body caps, both 1 MiB, both their own constants:** `MAX_GUEST_REQUEST_BODY_BYTES` (enforced with `Limited` **before** instantiation → 413) and `MAX_GUEST_RESPONSE_BODY_BYTES` (after return → 500). Plus `MAX_GUEST_REQUEST_HEADERS` / `MAX_GUEST_RESPONSE_HEADERS`, both 64 (request → 431, response → 500). | Deliberately not reusing `MAX_SMALL_BODY_BYTES` ([http.rs:87](../../../../crates/router/src/route_handler/http.rs#L87)) even though the number matches: that constant documents itself as the guard for `data-layer`/`messaging` routes, and the guest path has a different cost curve (D-A2-2's `Val` expansion). **Honest about what the response cap is not:** the guest's `list<u8>` is fully materialised in host memory before it can be measured, so this bounds what is *sent*, not what is *allocated* — the allocation bound is the guest's own `max_memory_bytes` store limiter. §10.4. |
| **D-A2-9** | **`roymctl svc deploy` gains `--custom-config <path.json>`, and the SDK helper is reshaped to carry it**: `deploy_svc_wasm_with_assets` is **replaced** by `deploy_svc_wasm_with_options(service_id, interfaces, wasm_bytes, DeploySvcOptions { .. })`. | F11: the flag alone cannot reach the substrate. A1 added a second `deploy_svc_wasm_*` method for the same reason; a third would make the next optional field a fourth. The replaced method has exactly **one** call site ([svc.rs:324](../../../../apps/roymctl/src/commands/svc.rs#L324)), and `deploy_svc_wasm` (seven call sites) keeps delegating unchanged, so the churn is one line. |
| **D-A2-10** | **Two deploy-time refusals:** (a) a `guest` route declared by a non-`Wasm` service, refused beside A1's identical `assets` check ([orchestration.rs:1653](../../../../crates/control_plane/src/service/orchestration.rs#L1653)); (b) a `guest` route whose compiled component does not export `handle-request`, refused inside `deploy_wasm_service` beside the stage-4 check. | (a) A `Tcp`/`Container` endpoint is `TcpHostPort`, routed to raw `copy_bidirectional` passthrough — the HTTP bridge is structurally unreachable, so the route would be silent dead configuration, exactly the bug A1's post-review check closed. (b) F12: the alternative is a route that 500s on every request, discoverable only in production. Must run *after* `deploy_wasm` compiles, which is where `exports_*` has a real answer. |
| **D-A2-11** | **Guest HTTP concurrency is bounded per service** by a new per-service semaphore, sized by a new **`AppSandboxRole`** knob `max_concurrent_guest_http_per_service` (default 4). Acquire **before** instantiating. A permit wait past `GUEST_HTTP_ADMISSION_TIMEOUT` (2s), or a `PoolConcurrencyLimitError` from `instantiate_async`, becomes **503 + `Retry-After: 1`**, never 500. **The global pool accounting is deliberately not re-tuned.** | F8: a browser page is six-plus parallel instantiations against a pool whose ordinary-call headroom is arithmetically zero, and pool exhaustion is a hard error. Matrix row 8's principle ("exhausting them degrades that service, not the node") is written about SSE but is the same property. Cap *shape* copied from `max_concurrent_streams_per_service` ([config.rs:1166-1174](../../../../crates/core/src/config.rs#L1166)), but **not its home**: `StreamingConfig` is documented as "M3B Slice 6B bidirectional streaming (ADR-0014)" and holds that one field, while every instance-budget knob this engine actually reads (`max_concurrent_instances`, `dispatch_epoch_timeout_secs`, `abac_epoch_timeout_secs`, `abac_max_instructions`) lives on `AppSandboxRole`. `AppSandboxRole` is an `Option` on the config, which `init` already handles with a default pair for the abac knobs ([engine.rs:386-390](../../../../crates/sandbox_wasm/src/engine.rs#L386)) — follow that, do not add a second pattern. **Not** re-deriving `STREAM_INSTANCE_POOL_HEADROOM`: that constant's arithmetic is asserted exactly by an existing test, it governs every call path rather than this one, and re-tuning it blind without a measurement is precisely what `D-A1-6` refused to do for the blob path. A3's Playwright run under a real browser is the measurement; backlog row owed either way. §9.6. |
| **D-A2-12** | **`http-request` carries `caller: option<caller-identity>`** — `did`, `auth`, `app-instance` — built from the router's `Option<CallerContext>` *before* the `service_system` fallback, so `none` means genuinely anonymous. **`auth` is derived per-variant from whichever source is honest for it (F5a, F5b), never uniformly from one:** `ucan` from **`CallerContext.auth == AuthLevel::Ucan`**, `delegated` from **`preamble.delegation.is_some()`**, else `self-asserted` — a third `caller-auth` variant added for exactly this. A substrate-injected `AuthLevel` (`LocalElevated`/`LocalReadOnly`/`System`) reaching this path is a host bug: **fail closed with a 500**, never a lossy map. | F5: without this the guest is identity-blind, so a `public: true` route cannot tell a signed-in caller from a stranger, and `D-A2-7`'s "the guest decides the rest" is not reachable. Adding it after A3 ships would be a breaking WIT record change for every guest. **The split is not fussiness — each source is unreliable for the other half.** `AuthLevel::Delegated` is assigned to *every* verified preamble ([io.rs:173](../../../../crates/router/src/route_handler/io.rs#L173)), including the client gateway's unchallenged node-DID pubkey, so it cannot carry `delegated`; the preamble can, because a malformed certificate is a hard reject before this point. Conversely `preamble.ucan.is_some()` means only that a token was *attached* — `build_caller` fails open on a bad chain (F5b) — so it cannot carry `ucan`; `AuthLevel::Ucan` can, being set only on a verified, unrevoked, capability-bearing chain. Revision 3 used the preamble for both and made the stronger label caller-controllable: **any client could have attached a junk UCAN string and been reported as `ucan`.** Fail-closed on the injected levels for the same reason throughout: a guest may branch on this field, so every value it can hold must be one the host can actually stand behind. |

---

## §3 Exact type and signature changes

### 3.1 New WIT — `crates/wit_interfaces/wit/http/http.wit`

Both records live **inside** `incoming-handler`: the host looks up one
interface name, a guest exports one thing. (`messaging` splits `stream-types`
out only because guest-implemented *resources* require it — ADR-0014; there are
no resources here.) Standalone package, **not** referenced from
`wit/host/host.wit` — `D-A2-1`, F6.

```wit
package syneroym:http@0.1.0;

/// Inbound HTTP handed to a component as an ordinary request/response
/// (M06A A2). Deliberately **not** `wasi:http`: a component gets a request
/// handed to a function, never a socket. Static assets never reach here --
/// they are served straight from blob storage without instantiating the
/// component (M06A A1).
///
/// Optional, like `syneroym:data-layer/authorizer`: a component only
/// exports this when it declares an `http_routes` entry with
/// `target = "guest"`, and the deploy refuses that combination when the
/// export is missing.
interface incoming-handler {
    /// How the caller's identity was established -- read off what the
    /// connection actually presented, not off any host-internal level.
    ///
    /// The substrate-injected levels (lifecycle, stage-4 ABAC,
    /// service-system) are deliberately absent: none can reach an inbound
    /// HTTP request, and one arriving is a host bug the router fails
    /// closed on.
    enum caller-auth {
        /// A verified delegation certificate. A malformed one is refused at
        /// the handshake, so reaching a guest at all means it verified.
        delegated,
        /// A UCAN capability chain that verified, was not revoked, and
        /// carried at least one capability -- a chain that fails any of
        /// those reports `delegated` or `self-asserted` instead, never
        /// this.
        ///
        /// **Still not proof that the caller is who the DID says**, when
        /// no delegation accompanies it: such a chain is audienced to a
        /// public key nobody challenged, so its signatures are sound while
        /// possession of that key is unproven. The capabilities are
        /// trustworthy; the identity holding them is only as trustworthy
        /// as `self-asserted` below.
        ucan,
        /// A bare public key with no delegation and no UCAN: an assertion
        /// the handshake never challenged. **Treat as pseudonymous, not
        /// authenticated.** This is what every request proxied by the
        /// local client gateway looks like, and there `did` is the
        /// *node's* own key -- the same value for every visitor, saying
        /// nothing about who is at the keyboard.
        self-asserted,
    }

    /// Who is asking, as far as the host can honestly say. Absent means
    /// genuinely anonymous -- nothing usable on the connection at all --
    /// which only a route declared `public` can reach.
    record caller-identity {
        /// did:key of the immediate caller. Trustworthy as an identity
        /// only when `auth` is `delegated` or `ucan`.
        did: string,
        auth: caller-auth,
        /// The app instance the caller acts as, when it acts as one.
        app-instance: option<string>,
    }

    /// One inbound request, already matched against the service's declared
    /// `http_routes` table by the router.
    record http-request {
        /// Uppercase HTTP method, verbatim (`GET`, `POST`, ...).
        method: string,
        /// Request path only, no query string. Percent-encoded exactly as
        /// the client sent it -- the host does not decode it, so a guest
        /// comparing against its own literals sees what was on the wire.
        path: string,
        /// Raw query string with no leading `?`, empty when absent.
        /// Neither decoded nor split.
        query: string,
        /// The declared route *pattern* that matched (e.g.
        /// `/api/comments/{id}`), so a guest can switch on the route it
        /// declared rather than re-implementing path matching.
        route: string,
        /// Captures from `route`'s `{name}` segments, name -> value. At
        /// most one entry today (the router matches a single capture).
        path-params: list<tuple<string, string>>,
        /// Request headers, names lowercased. Hop-by-hop and framing
        /// headers are removed by the host; a header whose value is not
        /// UTF-8 is dropped rather than failing the request.
        headers: list<tuple<string, string>>,
        /// Request body, empty when absent. Capped by the host before this
        /// function is reached; an over-cap request never instantiates the
        /// component.
        body: list<u8>,
        /// Who is asking. `none` on a route declared `public` reached with
        /// nothing usable on the connection. Check `auth` before trusting
        /// `did`: `self-asserted` is unchallenged, and on gateway-proxied
        /// traffic it is the node's own key, not the end user's.
        caller: option<caller-identity>,
    }

    /// The guest's answer. `status` and `body` are taken verbatim; the host
    /// owns framing (`content-length`, `transfer-encoding` and the other
    /// hop-by-hop headers are stripped if present).
    record http-response {
        /// 200-599. Anything else is a malformed response and becomes a
        /// 500 to the client.
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }

    /// Invoked once per matched request. `Ok` is the response to send --
    /// including a deliberate rejection, which is an ordinary `Ok` with a
    /// 4xx `status` and the guest's own message, not an `Err`. `Err` means
    /// the handler itself failed, and becomes a 500 carrying the message.
    handle-request: func(request: http-request) -> result<http-response, string>;
}

world http-guest {
    export incoming-handler;
}
```

Interface name for host lookup: **`syneroym:http/incoming-handler@0.1.0`**
(package-qualified — the short name does not resolve; see
[stream.rs:31-35](../../../../crates/sandbox_wasm/src/stream.rs#L31)).

### 3.2 New module — `crates/core/src/guest_http.rs`

Host-side mirror of the records, in `core` for the same reason
`StreamDirection` is (`syneroym-router` builds them, `syneroym-sandbox-wasm`
consumes them, and neither depends on the other).

```rust
/// How an inbound caller's identity was established, as a guest sees it.
/// Mirrors the WIT `caller-auth`. Deliberately **not** a projection of
/// `syneroym_rpc::AuthLevel`: that enum's `Delegated` covers an
/// unchallenged self-asserted pubkey too, which is what the client gateway
/// sends (M06A F5a, D-A2-12). Derived from the preamble instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestCallerAuth { Delegated, Ucan, SelfAsserted }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCallerIdentity {
    pub did: String,
    pub auth: GuestCallerAuth,
    pub app_instance: Option<String>,
}

/// One inbound HTTP request on its way to a guest's
/// `syneroym:http/incoming-handler#handle-request` (M06A A2). Mirrors the
/// WIT `http-request` field for field, **in the same order** -- the dynamic
/// `Val::Record` built from it must match the declared field order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestHttpRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub route: String,
    pub path_params: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub caller: Option<GuestCallerIdentity>,
}

/// A guest's answer, exactly as returned. Nothing here is validated yet --
/// status range, header well-formedness and body size are the router's
/// checks (M06A D-A2-5), since they are HTTP questions and neither `core`
/// nor `sandbox_wasm` holds `hyper` types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
```

`crates/core/src/lib.rs`: `pub mod guest_http;`

### 3.3 `syneroym_core::http_routes` — one field, one function

On `HttpRoute`, after `protocol`:

```rust
    /// Whether a caller with no verified identity may reach this route
    /// (M06A D-A2-7). Only meaningful for `target = "guest"`, where `false`
    /// -- the default -- answers an anonymous request with 401 before the
    /// component is instantiated. Refused at deploy on any other target,
    /// where it would do nothing: `data-layer`/`messaging` already reject
    /// an anonymous caller inside `dispatch_native`, and `stream` predates
    /// this field (M06A §9.5).
    ///
    /// A bool, not ADR-0018's `visibility` enum: this is neither
    /// endpoint-record publication nor byte readability, `syneroym-core`
    /// cannot see `syneroym_app_orchestration::Visibility`, and there is no
    /// middle tier to express.
    #[serde(default)]
    pub public: bool,
```

```rust
/// The name of the single capturing segment in `pattern` (`/orders/{id}` ->
/// `Some("id")`), or `None` when the pattern has no `{...}` segment.
///
/// Returns the **last** such segment, matching `match_path`'s own last-wins
/// capture: with two `{...}` segments the two functions must describe the
/// same segment, or a guest would receive a name and a value from different
/// parts of the path. Only a single capture is supported anyway.
#[must_use]
pub fn param_name(pattern: &str) -> Option<&str>;
```

### 3.4 `crates/sandbox_wasm/src/stream.rs` — visibility only

`bytes_to_val_list` ([:122](../../../../crates/sandbox_wasm/src/stream.rs#L122)),
`val_list_to_bytes` ([:127](../../../../crates/sandbox_wasm/src/stream.rs#L127))
and `extract_result` ([:148](../../../../crates/sandbox_wasm/src/stream.rs#L148))
become `pub(crate)`. No body changes.

### 3.5 New module — `crates/sandbox_wasm/src/http.rs`

```rust
/// WIT-package-qualified name of the guest HTTP handler interface (M06A
/// A2) -- the short name alone does not resolve, same as
/// `STREAM_TYPES_INTERFACE` and `AUTHORIZER_INTERFACE`.
pub(crate) const HTTP_HANDLER_INTERFACE: &str = "syneroym:http/incoming-handler@0.1.0";

/// Builds the `Val::Record` argument for `handle-request`. Field order is
/// the WIT declaration order and must stay in sync with
/// `wit/http/http.wit`.
pub(crate) fn request_to_val(request: &GuestHttpRequest) -> Val;

/// Decodes `handle-request`'s `result<http-response, string>` return.
pub(crate) fn response_from_results(
    results: &[Val],
) -> result::Result<GuestHttpResponse, GuestHttpFailure>;
```

### 3.6 `crates/sandbox_wasm/src/engine.rs`

```rust
/// How a `handle-request` call ended (M06A A2). `Err` from the enclosing
/// `Result` is reserved for host-side failure; everything a *guest* can do
/// lands in here, mirroring `StreamRequestOutcome`'s split.
#[derive(Debug)]
pub enum GuestHttpOutcome {
    Response(GuestHttpResponse),
    Failed(GuestHttpFailure),
}

/// Why a guest HTTP call produced no usable response. Every variant maps to
/// 500 **except `Unavailable`, which maps to 503** -- resource exhaustion is
/// "try again", not "the guest broke".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestHttpFailure {
    /// The component does not export `handle-request`. Unreachable through
    /// a normal deploy (D-A2-10b refuses it); kept because the engine
    /// cannot assume its caller checked.
    NoHandler,
    /// The guest returned `Err(msg)` -- the handler failed. A guest
    /// *rejecting* a request returns `Ok` with a 4xx status instead.
    Declined(String),
    /// Fuel exhausted or the epoch deadline reached.
    BudgetExceeded(String),
    Trap(String),
    /// The return value was not `result<http-response, string>`.
    Malformed(String),
    /// No instance could be obtained: the per-service admission permit
    /// timed out, or wasmtime's pool refused
    /// (`PoolConcurrencyLimitError`). M06A D-A2-11.
    Unavailable(String),
}

impl AppSandboxEngine {
    /// Whether `service_id`'s compiled component exports the guest HTTP
    /// handler. Cheap (static component type, no instantiation) -- exactly
    /// `exports_authorize_rows`' shape and deploy-gate role.
    #[must_use]
    pub fn exports_http_handler(&self, service_id: &str) -> bool;

    /// Runs one inbound HTTP request through the guest's `handle-request`
    /// export on a fresh per-call instance, bounded by
    /// `dispatch_epoch_ticks` (task.md's existing 5s
    /// `dispatch_epoch_timeout_secs`), the service's fuel/memory quota, and
    /// this service's own guest-HTTP admission permit (D-A2-11).
    ///
    /// `caller` is forwarded into `HostState.caller` exactly as
    /// `execute_wasm_json` does. `None` reaches here only for a route the
    /// deploy declared `public` -- the router answers 401 otherwise, before
    /// this function is called (D-A2-7).
    pub async fn handle_guest_http_request(
        &self,
        service_id: &str,
        request: &GuestHttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<GuestHttpOutcome>;
}

/// How a `Func::call_async` failure should be read. One definition, three
/// consumers -- see F9 for the two divergent copies it replaces.
pub(crate) enum CallFailure { OutOfFuel, MemoryFault, Deadline, Other }

pub(crate) fn classify_call_failure(e: &anyhow::Error) -> CallFailure;
```

Two new fields beside the other permit semaphores, one constant, and one
teardown method — **all three of the first were used but undeclared in
revision 2**:

```rust
    /// Pool slots this node will let *guest HTTP* requests hold
    /// concurrently, per service (M06A D-A2-11). Unlike an RPC client, one
    /// browser page issues six or more parallel requests, and exhausting
    /// wasmtime's pool is a hard `PoolConcurrencyLimitError` at
    /// instantiation rather than a wait -- so without this, a single page
    /// load turns into 500s and can also drain the headroom
    /// `stream_instance_permits` reserves for ordinary calls. Bounded
    /// queuing instead, with a 503 past the wait.
    ///
    /// Entries are removed by `forget_guest_http_permits` on undeploy,
    /// matching `unsubscribe_all`/`abort_streams` -- every other
    /// per-service map here has an explicit teardown, and a map that only
    /// ever grows is a leak however small.
    guest_http_permits: Arc<DashMap<String, Arc<Semaphore>>>,
    /// Snapshot of `AppSandboxRole::max_concurrent_guest_http_per_service`,
    /// the size each per-service semaphore above is created at.
    max_concurrent_guest_http_per_service: u32,
```

```rust
/// How long a guest HTTP request waits for its service's admission permit
/// before the router answers 503 (M06A D-A2-11). Short on purpose: a
/// browser that waited longer than this has already given the user a
/// stalled page, and a fast, honest "busy, retry" beats a slow success.
const GUEST_HTTP_ADMISSION_TIMEOUT: Duration = Duration::from_secs(2);
```

```rust
    /// Drops `service_id`'s guest HTTP admission semaphore (M06A A2).
    /// Called from the same undeploy path as `unsubscribe_all`; in-flight
    /// requests keep their own `OwnedSemaphorePermit` and finish, they just
    /// stop sharing a budget with a service that no longer exists.
    pub fn forget_guest_http_permits(&self, service_id: &str);
```

```rust
// crates/core/src/config.rs, on `AppSandboxRole` -- **not** `StreamingConfig`
// (D-A2-11): that struct is ADR-0014 streaming, while every instance-budget
// knob the engine reads already lives here.
    /// Concurrent guest HTTP requests one service may have in flight
    /// (M06A A2).
    #[serde(default = "default_max_concurrent_guest_http_per_service")]
    pub max_concurrent_guest_http_per_service: u32,   // default 4
```

`crates/sandbox_wasm/src/lib.rs` adds `mod http;` and re-exports
`GuestHttpOutcome`, `GuestHttpFailure`.

### 3.7 `crates/router/src/route_handler/http.rs`

```rust
/// Request-body ceiling for a `guest` route (M06A D-A2-8). Its own constant
/// rather than `MAX_SMALL_BODY_BYTES`: this body is additionally marshalled
/// into a `Vec<Val::U8>` for the component-model call, so the two limits
/// have different cost curves and may diverge.
const MAX_GUEST_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Response-body ceiling for a `guest` route. Bounds what is **sent**, not
/// what is allocated: the guest's `list<u8>` is fully materialised in host
/// memory before it can be measured, and the allocation bound is the
/// guest's own `max_memory_bytes` store limiter.
const MAX_GUEST_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

const MAX_GUEST_REQUEST_HEADERS: usize = 64;
const MAX_GUEST_RESPONSE_HEADERS: usize = 64;

/// Headers the host owns, never the guest: stripped from a guest response
/// and never forwarded from a request (M06A D-A2-5).
const HOST_OWNED_HEADERS: [&str; 8] = [
    "content-length", "transfer-encoding", "connection", "keep-alive",
    "upgrade", "proxy-connection", "te", "trailer",
];

/// Request headers a guest sees. A free function so the filtering rule is
/// unit-testable without a live `HttpHandler`, same as
/// `blob_hash_from_path`/`if_none_match_hits`.
fn guest_request_headers(
    headers: &HeaderMap,
) -> result::Result<Vec<(String, String)>, Response<HttpBody>>;

/// The router's view of `CallerContext` as a guest may see it (M06A
/// D-A2-12). `Err` on a substrate-injected `AuthLevel`, which cannot
/// legitimately reach an inbound request.
///
/// Takes `preamble` as well as `caller` because the two `auth` halves read
/// different sources: `CallerContext.auth` cannot distinguish a verified
/// certificate from an unchallenged pubkey (F5a), while the preamble can;
/// and `preamble.ucan` says only that a token was attached, while
/// `CallerContext.auth` says it verified (F5b).
fn guest_caller_identity(
    caller: Option<&CallerContext>,
    preamble: &RoutePreamble,
) -> result::Result<Option<GuestCallerIdentity>, String>;

/// Turns a guest's answer into an HTTP response, or into the 500
/// failure-matrix row 6 requires. Free function, same reason.
fn build_guest_response(response: GuestHttpResponse) -> Response<HttpBody>;

impl HttpHandler {
    /// The fourth `dispatch_route` target (M06A A2).
    async fn handle_guest_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>>;
}
```

### 3.8 `crates/control_plane/src/http_routes.rs`

`validate_route`'s match gains two arms **before** the `_ => Ok(())`
catch-all, plus one check outside it:

```rust
("guest", "handle-request") => Ok(()),
("guest", other) => Err(format!(
    "http_routes entry `{} {}` has target=guest with unsupported operation `{other}`; \
     the only guest operation is `handle-request`",
    route.method, route.path
)),
```

```rust
// D-A2-7: `public` does nothing outside a guest route, so accepting it
// there would be exactly the silently-dead configuration this module's
// duplicate-route check already exists to prevent.
if route.public && route.target != "guest" {
    return Err(format!(
        "http_routes entry `{} {}` sets `public` on target={}; `public` is only \
         meaningful for target=guest",
        route.method, route.path, route.target
    ));
}
```

### 3.9 `crates/control_plane/src/service/orchestration.rs`

```rust
    /// `http_routes` is the parsed table (M06A D-A2-10b): a declared
    /// `guest` target needs the compiled component to actually export the
    /// handler, which is only checkable after `deploy_wasm` below has
    /// compiled it.
    async fn deploy_wasm_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        wasm_manifest: &WasmManifest,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        new_fdae_policy: Option<&Policy>,
        http_routes: &[HttpRoute],
    ) -> Result<(), String>;
```

### 3.10 `crates/sdk/src/lib.rs`

```rust
/// Everything optional about a `deploy_svc_wasm_with_options` call (M06A
/// A2). Replaces the growing positional tail
/// `deploy_svc_wasm_with_assets` had started: `assets` was A1's addition,
/// `custom_config` is A2's, and a third would have meant a third method.
#[derive(Debug, Default)]
pub struct DeploySvcOptions {
    pub registry_certificate: Option<SignedEndpointInfo>,
    pub instance_certificate: Option<DelegationCertificate>,
    pub assets: Option<AssetBundle>,
    /// Verbatim `ServiceConfig.custom_config`. The reserved `http_routes`
    /// key inside it is what declares HTTP routes.
    pub custom_config: Option<String>,
}

pub async fn deploy_svc_wasm_with_options(
    &self,
    service_id: String,
    interfaces: Vec<String>,
    wasm_bytes: Vec<u8>,
    options: DeploySvcOptions,
) -> Result<()>;
```

`deploy_svc_wasm` keeps its signature and delegates with
`DeploySvcOptions { registry_certificate, instance_certificate, ..Default::default() }`.

### 3.11 `apps/roymctl/src/commands/svc.rs`

```rust
        /// Path to a JSON file used verbatim as the service's
        /// `custom_config` -- the reserved `http_routes` key inside it is
        /// what declares HTTP routes (M3B Slice 7, M06A A2).
        #[arg(long)]
        custom_config: Option<PathBuf>,
```

---

## §4 Call sites

`cargo check --workspace` after items 1–5 enumerates the rest. Do not
hand-search.

| # | File | Change |
|---|---|---|
| 1 | `crates/wit_interfaces/wit/http/http.wit` | New package (§3.1). **No** `host.wit` edit, **no** `wit/host/deps` symlink |
| 2 | `crates/core/src/guest_http.rs` + `crates/core/src/lib.rs` | New module, `pub mod guest_http;` |
| 3 | `crates/core/src/http_routes.rs` | `HttpRoute.public`, `param_name`, and their unit tests |
| 4 | `crates/core/src/config.rs` | `max_concurrent_guest_http_per_service` on **`AppSandboxRole`** + its default fn + `Default` impl + the existing config-defaults test |
| 5 | `crates/sandbox_wasm/src/stream.rs:122, :127, :148` | Three fns → `pub(crate)` |
| 6 | `crates/sandbox_wasm/src/http.rs` | New module (§3.5) |
| 7 | `crates/sandbox_wasm/src/lib.rs` | `mod http;`, re-export `GuestHttpOutcome`/`GuestHttpFailure` |
| 8 | `crates/sandbox_wasm/src/engine.rs` | `exports_http_handler`, `handle_guest_http_request`, `guest_http_permits`, `max_concurrent_guest_http_per_service`, `GUEST_HTTP_ADMISSION_TIMEOUT`, `forget_guest_http_permits`, `CallFailure`/`classify_call_failure` |
| 9 | `crates/sandbox_wasm/src/engine.rs` `init` (~`:315-450`) | Build `guest_http_permits`; read the new `AppSandboxRole` field through the same `if let Some(sandbox_config)`/default pair the abac knobs use at `:386-390`. **Do not touch** `stream_instance_budget`/`abac_instance_budget` (D-A2-11) |
| 9a | `crates/control_plane/src/service/orchestration.rs:2499` | Call `forget_guest_http_permits` beside the existing `unsubscribe_all`, so the per-service map is torn down like every sibling (review-3 finding 4) |
| 10 | `crates/router/src/route_handler/http.rs:781-789` | Fourth arm: `"guest" => self.handle_guest_route(route, path_param, req).await` |
| 11 | `crates/router/src/route_handler/http.rs` | `handle_guest_route`, `guest_request_headers`, `guest_caller_identity`, `build_guest_response`, the four caps and `HOST_OWNED_HEADERS` (§3.7) |
| 12 | `crates/sandbox_wasm/src/engine.rs`, inside `handle_guest_http_request` | **Metrics parity**: an `ActiveInstanceGuard` (`substrate.wasm.active_instances`) and a `substrate.wasm.execution_ms` histogram around the call. Every other guest-invoking path records both; this one bypasses `execute_wasm_vals`, so it must record them itself |
| 12a | `crates/sandbox_wasm/src/engine.rs:870-882` | **Hoist `ActiveInstanceGuard` to module scope.** It is declared *inside* `execute_wasm_vals` today, so call site 12 cannot reuse it without this first. Move the struct and its `new`/`Drop` out unchanged; `execute_wasm_vals`' body keeps `let _guard = ActiveInstanceGuard::new();` verbatim |
| 13 | `crates/sandbox_wasm/src/engine.rs:915-929` | `execute_wasm_vals` uses `classify_call_failure`. **Pinned:** `OutOfFuel` → `"QuotaExceeded: ..."`, `MemoryFault` → `"MemoryFault: ..."`, **`Deadline` → `Err(e.into())`, `Other` → `Err(e.into())`** (the two are indistinguishable today and must stay so) |
| 14 | `crates/sandbox_wasm/src/engine.rs:1851-1868` | `authorize_rows` uses `classify_call_failure`. **Pinned:** `OutOfFuel` → `BudgetExceeded`, `Deadline` → `BudgetExceeded`, **`MemoryFault` → `AbacError::Trap`** (this site has no memory arm today — review finding 4), `Other` → `Trap` |
| 15 | `crates/router/src/route_handler/http.rs:1-21` | Module doc's numbered resolution order: step 3 gains the guest target |
| 16 | `crates/core/src/http_routes.rs:1-20` | Module doc ("bridges onto `data-layer`/`messaging`/a registered stream protocol") **and** the `HttpRoute` doc comment listing which optional field belongs to which target |
| 17 | `crates/control_plane/src/http_routes.rs:1-16` | Module doc, same enumeration |
| 18 | `crates/control_plane/src/http_routes.rs` | `validate_route`'s new arms and the `public` check (§3.8) |
| 19 | `crates/control_plane/src/service/orchestration.rs:764` | `deploy_wasm_service` gains `http_routes`; export check after the stage-4 check at `:799`, same rollback pair |
| 20 | `crates/control_plane/src/service/orchestration.rs:1863` | Pass `&http_routes` at the one call site |
| 21 | `crates/control_plane/src/service/orchestration.rs:1659` | D-A2-10a: refuse a `guest` route on a non-`Wasm` service, immediately after A1's identical `assets` check |
| 22 | `crates/control_plane/src/service/orchestration.rs`, near `:1846` | D-A2-7's `info!` for each `public` guest route, beside A1's asset-bundle `info!` |
| 23 | `crates/sdk/src/lib.rs:681, :704-747` (the `custom_config: None` is `:725`) | `DeploySvcOptions` + `deploy_svc_wasm_with_options`, **replacing** `deploy_svc_wasm_with_assets`; `deploy_svc_wasm` delegates unchanged |
| 24 | `apps/roymctl/src/commands/svc.rs:324` | The one `deploy_svc_wasm_with_assets` call site → the new form; add `--custom-config` and read the file |
| 25 | `crates/core/src/test_constants.rs` | `http_guest_test_wasm_path()` + `HTTP_GUEST_TEST_DRIVER_INTERFACE` |
| 26 | `test-components/http-guest-test/` | New fixture (§6) |
| 27 | `test-components/README.md` | One line for the new fixture |
| 28 | `docs/system-architecture.md:1929` | §10.7's correction to the "HTTP Passthrough" bullet |
| 29 | `crates/control_plane/src/assets.rs:483, :507, :528, :551, :570`; `crates/router/tests/native_dispatch_identity.rs:760` | The six `HttpRoute { .. }` literals the new `public` field breaks (F13). Mechanical, but they exist |
| 30 | `crates/client_gateway/src/gateway.rs:52-56` | §10.8's correction to the "harmless today only because it proxies to deployed services" note, and its backlog row |

---

## §5 Pseudo-code

### 5.1 Router — `handle_guest_route`

```
handle_guest_route(route, path_param, req) -> Result<Response>:
    if route.operation != "handle-request":
        return 500 "unsupported guest operation: {route.operation}"

    # D-A2-7, BEFORE any engine work: an anonymous caller on a non-public
    # route never instantiates anything. Same code and status shape
    # `dispatch_native` uses, so one 401 taxonomy covers the whole bridge.
    if self.caller.is_none() && !route.public:
        return structured_rpc_error(401, UNAUTHENTICATED_RPC_CODE,
            "unauthenticated caller for guest route {route.method} {route.path}")

    caller_identity = guest_caller_identity(self.caller.as_ref(), &self.preamble):
        Err(reason) -> return 500 "unexpected caller context: {reason}"   # D-A2-12 fail-closed

    engine = inner.app_sandbox_engine else
        return 503 "app sandbox engine not available (coordinator mode)"
    if !engine.is_deployed(preamble.service_id):
        return 503 "service has no deployed WASM component"

    (parts, body) = req.into_parts()
    headers = guest_request_headers(&parts.headers)?          # Err -> 431
    body = Limited::new(body, MAX_GUEST_REQUEST_BODY_BYTES).collect():
        LengthLimitError -> return 413 "request body exceeds N byte limit"
        other error      -> return 400 "failed to read request body: {e}"
    # Every rejection above happens BEFORE any engine call, so each costs
    # zero instantiations (asserted in §7).

    request = GuestHttpRequest {
        method:      parts.method.as_str().to_string(),
        path:        parts.uri.path().to_string(),
        query:       parts.uri.query().unwrap_or("").to_string(),
        route:       route.path.clone(),
        path_params: match (core::http_routes::param_name(&route.path), path_param) {
                         (Some(name), Some(value)) => vec![(name.into(), value)],
                         _                         => vec![],
                     },
        headers, body: body.to_vec(), caller: caller_identity,
    }

    match engine.handle_guest_http_request(&preamble.service_id, &request, self.caller.clone()).await:
        Ok(Response(r))            -> Ok(build_guest_response(r))
        Ok(Failed(Unavailable(d))) -> Ok(retry_after(503, "service is at its guest HTTP concurrency limit"))
        Ok(Failed(failure))        -> error!(service_id, route = route.path, ?failure,
                                             "guest HTTP handler failed");
                                      Ok(http_error(500, describe(failure)))
        Err(e)                     -> Ok(http_error(500, e.to_string()))
```

`retry_after` is `http_error(503, ..)` plus `Retry-After: 1`. `describe` is a
short sentence per variant; the guest's own string is already truncated by
`truncate_detail` inside the engine, so it is safe to include.

### 5.2 Router — the three free functions

```
guest_request_headers(headers) -> Result<Vec<(String,String)>, Response>:
    out = []
    for (name, value) in headers.iter():
        lower = name.as_str().to_ascii_lowercase()
        if HOST_OWNED_HEADERS.contains(lower): continue      # host owns framing
        let Ok(text) = value.to_str() else: continue         # non-UTF-8: dropped, not fatal
        if out.len() == MAX_GUEST_REQUEST_HEADERS:
            return Err(431 "request has more than N headers")
        out.push((lower, text.to_string()))
    Ok(out)

guest_caller_identity(caller, preamble) -> Result<Option<GuestCallerIdentity>, String>:
    let Some(c) = caller else: return Ok(None)               # genuinely anonymous

    # Invariant check first. D-A2-12: the router only ever holds a
    # wire-verified caller or None, so these mean something broke upstream.
    # Fail closed rather than reporting a level that isn't true.
    if matches!(c.auth, LocalElevated | LocalReadOnly | System):
        return Err("substrate-injected auth level on an inbound HTTP request")

    # MIXED on purpose -- each source is unreliable for the other half.
    # Do not collapse this to one source; revision 3 did, and made the
    # strongest label caller-controllable.
    #
    #   `ucan`      <- c.auth, NOT preamble.ucan. `build_caller` fails open
    #                  on a bad chain (F5b, io.rs:241-247): expired, revoked
    #                  or garbage leaves auth at Delegated and grants
    #                  nothing, while preamble.ucan stays Some -- so keying
    #                  on the preamble would let any caller self-label
    #                  `ucan` with a junk string. AuthLevel::Ucan is set
    #                  only on a verified, unrevoked, capability-bearing
    #                  chain (io.rs:232-234).
    #   `delegated` <- preamble, NOT c.auth. build_caller assigns
    #                  AuthLevel::Delegated to every verified preamble
    #                  (io.rs:173), including the gateway's unchallenged
    #                  node-DID pubkey (F5a). A malformed certificate is a
    #                  hard reject earlier (io.rs:346-355), so a delegation
    #                  present here did verify.
    auth = if matches!(c.auth, AuthLevel::Ucan)   { Ucan }
           else if preamble.delegation.is_some()  { Delegated }
           else                                   { SelfAsserted }

    Ok(Some(GuestCallerIdentity { did: c.caller_did.clone(), auth,
                                  app_instance: c.app_instance.clone() }))

build_guest_response(r) -> Response:
    if r.body.len() > MAX_GUEST_RESPONSE_BODY_BYTES:
        return http_error(500, "guest response body exceeds N byte limit")
    if r.headers.len() > MAX_GUEST_RESPONSE_HEADERS:
        return http_error(500, "guest response declares more than N headers")
    if !(200..600).contains(r.status) or StatusCode::from_u16 fails:
        return http_error(500, "guest returned an out-of-range status: {r.status}")
        # 1xx rejected too: informational is not a final response

    builder = Response::builder().status(status)
    saw_content_type = false; saw_nosniff = false
    for (name, value) in r.headers:
        lower = name.to_ascii_lowercase()
        if HOST_OWNED_HEADERS.contains(lower):
            debug!(header = lower, "stripping host-owned header from a guest response")
            continue
        let Ok(hn) = HeaderName::from_bytes(lower.as_bytes()) else:
            return http_error(500, "guest returned an invalid header name: {name:?}")
        let Ok(hv) = HeaderValue::from_str(&value) else:
            return http_error(500, "guest returned an invalid value for header {lower}")
            # from_str rejects CR/LF -- this is the header-injection close
        saw_content_type |= lower == "content-type"
        saw_nosniff      |= lower == "x-content-type-options"
        builder = builder.header(hn, hv)       # appends: repeated set-cookie survives

    if !saw_content_type: builder = builder.header(CONTENT_TYPE, "application/octet-stream")
    if !saw_nosniff:      builder = builder.header(X_CONTENT_TYPE_OPTIONS, "nosniff")
    builder = builder.header(CONTENT_LENGTH, r.body.len().to_string())   # always the host's
    builder.body(full_body(Bytes::from(r.body)))
```

Two things that must not be got wrong: `Content-Length` is the host's computed
one and never the guest's (a mismatch is a connection desync), and an invalid
header **fails the response** rather than being dropped — a guest that thought
it set `Content-Type: application/json` must not silently serve octet-stream.

### 5.3 Engine — `handle_guest_http_request`

```
handle_guest_http_request(service_id, request, caller) -> Result<GuestHttpOutcome>:
    Self::validate_service_id(service_id)?
    debug_assert!(caller is not LocalElevated | LocalReadOnly)
        # identical guard and reasoning to prepare_wasm_execution

    # D-A2-11: bounded queuing instead of the pool's hard refusal. Per
    # service, so one service's traffic degrades that service.
    #
    # MUST NOT be written as `entry(..).or_insert_with(..)` followed by an
    # `.await` on the result: `entry` returns a `RefMut` that holds the
    # DashMap shard's write lock for as long as it lives, so awaiting the
    # permit would block every other task touching that shard for up to
    # GUEST_HTTP_ADMISSION_TIMEOUT. Clone the `Arc` out and drop the guard
    # in its own scope, BEFORE the await. Do not "simplify" this back.
    let permits: Arc<Semaphore> = {
        let entry = self.guest_http_permits.entry(service_id.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(
                self.max_concurrent_guest_http_per_service as usize)));
        entry.value().clone()
    };  # guard dropped here
    let Ok(Ok(_permit)) = timeout(GUEST_HTTP_ADMISSION_TIMEOUT,
                                  permits.acquire_owned()).await
        else: return Ok(Failed(Unavailable("guest HTTP admission timed out")))

    caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id))

    let _active = ActiveInstanceGuard::new()          # call site 12: metrics parity
    (store, instance, _fuel) = match self.build_store_and_instantiate(
            service_id, caller, self.dispatch_epoch_ticks, InstanceOptions::default()).await {
        Ok(v)  => v,
        Err(e) if e.downcast_ref::<wasmtime::PoolConcurrencyLimitError>().is_some() =>
            return Ok(Failed(Unavailable(e.to_string()))),   # public in 46.0.2, verified
        Err(e) => return Err(e),
    }
        # epoch + fuel + memory limiter armed here; A1's instantiation
        # counter increments here too, which is what §7's delta assertions read

    let Ok((func, results_len, _item)) = Self::get_wasm_func(
            &mut store, &instance, Some(HTTP_HANDLER_INTERFACE), "handle-request")
        else: return Ok(Failed(NoHandler))

    args = [http::request_to_val(request)]
    results = vec![Val::Bool(false); results_len]
    exec_start = Instant::now()
    let call = func.call_async(&mut store, &args, &mut results).await
    histogram!("substrate.wasm.execution_ms").record(exec_start.elapsed())   # call site 12
    if let Err(e) = call:
        return Ok(Failed(match classify_call_failure(&e) {
            OutOfFuel | Deadline => BudgetExceeded(truncate_detail(e.root_cause().to_string())),
            MemoryFault | Other  => Trap(truncate_detail(e.root_cause().to_string())),
        }))

    match http::response_from_results(&results):
        Ok(response) -> Ok(Response(response))
        Err(failure) -> Ok(Failed(failure))
```

### 5.4 Engine — `request_to_val` / `response_from_results`

```
request_to_val(r) -> Val:
    pairs(v) = Val::List(v.map(|(k, val)| Val::Tuple(vec![Val::String(k), Val::String(val)])))
    Val::Record(vec![                     # WIT declaration order, exactly
        ("method",      Val::String(r.method)),
        ("path",        Val::String(r.path)),
        ("query",       Val::String(r.query)),
        ("route",       Val::String(r.route)),
        ("path-params", pairs(r.path_params)),
        ("headers",     pairs(r.headers)),
        ("body",        bytes_to_val_list(r.body)),   # reused from stream.rs
        ("caller",      Val::Option(r.caller.map(|c| Box::new(Val::Record(vec![
                            ("did",          Val::String(c.did)),
                            ("auth",         Val::Enum("delegated" | "ucan")),
                            ("app-instance", Val::Option(c.app_instance.map(..))),
                        ]))))),
    ])

response_from_results(results) -> Result<GuestHttpResponse, GuestHttpFailure>:
    # Distinguish a guest `Err(msg)` from a wrong shape BEFORE delegating:
    # `extract_result` collapses both into one anyhow::Error, and they must
    # not both become `Malformed`. Do not "simplify" this away.
    match results:
        [Val::Result(Err(payload))] -> return Err(Declined(truncate_detail(payload as string)))
        [Val::Result(Ok(Some(Val::Record(fields))))] -> fields
        other -> return Err(Malformed("expected result<http-response, string>, got {other:?}"))

    status  = field "status"  as Val::U16                                          else Malformed
    headers = field "headers" as Val::List of Val::Tuple[Val::String, Val::String]  else Malformed
    body    = field "body"    via val_list_to_bytes                                 else Malformed
    Ok(GuestHttpResponse { status, headers, body })
```

### 5.5 Deploy — the refusals and the `info!`

```
# (a) in deploy_with_context, immediately after A1's assets check (:1653-1659)
if http_routes.iter().any(|r| r.target == "guest") && service_type != AppServiceType::Wasm:
    return Err("service '{id}': an http_routes entry with target=guest is only servable for a
                'Wasm' service; a '{service_type:?}' service's endpoint is raw TCP passthrough,
                which never reaches the guest HTTP path")

# (b) in deploy_wasm_service, right after the stage-4 export check (:799)
if http_routes.iter().any(|r| r.target == "guest")
   && !self.app_sandbox_engine.exports_http_handler(service_id):
    self.rollback_config_generation(service_id, new_gen).await
    self.rollback_fdae_policy(service_id, previous_fdae_policy).await
    return Err("service {id} declares an http_routes entry with target=guest, but the deployed
                component does not export syneroym:http/incoming-handler#handle-request")
# the caller's existing `rollback_asset_bundle` on this branch (:1877) covers A1's half

# (c) beside A1's asset-bundle info! (~:1846) -- D-A2-7's loud signal
for r in http_routes.iter().filter(|r| r.target == "guest" && r.public):
    info!("guest HTTP route for '{id}': {} {} declared public -- reachable with no verified
           caller identity (M06A D-A2-7)", r.method, r.path)
```

---

## §6 The test fixture — `test-components/http-guest-test`

`syneroym-test-http-guest`, `crate-type = ["cdylib"]`, WIT copied to
`wit/deps/http/http.wit` (mirroring how `stream-test` copies
`syneroym:messaging`). Built and consumed like every other fixture: built by
hand with `cargo build --release --target wasm32-wasip2`, excluded from the
workspace graph, and every test that needs it **skips with a message when the
artifact is absent** ([stream_integration.rs:169](../../../../crates/sandbox_wasm/tests/stream_integration.rs#L169)).

**It exports two interfaces, not one.** `syneroym:http/incoming-handler` is the
subject under test; `syneroym-test:http-guest-test/test-driver@0.1.0` exists as
a **second assertion channel** — `last-request() -> string` lets a test check
what the guest actually received, not only what it chose to echo back. That is
the whole reason. Revision 2 also claimed `--interfaces` forces one, which is
**wrong**: a blank value defaults to `DEFAULT_INTERFACE_NAME`
([svc.rs:516-519](../../../../apps/roymctl/src/commands/svc.rs#L516)), so
`--interfaces ""` would have been a valid answer. `stream-test` has the same
two-interface shape for the same (real) reason.

Behaviour switches on `request.path`, so one binary covers the matrix:

| Path | Behaviour | Proves |
|---|---|---|
| `/echo` | 200, JSON echoing `method`/`path`/`query`/`route`/`path-params`/header count/body length/`caller` | Every field of `http-request` arrives, including `D-A2-12`'s caller |
| `/items/{id}` | 200, body is the captured `id` | `D-A2-4`'s `param_name`/`match_path` agreement |
| `/whoami` | 200, body is `"anonymous"` or `"{auth}:{did}"` — **`auth` included deliberately**, so a test can assert the *shape* per transport (F5a): `self-asserted:<node-did>` through the gateway, `anonymous` over direct WebRTC, `delegated:<caller-did>` with a real certificate | `D-A2-12`, and the F5a transport split |
| `/reject` | `Ok` with `status: 422`, body `"comment is empty"` | **Exit criterion 5** — the guest's own status and message |
| `/fail` | `Err("handler blew up")` | `Declined` → 500 carrying the message |
| `/trap` | `unreachable!()` | Matrix row 5, trap half |
| `/spin` | `loop {}` | Matrix row 5, wasm-execution half (see §10.5) |
| `/huge` | `Ok` with a body over `MAX_GUEST_RESPONSE_BODY_BYTES` | Matrix row 6, oversized half |
| `/bad-header` | `Ok` with a header value containing `\r\n` | Matrix row 6, malformed half + header injection |
| `/framing` | `Ok` setting `content-length: 999` and `connection: close` | `D-A2-5`'s strip rule |
| `/slow` | busy-waits ~1s, then 200 | `D-A2-11`: N of these in parallel queue rather than 500 |

---

## §7 Phases

| Phase | Scope | Done when |
|---|---|---|
| **P1** | WIT + `core::guest_http` + `HttpRoute.public` + `param_name` + config knob + `stream.rs` visibility + `D-A2-6`'s classifier refactor | `cargo check --workspace` clean; **`cargo test -p syneroym-sandbox-wasm` and `-p syneroym-control-plane` unchanged** — the classifier must move no behaviour (call sites 13, 14) |
| **P2** | `sandbox_wasm/src/http.rs` + `handle_guest_http_request` + `exports_http_handler` + permits + metrics | Unit tests for `request_to_val`/`response_from_results` (every `Malformed` shape) pass with no component involved |
| **P3** | The fixture + `test_constants` | Fixture builds for `wasm32-wasip2`; a `sandbox_wasm` integration test drives `handle_guest_http_request` end to end |
| **P4** | Router: `dispatch_route` arm, `handle_guest_route`, 401 gate, header/response mapping | Free-function unit tests pass; an integration test gets a guest response over the HTTP bridge |
| **P5** | Deploy-time refusals, `validate_route`, `info!`, SDK reshape, `roymctl --custom-config` | `control_plane` tests for both refusals; a CLI deploy declaring a guest route works end to end |
| **P6** | Failure matrix row 5 (wasm half) and row 6, exit criterion 5, `D-A2-11` concurrency | §8 complete |

P3 before P4 deliberately: the router work is untestable end to end without a
component that exports the handler, and A1's experience was that the fixture is
where the boundary's real shape shows up.

---

## §8 Tests

Covering **A2's rows only**. Matrix rows 1–4 and 7 are A1's (shipped); row 8
names SSE and is A3/A4's, though `D-A2-11` applies its principle here. Exit
criteria 1, 2, 4, 6, 7 belong to A3/A4.

| Test | Covers |
|---|---|
| `param_name` returns the last `{...}` segment, `None` for a literal, and agrees with `match_path` on a two-capture pattern | `D-A2-4`, F10 |
| `request_to_val` builds fields in WIT declaration order; `list<tuple<..>>` as `Val::List(Val::Tuple(..))`; `caller: none` and `caller: some` both shaped right | §5.4, `D-A2-12` |
| `response_from_results`: valid record; guest `Err(msg)` → `Declined` (**not** `Malformed`); wrong arity, non-record `Ok`, missing field, wrong field type, non-`u8` body element → `Malformed`, one case each | Matrix 6, §5.4 |
| `guest_request_headers` lowercases, drops non-UTF-8, removes every `HOST_OWNED_HEADERS` entry, 431s past the count cap | `D-A2-5`, `D-A2-8` |
| `guest_caller_identity`: `None` → `Ok(None)`; **bare pubkey → `SelfAsserted` even though `CallerContext.auth` says `Delegated`** (F5a); **`preamble.ucan = Some(junk)` with `CallerContext.auth = Delegated` → `SelfAsserted`, never `Ucan`** (F5b — the attacker-controlled case, and the one revision 3 got wrong); `AuthLevel::Ucan` → `Ucan`; delegation present → `Delegated`; each of `LocalElevated`/`LocalReadOnly`/`System` → `Err` | `D-A2-12`, F5a, F5b, fail-closed |
| e2e: a connection presenting a **rejected** UCAN (expired or garbage) plus a bare pubkey reaches a `public` route and `/whoami` reports `self-asserted`, not `ucan` — driven over the wire, so it also pins `build_caller`'s fail-open staying fail-*closed* at this boundary | F5b |
| **`HttpRoute` with no `public` key deserializes to `public: false`** — one line, asserted directly, because it is the line that makes the default safe | `D-A2-7` |
| `build_guest_response`: strips framing headers; rejects invalid name, invalid value, CR/LF value, status 0/99/600, over-cap body, over-cap header count; sets `Content-Length` from the body; adds `nosniff` only when absent; keeps two `set-cookie` headers | `D-A2-5`, matrix 6 |
| `classify_call_failure` maps fuel/memory/deadline/other, **and** `execute_wasm_vals`/`authorize_rows` produce byte-identical errors to before the refactor — including `MemoryFault` still reaching `AbacError::Trap` | `D-A2-6`, review finding 4 |
| Engine integration: `handle_guest_http_request` against the fixture returns the echoed request; against `greeter` (no such export) returns `Failed(NoHandler)` | §5.3 |
| Deploy refuses a `guest` route on a `Tcp` service and on a `Container` service, before anything fallible runs | `D-A2-10a` |
| Deploy refuses a `guest` route when the component lacks the export, rolling back config generation, FDAE policy **and** any asset bundle written | `D-A2-10b`, F12 |
| `validate_route` accepts `("guest", "handle-request")`, rejects `("guest", other)`, and rejects `public: true` on a `data-layer` route | §3.8 |
| e2e: **anonymous request to a non-`public` guest route → 401, instantiation delta 0** (drive it with a preamble carrying no usable pubkey, the WebRTC shape) | `D-A2-7` |
| e2e: same route declared `public` → the guest answers, `/whoami` returns `anonymous` | `D-A2-7`, `D-A2-12` |
| e2e: a delegated connection to a non-`public` route reaches the guest, and `/whoami` returns `delegated:<caller-did>` | `D-A2-12`, F5 |
| **e2e through the client gateway: a non-`public` route is reached anyway (the 401 does not fire), and `/whoami` returns `self-asserted:<node-did>` — the node's own DID, not an end user's** | **F5a**, `D-A2-7`'s stated scope, `D-A2-12`'s `self-asserted`. This test exists to pin the limitation in code, so it cannot quietly stop being true or be mistaken for authentication |
| e2e `POST /reject` returns **422 and the guest's own message** | **Exit criterion 5** |
| e2e `POST` with an over-cap body → 413, instantiation delta 0 | `D-A2-8`, reusing A1's `instantiations()` |
| e2e `GET /trap` and `GET /spin` each → 500 with a structured error, **and a second request on a new stream still succeeds** | Matrix 5, wasm half (§10.5) |
| e2e `GET /huge` and `GET /bad-header` each → 500, **no partial body**, no chunked framing | Matrix 6 |
| `max_concurrent_guest_http_per_service + 2` parallel `/slow` requests: all succeed (queued), none 500s; with the permit budget forced to 1 and the admission timeout forced low, the excess returns **503 + `Retry-After`**, not 500 | `D-A2-11` |
| e2e: a `guest` route and a `data-layer` route on one service both work, neither shadowing the other | F1, regression |
| e2e: a `guest` route and an asset bundle coexist (asset served with zero instantiation, guest path invoked) | A1/A2 independence |

---

## §9 What this plan does not decide

1. **A typed (`bindgen!`) guest-export call path.** `D-A2-2` takes the dynamic
   one. Backlog row owed: *a guest HTTP body is marshalled as `Vec<Val::U8>` in
   both directions, the same per-chunk cost `stream.rs` already pays; the typed
   component-model path would lower `Vec<u8>` directly, and is the fix if
   `MAX_GUEST_REQUEST_BODY_BYTES` ever rises materially above 1 MiB.*
2. **A wall-clock ceiling on a guest HTTP request.** F7: the epoch deadline
   interrupts guest wasm only; a guest blocked in a host call has no bound on
   this path — exactly as it has none on the JSON-RPC bridge into a guest
   today. Backlog row owed; pre-existing shape, and a second timeout knob for
   one route target would be the wrong place to fix it.
3. **Streaming a guest response.** The whole body is buffered, which is what
   makes matrix row 6's "bounded and rejected, not streamed" enforceable at
   all. Large payloads already have a home: the `stream` target, which is what
   task.md's reference scenario steps 8 and 9 use.
4. **Idempotency fencing for a guest `POST`.** `dispatch_json_rpc_once`'s
   `KeyProbe` fence never applies to any bridged HTTP route (`dispatch_native`
   synthesises requests with `idempotency_key: None`), and A2 does not change
   that. Consistent, not accidental — but worth a backlog row, since a browser
   retrying a `POST` is exactly the case the fence exists for.
5. **Extending `public` to the `stream` target.** R2-B: with `D-A2-7` in place,
   `stream` is the only bridge target reaching guest code with neither a caller
   check nor a declaration. Changing it is M3B behaviour with no consumer in
   this milestone asking for it. Backlog row owed.
6. **Re-tuning the global instance-pool accounting.** `D-A2-11` bounds guest
   HTTP per service but leaves `STREAM_INSTANCE_POOL_HEADROOM` and the
   stream/ABAC budgets exactly as they are, even though F8 shows ordinary calls
   have zero dedicated slots at the default pool size. Re-deriving those
   numbers affects every call path, and their arithmetic is asserted by an
   existing test. Backlog row owed: *A3's Playwright run under a real browser
   is the first realistic measurement of concurrent guest instantiation; re-tune
   from that, not from a guess.*
7. **`HEAD` on a guest route.** `resolve_route` matches the method exactly, so
   a declared `GET /api/x` does not answer `HEAD /api/x`. The fix is a general
   route-table question, not a guest-target one.
8. **SPA history fallback / deep links, which A3 needs and neither A1 nor A2
   provides.** `D-A1-11` deliberately left `/some/route` → `index.html` as
   "A3's problem", and F10 shows the route table cannot cover it either:
   `match_path` requires equal segment counts and has no wildcard, so a deep
   link of unknown depth cannot be declared as a route at all. So A3 starts
   blocked on a mechanism no shipped slice has. Recorded here because A2 is the
   only slice touching path matching, and the two candidate fixes both belong
   to whoever picks this up: a wildcard/prefix segment in `match_path` (which
   would also need a matching extension to D-A1-4's collision check), or an
   asset-side fallback rule reinstating what `D-A1-11` removed, scoped so it
   cannot swallow `/api/*`. **Not** done here: A2 has no consumer for it, and
   revision-2 finding 1 of the A1 review is the record of what happens when a
   fallback is added without one.
9. **A real end-user identity at the gateway.** F5a: the gateway proxies under
   the *node's* DID, so no guest can learn who is at the keyboard through it.
   That is M06B's "person identity at the gateway", explicitly out of M06A
   ([task.md](task.md)'s non-goals). A2's job here is to stop the WIT from
   *implying* otherwise, which `self-asserted` does. Backlog row owed, pointing
   at the gateway's own `TODO(post-B0)`
   ([gateway.rs:46-56](../../../../crates/client_gateway/src/gateway.rs#L46)).
10. **A3's `public: true` requirement — flagged here because A2 is where it
    becomes true.** Every route A3's demo declares must set `public: true`, or
    exit criterion 2 ("all four Playwright cases pass in the **direct WebRTC**
    configuration") fails at the first request: that transport is genuinely
    anonymous (F5a), so `public: false` returns 401 before the guest runs. The
    same routes would appear to work through the local gateway, which makes
    this a difference that only shows up in the configuration the exit criterion
    names. A2 does not set it for them — A3 owns its own manifest — but A3's
    plan must not discover it from a red test.
11. **Guest-originated SSE.** `D-06A-2` keeps live updates on the existing
   `messaging`/`subscribe-sse` route, which A3 uses. A guest handler cannot
   hold a stream open through `result<http-response, string>`, deliberately.

---

## §10 Corrections owed to other documents

1. **task.md, Migration impact bullet 3** — "Slice A2's plan may add further
   WIT surface if it chooses a dedicated guest export" resolves to **does**: a
   new `syneroym:http@0.1.0` package with `incoming-handler`, two records and
   one enum. Additive, and *not* added to the `host-environment` world, so a
   component that does not export it deploys exactly as before.
2. **task.md, second open design point** ("What a guest HTTP handler looks like
   in WIT") is resolved by `D-A2-1`. Its note that "the choice affects M06C's
   Web entrypoint" could not be checked against anything — no M06C document
   exists in the tree yet.
3. **task.md, failure-matrix row 8** (many concurrent SSE subscribers) is not
   A2's; A2 adds no SSE. Its *principle* ("degrades that service, not the
   node") is what `D-A2-11` applies to guest HTTP, and the row should say the
   principle is general rather than SSE-specific.
4. **task.md, failure-matrix row 6** ("Bounded and rejected, not streamed to
   the client") needs one word of precision: the host cannot bound the
   *allocation*. A guest's `list<u8>` return is fully materialised in host
   memory before its size is knowable; what bounds that is the guest's own
   `max_memory_bytes` store limiter, and what `MAX_GUEST_RESPONSE_BODY_BYTES`
   bounds is what gets **sent**.
5. **task.md, failure-matrix row 5** ("Guest route handler traps or exceeds its
   epoch bound") — A2 covers the **wasm-execution half only**. The epoch
   deadline does not interrupt a guest blocked inside a host call (F7), and
   that half is deliberately left untested rather than tested badly: no
   guest-reachable host function in this tree blocks unboundedly, so the case
   cannot be provoked from a fixture today. The row should say the bound is on
   guest execution, and §9.2's backlog row is where the gap lives.
6. **task.md, A2's slice-table scope line** should mention the deploy-time
   export check (`D-A2-10b`), the non-`Wasm` refusal (`D-A2-10a`), and the
   `public` opt-in (`D-A2-7`) — the difference between "a wrong route 500s in
   production" and "a wrong route fails the deploy", and between an
   authenticated and a world-reachable endpoint.
7. **`docs/system-architecture.md:1929`**, the "HTTP Passthrough (M3C, Slice
   7)" bullet, on two counts. It enumerates the bridge's targets, which A2
   extends. And its "Gap closed (M04A Slice B0)" sentence — "an anonymous
   caller is rejected before the native service is invoked (bridged routes
   return HTTP 401)" — is stated as though it covered the whole bridge; it has
   never covered the `stream` target, which reaches guest code with no caller
   check, and A2 adds a second exception under an explicit `public`
   declaration. This is the one line in the architecture doc a reader would use
   to check A2's security posture, so it must say: native-capability targets
   reject anonymous callers; guest targets reach guest code, and only when the
   route declares `public`.
8. **`crates/client_gateway/src/gateway.rs:52-56`**, and the backlog row it
   points at. The comment argues that proxying under the node DID is "harmless
   today only because it proxies to deployed services, never to
   `orchestrator`/`security`". That reasoning depends on a deployed service
   ignoring the caller — true while the bridge's targets were `data-layer`,
   `messaging` and `stream`. A2 makes "deployed service" include **guest code
   that may branch on `caller.did`**, so the note must say that a guest HTTP
   handler now sees this DID, that it is the node's own and identical for every
   visitor, and that A2 labels it `self-asserted` in the WIT precisely so a
   guest cannot mistake it for a user. R3-A: this correction belongs to A2, not
   to M06B.
9. **`status.md` — done (2026-08-14).** It named
   [slice-a1-implementation-plan.md](slice-a1-implementation-plan.md) as *the*
   design of record for the milestone; it now names both, A2's slice row reads
   "Planned" rather than "Not started", and an "A2 — Planned" section records
   the two decisions that changed during review plus the two gaps A3 inherits
   (§9.8, §9.10). The remaining eight corrections above are still owed, and
   land with the implementation.

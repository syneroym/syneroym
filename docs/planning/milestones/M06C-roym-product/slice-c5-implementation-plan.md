# M06C Slice C5 — Catalog and Conversation in the Product: Implementation Plan

> **Scope.** [task.md](task.md)'s **C5** row — R1 rows 2 and 3: the versioned
> listing schema across all seven dimensions the spec names (booking,
> payment, product, service, location, relationship, service-record),
> signed and editable, where a material change produces a new version with
> `supersedes` and never an edit; 1:1 conversation over B4 with
> `pending` / `delivered` / `failed` visible to the person, never
> optimistic, surviving a restart on both sides; and Roym's own copy of
> conversation content (`D-06C-5`), which is what export, search, and
> delete act on. Gate: **C4**.
>
> **C5 also owes five things C4 handed it by name**
> ([status.md](status.md) §9, [deferred-backlog.md](../../deferred-backlog.md)):
> R1 row 6's inbox enforcement point (`D-C4-13`), R1 row 1's
> conversation-history half, the publication limiter's catalog-side caller
> (`D-C4-15`), wire-side authorization on the services a foreign caller can
> reach (`D-C4-4`/`F3`), and the `conversation → profile` `depends_on` edge
> `D-C4-11` deferred until there was a caller for it. Mounting
> `roym_core::signing::handle_certificate_verb` on the public services was
> blocked on that authorization and unblocks here.
>
> **Read §18 first if you are executing this plan.** Four claims in the
> input documents do not hold against the tree. Two of them (**A**, **B**)
> change what gets built, and **B** is the reason this slice contains one
> new host interface rather than none.
>
> **Planning identifiers appear in this document and must not appear in
> the code it describes.** AGENTS.md forbids slice and milestone ids in
> comments and doc comments, not only in names. Every code block below is
> written with that already applied. §16 item 11 checks comments, not just
> names.

---

## §0 What C4 handed C5, and what is missing

| Handed over | Where |
|---|---|
| A `profile` service with real product state, eight collections, and 25 verbs | `crates/roym_profile/src/app.rs` |
| The block list and the admission decision, both tested, with no caller | `block.*`, `contacts.admit-first-contact` (`app.rs`), `roym_core::safety::admit_first_contact` |
| `safety::admit_publication` and `PublicationLimits`, unit-tested, **with no caller at all** | `crates/roym_core/src/safety.rs` |
| A per-service record-signing certificate store and a shared verb handler, mounted on `profile` alone | `roym_core::signing::{install, status, person_principal, handle_certificate_verb}` (`crates/roym_core/src/signing.rs:226`) |
| A versioned, integrity-checked app-data bundle with per-section digests | `roym_core::backup::{Bundle, BundleManifest, SectionDigest}` |
| `MethodAuth` (`Public` / `Owner`, no default arm) and `web`'s `admit()` on both of its ingress paths | `crates/roym_core/src/router.rs`, `crates/roym_web/src/app.rs` |
| The one wall-clock read, and the rule that every rule below it takes `now_secs: u64` | `roym_core::clock::now_secs` |
| `ProfilePayload.conversation_address` — the person→conversation-address mapping, signed into a `profile` record | `crates/roym_core/src/person.rs` |
| `content_digest(prefix, value)` as the single content-hash definition | `syneroym_signed_record::content_digest` |
| A parity harness with owner, record signer, pinned `RecordClock`, and one certificate installed byte-identically on both stacks | `crates/roym_web/tests/dual_build_parity.rs` |
| A two-substrate conversation e2e harness with boot / deploy / restart | `crates/substrate/tests/conversation_e2e.rs` |

Missing, and C5's to build:

1. **No catalog state.** `roym_catalog::app::invoke` answers `listing.ping`
   and nothing else (`crates/roym_catalog/src/app.rs`). No listing type
   exists anywhere in the tree.
2. **No conversation state.** `roym_conversation::app::invoke` answers
   `conversation.ping`. Roym has no copy of any message.
3. **No inbound path at all.** Neither the WASM world nor the native
   wiring receives `on-message`: `crates/roym_conversation/wit/world.wit`
   exports only `api`, and `init_roym`
   (`crates/substrate/src/runtime.rs:1530`) never calls
   `set_conversation_sink` for any Roym factory. A message delivered to a
   Roym installation today reaches the host store and stops there.
4. **No way for a service to know who is calling it.** Outside
   `syneroym:http`'s `caller-identity`, nothing tells a component whether
   an `api.invoke` came from a sibling or off the wire (`F4`). This is what
   makes wire-side authorization impossible in guest code today, and it is
   why §3 exists.
5. **No caller for `admit_publication`**, and therefore no publication
   limit in force anywhere.
6. **No `depends_on` edge from any sibling.** Only `web` declares
   dependencies; `F2` (C4) showed resolution works without the
   declaration, which is the gap `D-C4-11` recorded rather than papered
   over.

---

## §1 Findings from reading the tree

Verified 2026-09-02 against `feat/m06c-slice-c4` at `5077560`. Each is
load-bearing for a decision in §2. Line references were checked, not
carried over.

### F1 — a conversation peer is addressed by the peer's Roym Conversation **service id**, and that is the same string that reaches its `api.invoke`

Four facts, each read from the tree:

1. `ProfilePayload.conversation_address` is documented as *"The routing
   service id `open-direct` takes -- this person's own Conversation
   service"* (`crates/roym_core/src/person.rs:21`), and C4 signs it into
   the `profile` record so *"a stranger who verifies the profile has an
   address they can attribute"*.
2. `open-direct`'s own WIT says `peer-address` is *"the peer's Conversation
   **service** id -- the same string a proxy call would target"*
   (`crates/wit_interfaces/wit/conversation/conversation.wit`).
3. Delivery uses it verbatim as `ProxyRequest.target_service` with
   `interface: "conversation"`
   (`crates/conversation/src/transport.rs:88`), and `invoke_remote`
   resolves it through the registry
   (`crates/router/src/proxy.rs:749`).
4. The route preamble is `<scheme>://<interface>.<service_id>`
   (`crates/router/src/preamble.rs:7`).

So one string does two jobs: *"send me messages here"* and *"call this
service's API here"*. Every consumer who receives a listing (the spec's own
journey step C10, *"Consumer starts a conversation from a listing"*) and
every directory a provider publishes to holds it. Nothing has to be guessed
or scanned — the community registry has no listing endpoint at all, only
`GET /lookup/{service_id}` (`crates/community_registry/src/registry.rs:306`).

### F2 — the Roym Conversation service **must** stay publicly resolvable

Following from `F1`(3): the sender's node resolves the recipient's
conversation service id through the registry before it can deliver
anything. `visibility` (ADR-0018 §1) decides publication: `public` is
propagated to the community registry, `internal` is this substrate's
registry only, `private` is registered nowhere, and ADR-0018 §3's
peer-known-records store is still deferred. A `conversation` service that
is not `public` cannot be reached from another installation, so R1 row 3
cannot pass. **Narrowing visibility is therefore not available as the fix
for `F3`.**

### F3 — inbound wire dispatch into a WASM component admits an anonymous caller, by design, with no per-method gate anywhere

`dispatch.rs`'s WASM arm forwards the router-verified caller (or `None`)
straight into `HostState.caller` and runs the export
(`crates/router/src/route_handler/dispatch.rs:242`). Its own comment is
explicit: *"native interfaces reject anonymous callers, WASM guests admit
them"*, under a live `TODO(B7b / post-B7)` for the question *"may this
caller touch this service at all?"* — the same TODO appears at
`crates/router/src/route_handler/io.rs:147`. `EndpointInfo` carries no
interface list (`crates/core/src/dht_registry.rs:76`), so a published
service is reachable on **every** interface it registers, and
`register_wasm_endpoints` registers each interface the manifest declares
(`crates/control_plane/src/service/orchestration.rs:606`).

`topology_visibility = open` does **not** help: it is consulted only in the
supervisor's logical-name resolution (`crates/app_supervisor/src/service.rs:5350`),
never on a direct DID dispatch.

### F4 — nothing tells guest code who called it, except an inbound HTTP request

A grep for `caller` across every `.wit` in `crates/wit_interfaces/wit/`
returns prose in `signing`, `proxy`, `control-plane`, `supervisor` and
`conversation`, and exactly one *type*: `caller-identity` in
`syneroym:http/incoming-handler`. `syneroym_roym_core::envelope::Request`
says the same thing from the other side, and the backlog already carries
the row (*"The Roym `invoke` envelope carries no caller field, so a sibling
cannot learn who called it even when the host knows"*).

So a guest reached at `api.invoke` cannot make an authorization decision
about its caller. `F1`–`F4` together are why §3 adds a host interface
rather than a guest-side check.

### F5 — a sibling call and an anonymous wire call collapse to the identical `HostState.caller`

`prepare_wasm_execution` substitutes `CallerContext::service_system(service_id)`
whenever no caller is supplied (`crates/sandbox_wasm/src/engine.rs:1378`).
Both production call sites can supply none: `ProxyRouter::invoke_local`
passes `None` unconditionally for a `WasmChannel` target
(`crates/router/src/proxy.rs:729`), and `dispatch.rs` passes `None` for an
unauthenticated wire connection. The resulting `CallerContext` is
byte-identical.

Consequence: **exposing `HostState.caller` to a guest would not be enough.**
The origin has to be carried separately, which is what §3.2 does. It is
also why `caller_binding` (`crates/sandbox_wasm/src/host_capabilities.rs:581`)
maps both to `CallerBinding::Internal` — correct for what it does, and
insufficient for what C5 needs.

### F6 — `execute_wasm_json` has exactly two production call sites

`crates/router/src/proxy.rs:729` (local dispatch) and
`crates/router/src/route_handler/dispatch.rs:242` (wire ingress). Every
other match is a test. So the origin can be threaded with a second
entrypoint on the engine and one changed line at the wire site, with no
churn across ~15 test call sites (`D-C5-2`).

### F7 — the guest-export delivery path exists and is proven, and Roym uses none of it

`AppSandboxEngine::notify_guest_message` invokes the component's optional
`syneroym:conversation/guest-api@0.1.0#on-message` export with a 4-attempt
retry, silently discarding when the export is absent
(`crates/sandbox_wasm/src/engine.rs:1689`, interface constant at `:1694`);
`notify_guest_state` mirrors it for `on-delivery-state`. On the native side
`NativeHostFactory` implements `ConversationNotifier` and calls its
`Weak<dyn ConversationSink>` with no retry — a stated, permitted difference
(`crates/app_host_native/src/factory.rs:367`). Every factory registers
itself as its own service's notifier at construction (`:125`).

`test-components/dual-build-fixture` exports
`syneroym:conversation/guest-api@0.1.0` in its world and implements the
sink natively (`src/guest.rs:98`, `src/native.rs:84`), so the whole shape
is proven on both builds.

**What Roym is missing:** `crates/roym_conversation/wit/world.wit` declares
no export but `api`, and neither `init_roym`
(`crates/substrate/src/runtime.rs:1530`) nor
`crates/roym_web/tests/dual_build_parity.rs` ever calls
`set_conversation_sink` or `set_notifier`. Both are one line each per
stack.

### F8 — a signed payload may not contain a non-integer number

`RecordDraft::validate` walks the payload and returns
`DraftError::PayloadNonIntegerNumber` for any `Value::Number` that is
neither `i64` nor `u64`
(`crates/signed_record/src/envelope.rs:76`), because the canonical encoding
is only reproducible for integers. `syneroym:signing`'s own WIT states the
consequence: *"a price is minor units, not a decimal"*. The same walk caps
a payload at 64 KiB canonicalized (`MAX_PAYLOAD_BYTES`) and 32 levels deep.

This decides the listing schema's numeric shapes outright: **money is minor
units, and coordinates are micro-degrees** (`D-C5-6`). It is not a style
preference; a float in a listing payload is refused by the host before it
signs.

### F9 — the filter DSL supports what C5's queries need; FTS5 stays C6's

`crates/data_db/src/filter.rs` compiles `$and`/`$or`/`$not`,
`$gt`/`$gte`/`$lt`/`$lte`/`$ne`/`$in`/`$nin`/`$regex` (`:243`) and bare
equality against `json_extract(payload, ?)`. So conversation search over
Roym's own copy is expressible with one indexed collection and a `$regex`
clause — no `execute-ddl`, no raw SQL, which stay C6's (Gap 7).

### F10 — the host's own retention caps bound the duplication `D-06C-5` costs

`conversation_max_body_bytes` defaults to 262 144 and
`conversation_max_messages_per_conversation` to 100 000
(`crates/core/src/config.rs:715`, `:720`), both enforced in
`crates/conversation/src/transport.rs:199` and
`crates/conversation/src/group.rs:156`. `data-layer` imposes no payload cap
of its own. So Roym's copy is bounded by the same numbers, and the honest
statement is *2× the host's own ceiling* — with real text traffic the
figure that matters is ~1 KiB per message, so 100 000 messages is ~100 MB
per copy (§2 `D-C5-7`).

### F11 — dependency resolution already works without a declaration, on both builds

C4's `F2` restated and re-verified. `install_app_context` registers each
binding into the node-wide resolver keyed by
`TopologyKey::local(instance_id, dependency_name)`
(`crates/control_plane/src/service/orchestration.rs:672`) and sets an app
context for every member, bindings or not (`:2326` runs whenever
`prepared_app_context` is `Some`). `init_roym` does the same for the native
build. Because `web` declares all five siblings, `TopologyKey::local(roym,
"profile")` already exists, so `conversation` could resolve
`Dependency("profile")` today with no manifest change. C5 declares the edge
anyway, because it now has the caller `D-C4-11` was waiting for.

### F12 — `on-message` may make a proxy call on both builds

On WASM, `notify_guest_message` instantiates with a `service_system`
caller; `check_native_capability_gate` restricts only the six reserved
native-capability interfaces (`crates/router/src/proxy.rs:589`), and
`syneroym-roym:profile/api@0.1.0` is not one, so a sibling call from inside
`on-message` passes. Natively, the fixture's own sink builds its host with
`CallerContext::service_system(&self.service_id)`
(`test-components/dual-build-fixture/src/native.rs:84`) and every Roym
factory has `set_service_proxy` called on it
(`crates/substrate/src/runtime.rs:797`,
`crates/roym_web/tests/dual_build_parity.rs:729`).

### F13 — the parity harness deliberately leaves `conversation` unbound

Both stacks build their topology with
`services::SIBLINGS.into_iter().filter(|s| s.name != "conversation")`
(`crates/roym_web/tests/dual_build_parity.rs:528`, `:622`), with a comment
saying scenario 5 needs a real unbound dependency. C5 binds `conversation`,
so scenario 5 needs a different unbound service. `transaction` still has no
verbs beyond `receipt.ping` and is the obvious replacement (`D-C5-12`).

### F14 — no host surface reports a service its own routing address

Checked, because C5 would like `listing.set` to fill in the provider's
conversation address by itself. `AppSigning::signing_identity()` returns
the *signing* key's DID, derived under the owner
(`crates/core/src/record_signer.rs:82`), not the service id.
`app-config` reads a deploy-time config generation
(`crates/sandbox_wasm/src/host_capabilities.rs:813`) and the service ids
are minted *during* the deploy, so they cannot be in it. `conversations()`
reports the service's own address inside `participants` for a direct
conversation (`crates/conversation/src/lib.rs:387`) — but only once a
conversation exists.

So the address stays person-supplied, exactly as C4 left it, and `catalog`
reads it from `profile` rather than asking the person twice (`D-C5-8`).
A backlog row records the gap.

### F15 — `roym_profile`'s import refuses a section version it does not recognise

`profile.import` compares every `declared.schema_version` against
`SCHEMA_VERSION` and refuses a mismatch outright
(`crates/roym_profile/src/app.rs`). `SCHEMA_VERSION` is 2 today. C5 changes
none of `profile`'s collections, so it stays 2 and no C4-produced bundle is
invalidated. `conversation` and `catalog` each go 1 → 2 as they gain their
first state.

### F16 — six parity scenarios pass without any verb handler existing

Counted against the tree, not assumed. Scenarios **4** (`nope.thing` →
`-32601` from `web`'s route table), **5** (`conversation.ping` → `-32001`,
refused as an unbound dependency), **7** (malformed body → `-32700`),
**9** (WebSocket lifecycle, three no-ops), **25** (`-32010` for four
methods) and **26** (`-32011` for four methods) all assert values that
`web` produces *before* any sibling runs. Every one of them would still
pass if the named verb had no handler at all.

That is fine for what those six were written to prove — they test `web`'s
own dispatch and gate. It stops being fine the moment a slice adds verbs
and points a scenario at them, which is what C5 does at scale.
`D-C5-13` fixes the rule that keeps C5's scenarios honest.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C5-1** | **C5 adds one host interface, `syneroym:invocation@0.1.0`, whose single function tells a component whether the call it is handling arrived from inside this node, from a verified party over the wire, or from the wire with no identity.** Three arms, no more: `internal`, `verified(did)`, `anonymous`. It reports the dispatch path and the router's own verification; nothing in a request body can influence it. | `F1`–`F4`: the conversation address a person publishes *is* the string that reaches `api.invoke`, the service must stay publicly resolvable (`F2`), and a guest cannot see its caller (`F4`). Every alternative was checked and fails. Narrowing `visibility` breaks messaging (`F2`). A guest-side check has nothing to check (`F4`). A router-level per-service interface allowlist is coarser, gives C6/C7 nothing to build a per-caller rule on, and still needs a manifest field. This is the capability the milestone's own preamble means by *"where a capability is genuinely missing, this document names it as a gap and gives it a slice"*, and the backlog already carries the row it closes. **It is a gap, not a convenience:** without it, `conversation.history` is readable by anyone holding an address the product hands out on purpose. |
| **D-C5-2** | **The origin is threaded with a second engine entrypoint, not a changed signature.** `AppSandboxEngine::execute_wasm_json` keeps its signature and means *local*; a new `execute_wasm_json_from_wire` means *wire*; both funnel into one private function. `dispatch.rs`'s one wire site switches; `proxy.rs` does not move. `HostState` gains one field. | `F5` says the caller alone cannot carry this and `F6` says there are exactly two production sites. A fifth parameter would touch ~15 test call sites for no gain, and every one of them means *local*. Two named entrypoints also make the wire path greppable, which a boolean argument would not. |
| **D-C5-3** | **Every verb on every service except `web`'s HTTP surface is admitted only for an `internal` invocation, checked by one shared helper `roym_core::admit::require_internal`, called as the first statement of each service's `invoke`. There is no per-verb exception list in C5.** Refusal is `-32013`. | The product has exactly one legitimate ingress today — the person's browser through `web`, which already applies `MethodAuth` with a verified session (C4 `D-C4-2`). Nothing else should reach a Roym verb, and a rule with no exceptions is a rule a reader can check. A per-verb allowlist would be a decision made by omission, which `D-C4-2` refused for the same reason. When C6 needs a directory verb reachable by a stranger, it adds the first exception *with the wire-reachable verb in hand*, and the helper is where it goes. |
| **D-C5-4** | **`visibility` in the manifest is left exactly as C4 shipped it.** No service is narrowed or widened. | With `D-C5-3` in force, visibility is a discoverability choice, not an authorization control. Churning it would hide which mechanism is actually doing the work and would invite a later reader to believe a manifest field is protecting the API. Said out loud in `status.md` rather than left for someone to infer from a diff. |
| **D-C5-5** | **The listing is one record type, `listing`, at version 1, whose payload is a small required core plus seven optional named blocks — `booking`, `payment`, `product`, `service`, `location`, `relationship`, `service_record`.** Not seven record types and not seven listing kinds. An edit produces a new envelope carrying `supersedes`; nothing is rewritten. | The spec's Records table has exactly one `listing` row, signed by the provider, and the R1 row calls them *"dimensions"* of one schema. Seven types would need seven rows in that table and seven entries in `record::RECORD_TYPES`. Optional blocks also mean a provider selling a product and a provider selling a service produce the same record type with different blocks filled — which is what lets C6 index one shape. |
| **D-C5-6** | **Money is integer minor units with an explicit ISO-4217 currency; geography is integer micro-degrees. No non-integer number appears anywhere in a signed payload.** | `F8`: `RecordDraft::validate` refuses a non-integer number outright, so a decimal price is not a style question — the host will not sign it. Micro-degrees (1e-6 of a degree, ~11 cm) are far finer than any service area needs and keep the value an `i64`. |
| **D-C5-7** | **Roym's own copy of conversation content lives in the `conversation` service's own `data-layer`, holds the message body, and is bounded by the host's own retention caps — so the on-disk cost is 2× the host store, and C5 says so in the product.** No `blob-store` tier. | `D-06C-5` requires the copy, and export, search and delete all act on it, which needs it queryable — `blob-store` is content-addressed bytes with no query surface. `F10` bounds it: 100 000 messages × 256 KiB is the host's own ceiling, and Roym's copy doubles it; at realistic text sizes (~1 KiB) that is ~100 MB per copy per conversation at the cap. Attachments are excluded from the first release by the spec's own scope table, so there are no large bodies to tier out. The honest statement belongs in `profile.policy`'s retention text and in the Hub's Backup screen, not only in this plan. |
| **D-C5-8** | **`listing.set` fills `conversation_address` from the person's own `profile` record when the caller omits it, through a declared `catalog → profile` dependency.** The field is required in the signed payload; only the *typing* is optional. | `F14`: no host surface reports a service its own routing address, so the value is person-supplied once, at `profile.set`, and asking for it a second time per listing is how the two drift apart. A listing found by direct link with no directory in the path must carry it under the provider's own signature — task.md's open design point, answered here as it said C5 would. |
| **D-C5-9** | **C5 declares two `depends_on` edges and no more: `conversation → profile` and `catalog → profile`.** Each is added in the same change as the call that traverses it. | `D-C4-11`'s condition, met. `F11` says resolution already works without the declaration, so the edge buys honesty rather than function — which is exactly why C4 refused to add edges nothing traversed, and why these two are added now that something does. The general enforcement gap keeps its own backlog row. |
| **D-C5-10** | **Delivery state is never Roym's to invent. Roym's copy stores the last state the host told it, and `conversation.history` re-reads `delivery-status` from the host for every row not already `delivered`, persisting what it read.** A message written by `conversation.send` is stored `pending`, from the host's own return value, never optimistically. | R1 row 3's acceptance test is *"never shown as delivered while pending"*, and a cache that can go stale across a missed notification would fail it after a restart. Re-reading only non-`delivered` rows bounds the cost (a delivered message is terminal) and makes the guarantee hold without depending on `on-delivery-state` ever arriving. |
| **D-C5-11** | **A message refused at Roym's inbox is recorded in a separate `refused_messages` collection, body dropped, and is invisible to every verb that reads `messages`.** Block is checked on **every** inbound message; the first-contact rate limit is consulted only when Roym holds no admitted conversation with that peer. | `D-06C-8` requires the message to appear in no conversation, fire no notification, and be counted nowhere. A flag on the `messages` rows would leave every future query one forgotten predicate away from leaking it; a separate collection makes "counted nowhere" structural. Splitting block from the rate limit is forced by the verbs C4 shipped: `contacts.admit-first-contact` consumes a rate-limit budget, so calling it per message would refuse an ongoing conversation. |
| **D-C5-12** | **Parity scenario 5's unbound dependency moves from `conversation` to `transaction`.** | `F13`: C5 binds `conversation`, and the scenario needs a genuinely unbound one. `transaction` has no verbs until C7/C8, so nothing else needs it bound. Recorded here because a reader of scenario 5 will otherwise think the choice was arbitrary. |
| **D-C5-13** | **Every parity scenario C5 adds must assert on at least one value only the verb's own handler can produce, and the suite gains one guard test that drives every verb C5 adds and fails if any of them answers `-32601` or `-32013`.** | `F16`: six existing scenarios pass against verbs with no handler, because they assert values `web` produces before any sibling runs. That is correct for what those six prove and wrong as a template. The guard test is the cheap, general fix: it is one loop over a list the plan already has to write, and it fails loudly the moment a verb is named in a scenario but never implemented. |
| **D-C5-14** | **`listing.set` is the catalog-side caller of `safety::admit_publication`**, with limits stored per installation and readable/settable through `listing.limits` / `listing.set-limits`, mirroring `contacts.limits` / `contacts.set-limits` exactly. | `D-C4-15` said C5 (catalog-side) and C6 (directory-side) call it, and a provider minting listing versions in a loop is the flooding this bounds on the provider's own node. C6 adds the directory-side caller against the same function, which is what `[PRD-SAF]` asked be fixed once. Mirroring the contacts shape means one pattern, not two. |
| **D-C5-15** | **A listing's identity is content-derived from `(issuer, slug)` and carries no clock: `listing_id = content_digest("lst_", {"issuer": <owner did>, "slug": <slug>})`.** The slug is provider-chosen or derived from the title; a second `listing.create` for the same slug is the *edit* path, not a duplicate. | `D-C4-14`'s reasoning for `report_id`, applied again: a clock in an identifier makes the same content produce a different id on every call, and makes the two builds' values incomparable (`D-C4-12`). Deriving from the first version's `record_id` was the alternative and fails, because a payload cannot contain its own record id. |
| **D-C5-16** | **Availability is unsigned app state, not a record type.** `availability.*` stores explicit slots with integer second bounds and a capacity; no recurrence rules. | The spec's Records table has no availability row, so signing one would invent a tenth-plus record type nobody asked for. Recurring bookings are excluded by R2's own scope row, so explicit slots are the whole requirement. Booking a slot is C8's; C5 stores what a provider offers and nothing decides anything with it yet. |
| **D-C5-17** | **`conversation.export` / `conversation.import` mirror `profile.export` / `profile.import` exactly, over the same `Bundle` type, with sections `conversations` and `messages`. Composition across services stays C8's.** | `D-C4-8` already assigned the top-level composition and the signed manifest to C8. A cross-service composition now would need `profile → conversation` on top of `conversation → profile`, i.e. a cycle in the declared graph, to gain nothing C8 will not rebuild. Two symmetric bundles is the shape C8 composes. |
| **D-C5-18** | **The shared `handle_certificate_verb` is mounted on `catalog` and `conversation` in this slice, and on `directory`/`transaction` when those services first sign something.** | C4 §16 blocked this on wire-side authorization, which `D-C5-1`/`D-C5-3` now supply. `catalog` signs listings and needs a certificate; `conversation` signs nothing in C5, but mounting it there keeps the enrolment ceremony one command for the whole app rather than a list that grows per slice. `directory` and `transaction` are left alone because a verb that no flow exercises is what `D-C1-10` refuses everywhere else. |

---

## §3 `syneroym:invocation` — who is on the other end (`D-C5-1`)

The only new host interface in this slice. It is additive and, following
`syneroym:http`, `syneroym:conversation` and `syneroym:signing`, is
**not** added to the `host-environment` world by default: a component that
does not import it deploys exactly as before.

### 3.1 `crates/wit_interfaces/wit/invocation/invocation.wit` — new package

```wit
package syneroym:invocation@0.1.0;

/// Who is on the other end of the call a component is currently handling.
/// `syneroym:http`'s `caller-identity` answers this for an inbound HTTP
/// request only. Every other way into a component -- a sibling's proxy
/// call, an inbound JSON-RPC stream naming an exported interface -- had no
/// answer at all, so a component could not tell a call made from inside
/// its own node from one that arrived over the network.
interface invocation {
    /// What the host can honestly say. Every arm is decided by the
    /// dispatch path the call took and by the router's own verification.
    /// Nothing in a request body can influence it, and no arm is a claim
    /// the caller made about itself.
    variant caller-origin {
        /// The call arrived through a local dispatch path: another
        /// service of this node, or this node's own machinery. There is
        /// no attributable identity here and none is claimed -- an
        /// internal dispatch is trusted because of where it came from,
        /// never because of who it says it is.
        internal,
        /// The call arrived over the network under an identity the router
        /// verified -- a delegation certificate, a session token, or a
        /// UCAN chain that verified, was unrevoked, and carried a
        /// capability. The string is that party's `did:key`.
        verified(string),
        /// The call arrived over the network with nothing usable on the
        /// connection. Pseudonymous at best; treat as a stranger.
        anonymous,
    }

    /// Never fails: a call being handled always arrived somehow.
    caller: func() -> caller-origin;
}

/// This package declares no guest export, so one world serves the guest
/// and host views alike -- the same shape `signing-import` has, and for
/// the same reason.
world invocation-import {
    import invocation;
}
```

**Why not reuse `syneroym:http`'s `caller-identity`.** It carries
`caller-auth` and `app-instance`, neither of which an authorization rule
here can use, and importing types from an interface that is declared in the
export direction is what produced the `conversation-import` /
`data-layer-import` finding — a consumer inherits the export requirement
into its own component-type section and encoding fails. A three-arm variant
with one string is smaller than the conversion code reuse would need.

### 3.2 `crates/sandbox_wasm` — the origin and the host implementation

Three changes, each small.

**(a) `HostState` gains one field.** Beside `caller`
(`host_capabilities.rs:250`):

```rust
/// Where this invocation entered the node, which the caller alone cannot
/// say: a sibling's proxy call and an unauthenticated inbound stream both
/// arrive with the synthesized `service_system` caller, so the two are
/// indistinguishable from `caller` by construction.
pub invocation_origin: InvocationOrigin,
```

with `InvocationOrigin { Local, Wire }` defined next to it, defaulting to
`Local` — every constructor that does not name one is a host-driven path
(lifecycle hooks, `notify_guest_message`, the stage-4 after-step).

**(b) `AppSandboxEngine` gains one entrypoint** (`D-C5-2`):

```rust
/// A call that arrived over the network. The only caller is the router's
/// own JSON-RPC dispatch; every other path into a component originates on
/// this node.
pub async fn execute_wasm_json_from_wire(
    &self,
    service_id: &str,
    interface: &str,
    request: &JsonRpcRequest,
    caller: Option<CallerContext>,
) -> Result<Value>
```

`execute_wasm_json` keeps its signature and its meaning (local); both
delegate to one private function taking the origin, which passes it to
`prepare_wasm_execution` → `build_store_and_instantiate` → `HostState`.

**(c) The `invocation::Host` implementation**, beside `signing::Host`:

```rust
impl invocation::Host for HostState {
    async fn caller(&mut self) -> WitCallerOrigin {
        match self.invocation_origin {
            InvocationOrigin::Local => WitCallerOrigin::Internal,
            InvocationOrigin::Wire => match self.caller.auth {
                AuthLevel::Delegated | AuthLevel::Ucan => {
                    WitCallerOrigin::Verified(self.caller.caller_did.clone())
                }
                _ => WitCallerOrigin::Anonymous,
            },
        }
    }
}
```

The inner match is `caller_binding`'s rule (`host_capabilities.rs:581`),
deliberately: one definition of "the router verified this party", used
twice.

**(d) One changed line** at `crates/router/src/route_handler/dispatch.rs:242`:
`execute_wasm_json(...)` → `execute_wasm_json_from_wire(...)`. The
`TODO(B7b / post-B7)` comment above it gains a sentence saying a component
can now make this decision for itself, and that the host-side Tier-1 gate
is still absent.

### 3.3 `crates/app_host` — the trait

```rust
/// Mirrors `syneroym:invocation/invocation@0.1.0`. One function, and the
/// only host surface outside inbound HTTP that says anything about who is
/// calling.
pub trait AppInvocation {
    fn caller(&self) -> impl Future<Output = CallerOrigin> + Send;
}
```

`CallerOrigin` in `types::invocation`. `AppHost`'s supertrait list grows
from nine to ten — a breaking change to the bound, with the same two
in-tree implementors C1 counted (`GuestHost`, `NativeAppHost`), named here
rather than discovered in review. `guest.rs` implements it over the
`wit-bindgen` binding; `crates/app_host_native/src/host.rs` implements it
over the `CallerContext` the factory already holds:

```rust
impl AppInvocation for NativeAppHost {
    async fn caller(&self) -> CallerOrigin {
        // A natively linked service is registered only in the local
        // registry and is never published, so nothing can reach it from
        // the network: every call it sees originated on this node. The
        // caller's own auth level is still consulted rather than assumed,
        // so a future native ingress cannot silently read as internal.
        match self.caller_auth() {
            AuthLevel::Delegated | AuthLevel::Ucan => {
                CallerOrigin::Verified(self.caller_did().to_string())
            }
            _ => CallerOrigin::Internal,
        }
    }
}
```

`CallerOrigin::Anonymous` is therefore unreachable on the native build, and
that is not a divergence — §14 item 4 records why, in the same terms C4's
§13 item 3 used for the native-dispatch gate.

### 3.4 Worlds

All five non-`web` service worlds and `web`'s gain
`import syneroym:invocation/invocation@0.1.0;`, plus a
`wit/deps/invocation/` copy and a `[package.metadata.component.target.dependencies]`
entry per crate. `test-components/dual-build-fixture`'s world gains it too,
with one `run` op returning the arm it sees — the shim's own rule is that a
trait with only one of its two implementations exercised is how the native
build becomes second-class (`D-06B-3`, `D-06C-10`).

`crates/wit_interfaces/Cargo.toml` gains an `invocation` feature and
`src/lib.rs` a `bindgen!` module, following `signing`'s entry exactly.

### 3.5 Tests for §3

| Test | Proves |
|---|---|
| `crates/sandbox_wasm` unit | `execute_wasm_json` yields `Internal`; `execute_wasm_json_from_wire` with a `Delegated` caller yields `Verified(did)`, with `None` yields `Anonymous`, and with a `System` caller yields `Anonymous` — a substrate-injected level must never read as verified on a wire path. |
| `crates/app_host_native` unit | The native mapping, all five `AuthLevel` arms. |
| `crates/app_host_native/tests/dual_build_parity.rs` | The fixture's new `run` op returns the same arm on both builds for a local call and for a verified call. |

---

## §4 `syneroym-roym-core` — the shared vocabulary

`syneroym-roym-core` may depend on `syneroym-app-host`,
`syneroym-signed-record`, `serde`, `serde_json`, `async-trait` and
`thiserror`, and nothing else (`xtask/src/main.rs:71`). Everything below
stays inside that list; `cargo xtask check-roym-deps` is a step in §15.

### 4.1 `crates/roym_core/src/admit.rs` — new file (`D-C5-3`)

```rust
//! One admission rule for every service behind the entrypoint.
//!
//! The person's browser reaches this app through one service, which
//! checks its own session before it forwards anything. No other ingress
//! is legitimate, and the address a person publishes so others can
//! message them is the same string that addresses this service's API --
//! so "reachable" and "meant for you" are different questions, and only
//! the host can answer the first.

pub const NOT_LOCAL: i64 = -32013;

/// `None` admits. `Some(response)` is the refusal to return unchanged.
/// Deliberately not a `bool`: a refusal that a caller has to remember to
/// turn into a response is a refusal somebody eventually forgets.
pub async fn require_internal<H: AppHost>(host: &H) -> Option<Response> {
    match AppInvocation::caller(host).await {
        CallerOrigin::Internal => None,
        // The refusal names no DID and no service: a stranger learns only
        // that this method is not theirs to call.
        CallerOrigin::Verified(_) | CallerOrigin::Anonymous => Some(Response::err(
            NOT_LOCAL,
            "this method is reachable only from inside this installation",
        )),
    }
}
```

Unit tests over a `#[cfg(test)]` `AppHost` stub — the module
`roym_core::signing` already introduced for its own tests, extended with a
settable `CallerOrigin` rather than a second stub.

### 4.2 `crates/roym_core/src/area.rs` — new file (`D-C5-6`)

Three area shapes, one bounding-box projection, all integers.

```rust
/// Where a listing applies. Micro-degrees (1e-6 deg, about 11 cm) because
/// a signed payload may hold no number that is not an integer: the
/// canonical encoding is only reproducible for integers, so the host
/// refuses a decimal before it signs.
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Area {
    /// What an index wants.
    Bbox { min_lat_e6: i64, min_lon_e6: i64, max_lat_e6: i64, max_lon_e6: i64 },
    /// What a provider actually thinks in: "I travel this far from here".
    Circle { lat_e6: i64, lon_e6: i64, radius_m: u64 },
    /// What a person reads. Never queried geometrically.
    Named { label: String, code: Option<String> },
}

pub struct BoundingBox { pub min_lat_e6: i64, pub min_lon_e6: i64, pub max_lat_e6: i64, pub max_lon_e6: i64 }

/// `None` for `Named`, which has no geometry. One definition, so the
/// service that publishes an area and the service that indexes it cannot
/// disagree about what it covers.
pub fn bounding_box(area: &Area) -> Option<BoundingBox>
```

`Circle` → box uses an integer latitude/longitude degree-per-metre
approximation that **over**-covers, never under-covers, so an index built
on it can return a false positive that a later exact check drops, and can
never miss a match. Validation: latitude in `[-90e6, 90e6]`, longitude in
`[-180e6, 180e6]`, `min <= max`, `radius_m` at most 40 075 000, at most 8
areas per listing.

### 4.3 `crates/roym_core/src/listing.rs` — new file (`D-C5-5`, `D-C5-6`)

One required core and seven optional blocks. Every block is `Option`, and
`#[serde(skip_serializing_if = "Option::is_none")]` throughout, so an
absent block contributes no bytes to the signature — a provider who fills
in three blocks and one who fills in three and explicitly nulls four
produce the same canonical payload.

```rust
pub const LISTING_VERSION: u32 = 1;
pub const MAX_TITLE_LEN: usize = 160;
pub const MAX_SUMMARY_LEN: usize = 2048;
pub const MAX_CATEGORIES: usize = 8;
pub const MAX_AREAS: usize = 8;

pub struct ListingPayload {
    /// Stable across every version of this listing, content-derived from
    /// (issuer, slug). A new version supersedes the previous envelope and
    /// keeps this value.
    pub listing_id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    /// Free-form category tokens, lowercase, `[a-z0-9-]`. What a search
    /// filters on; the vocabulary is the group's, not the product's.
    pub categories: Vec<String>,
    /// Under the provider's own signature, so a stranger who verifies
    /// this listing can start a conversation with no directory and no
    /// prior contact entry.
    pub conversation_address: String,
    pub status: ListingStatus,          // draft | active | withdrawn

    pub booking: Option<BookingTerms>,
    pub payment: Option<PaymentTerms>,
    pub product: Option<ProductDetail>,
    pub service: Option<ServiceDetail>,
    pub location: Option<LocationTerms>,
    pub relationship: Option<RelationshipTerms>,
    pub service_record: Option<ServiceRecordTerms>,
}
```

The seven blocks, each validated, each carrying only integers and strings:

| Block | Fields | Notes |
|---|---|---|
| `booking` | `mode` (`slots` \| `order` \| `enquiry`), `lead_time_secs`, `cancellation_window_secs`, `max_per_booking` | `mode` decides whether `availability.*` means anything for this listing. |
| `payment` | `currency` (ISO-4217, 3 uppercase letters), `model` (`fixed` \| `per-hour` \| `per-unit` \| `quote-only`), `amount_minor`, `tax_included`, `fees_minor`, `methods` (list of tokens), `payee` | `amount_minor` absent for `quote-only`. `payee` is a free string here and becomes binding only inside an `agreement-receipt` (C7/C8) — the UI must never present a listing's payee as agreed terms. |
| `product` | `unit`, `pack_size`, `condition` (`new` \| `used` \| `refurbished`), `sku` | No stock counting: excluded by R1's own row. |
| `service` | `duration_secs`, `includes` (list), `excludes` (list), `prerequisites` (list) | |
| `location` | `where_` (`at-provider` \| `at-customer` \| `remote`), `service_area: Vec<Area>`, `address_disclosure` (`on-agreement` \| `public`) | `address_disclosure` carries the spec's C12 rule (*"the exact address is disclosed only when it is needed"*) as data rather than as UI convention. No street address is ever in a listing. |
| `relationship` | `open_to` (`anyone` \| `members` \| `referral` \| `existing-customers`), `member_of` (DID of the group, when `members`) | Stating the rule, not enforcing it — enforcement needs the membership credential, which is C9's. The UI says which. |
| `service_record` | `issues_fulfilment_receipt` (bool), `warranty_secs`, `retention_secs` | The spec's *"every durable record has a stated owner, retention policy, and deletion behaviour"*, made a field of the offer. |

`ListingPayload::validate(&self) -> Result<(), ListingError>` checks every
length, every enum, `slug` shape (`[a-z0-9-]`, 1..=64), category count and
shape, `Area::validate` per area, `currency` shape, and that `payment` is
present unless `booking.mode == enquiry`. Sixteen error variants, one per
rule, each with a unit test at and around its boundary.

`pub fn derive_listing_id(issuer: &str, slug: &str) -> Result<String, ...>`
— `content_digest("lst_", json!({"issuer": issuer, "slug": slug}))`
(`D-C5-15`).

### 4.4 `crates/roym_core/src/conversation.rs` — new file (`D-C5-7`, `D-C5-10`)

The row types for Roym's own copy, and the one ordering rule.

```rust
pub const CONVERSATION_SCHEMA_VERSION: u32 = 1;

pub struct ConversationRow {
    pub id: String,               // the host's own conversation-id
    pub peer_address: String,
    pub peer_person_did: Option<String>,
    pub opened_at_secs: u64,
    pub last_activity_ms: i64,    // from the newest message's sender timestamp
    pub message_count: u64,
}

/// What the host said, never what this service hoped. `Pending` is what a
/// freshly sent message is, from the host's own return value.
#[serde(rename_all = "kebab-case")]
pub enum StoredState { Pending, Delivered, Failed }

pub struct MessageRow {
    pub id: String,               // the host's own message-id
    pub conversation: String,
    pub author: String,
    pub direction: Direction,     // incoming | outgoing
    pub sender_timestamp_ms: i64,
    pub content_type: String,
    /// `Utf8` for a text content type, `Base64` otherwise. Two encodings
    /// and a discriminator, so a search knows which rows it can read.
    pub body_encoding: BodyEncoding,
    /// Absent once deleted. The row itself is the durable deletion
    /// record: the spec's rule is that deleting removes the local copy
    /// and writes a record, not that the row disappears.
    pub body: Option<String>,
    pub state: StoredState,
    pub last_error: Option<String>,
    pub deleted_at_secs: Option<u64>,
    pub stored_at_secs: u64,
}

/// ADR-0013 section 5's rule, applied to this service's own copy so the
/// order a person sees here is the order the host would give and the
/// order every other participant computes.
pub fn sort_key(m: &MessageRow) -> (i64, &str, &str) {
    (m.sender_timestamp_ms, m.author.as_str(), m.id.as_str())
}
```

Unit tests: `sort_key` orders identically for messages that arrived in the
opposite order; a deleted row keeps its id, timestamp and author and loses
its body; `StoredState` round-trips through the WIT `DeliveryState`
conversion both ways.

### 4.5 `crates/roym_core/src/backup.rs` — two new section names

```rust
pub const SECTION_CONVERSATIONS: &str = "conversations";
pub const SECTION_MESSAGES: &str = "messages";
```

Nothing else in that file changes: `Bundle`, `BundleManifest`,
`SectionDigest` and `check_integrity` are already section-agnostic, which
is what `D-C4-8` built them for. One unit test asserting a bundle carrying
only the two new sections passes `check_integrity`, and one asserting a
manifest that declares `messages` with no content is `MissingSection`.

### 4.6 `crates/roym_core/src/router.rs` — no new prefixes

`conversation.`, `listing.` and `availability.` are already routed and
already classified `Owner` (`ROUTES`, `crates/roym_core/src/router.rs:22`).
C5 adds no prefix and changes no classification. The existing tests
(`every_route_prefix_has_an_auth_classification`,
`reachable_services_set_equals_siblings`) keep passing untouched, and one
new test asserts `PUBLIC_METHODS` is still exactly `["profile.policy"]` —
so a slice that wants a second public method has to say so.

### 4.7 `crates/roym_core/src/record.rs` — unchanged

`listing` is already in `RECORD_TYPES` at version 1. C5 adds
`pub const RECORD_LISTING: &str = "listing";` beside `RECORD_PROFILE` and
nothing else.

### 4.8 `crates/roym_core/src/lib.rs`

`pub mod admit; pub mod area; pub mod conversation; pub mod listing;`
added to the existing list.

---

## §5 `syneroym-roym-catalog` — the provider's offer

`SCHEMA_VERSION` 1 → 2 (`F15`).

### 5.1 Collections

Created idempotently on first use with `ensure_coll`, the pattern C4
established from the fixture's own (`F7` of C4; `create-collection` is
idempotent and indexes are `IF NOT EXISTS`).

| Collection | Id | Indexes | Holds |
|---|---|---|---|
| `listings` | `listing_id` | `status` (string), `updated_at_secs` (numeric) | The current version: `{ envelope, record_id, listing_id, slug, status, updated_at_secs }` |
| `listing_history` | `record_id` | — | Every version's envelope, verbatim. Append-only. |
| `availability` | `slot_id` | `listing_id` (string), `start_secs` (numeric) | `{ slot_id, listing_id, start_secs, end_secs, capacity }` |
| `publications` | `{listing_id}:{n}` | `at_secs` (numeric) | One row per signed listing version, for `admit_publication`'s window |
| `settings` | fixed keys | — | `publication_limits` |
| `signing_certificates` | `current` | — | Owned by `roym_core::signing`, created by it (`D-C5-18`) |

### 5.2 Verb table

Every verb behind `admit::require_internal` (`D-C5-3`). All are `Owner` in
`web`'s table already.

| Method | Params | Returns |
|---|---|---|
| `listing.ping` | — | unchanged |
| `listing.set` | `slug?`, `title`, `summary`, `categories`, `conversation_address?`, `status?`, the seven optional blocks | `{ listing_id, record_id, version_count }` |
| `listing.get` | `listing_id` | `{ envelope, record_id, listing_id, status, updated_at_secs }` or `null` |
| `listing.list` | `status?`, `offset?`, `limit?` | listing rows |
| `listing.history` | `listing_id` | every envelope, oldest first |
| `listing.withdraw` | `listing_id` | a new signed version with `status = withdrawn` |
| `listing.verify` | `envelope` | a stranger's listing, verified locally: `{ verified, listing_id, issuer, conversation_address, reason? }` |
| `listing.limits` | — | `PublicationLimits` |
| `listing.set-limits` | `window_secs`, `max_per_window` | the stored limits |
| `availability.set` | `listing_id`, `slots: [{start_secs, end_secs, capacity}]` | `{ listing_id, slot_ids }` |
| `availability.list` | `listing_id`, `from_secs?`, `to_secs?` | slot rows, ordered by `start_secs` |
| `availability.remove` | `slot_id` | `{ removed }` |
| `catalog.signing-status`, `catalog.install-signing-certificate` | via `handle_certificate_verb` | `D-C5-18` |

`listing.verify` earns its place: it is the *"the consumer's own node
verifies, never the directory"* rule as a verb, it is what the no-directory
path (`D-06C-6a`) uses when a provider hands over a listing envelope by
direct link, and it is what C6 will call on every search result. It takes
an envelope and does not store it — storing a stranger's listing is C6's,
with a source and a freshness to record alongside.

### 5.3 `listing.set` — the one flow that signs

```
now      = clock::now_secs()
refuse unless the invocation is internal
owner    = signing::owner_did(host)?                       -> -32602 if none
principal, _ = signing::person_principal(host, now)?       -> "signing-not-enrolled" | expired | stale

slug     = params.slug  or  slug_from_title(params.title)
listing_id = listing::derive_listing_id(&owner, &slug)

address  = params.conversation_address
           or  the conversation_address inside this person's own profile
               record, fetched through Dependency("profile") -> profile.get {}   (D-C5-8)
           or  -32602 "conversation_address is required and no profile record carries one"

payload  = ListingPayload { listing_id, slug, ... }
payload.validate()?                                        -> -32602, naming the rule

prior    = every publications row inside the window
match safety::admit_publication(&prior, &limits, now) {     (D-C5-14)
    Allow                       => (),
    RateLimited { retry_after } => return -32602 with { admission, retry_after_secs },
    Blocked                     => unreachable: admit_publication never blocks,
                                   and the match arm says so rather than
                                   collapsing into a catch-all
}

supersedes = the current row's record_id, if any
envelope   = AppSigning::sign_record(draft{ version: 1, "listing", subject: listing_id,
                                            payload, supersedes }, principal)
refuse unless envelope.issuer == owner   -- the host signed under an issuer
                                            this service did not ask for

put listings/<listing_id>          (the pointer first: a crash between the two
                                    writes leaves the pointer on the previous
                                    valid version, and an unreferenced history
                                    row is harmless -- profile.set's own rule)
put listing_history/<record_id>
put publications/<listing_id>:<n>
```

`subject` is the `listing_id`, not the owner: the spec's Records table says
a `listing` proves *"who published this offer, and when"*, and the issuer
already carries who. `slug_from_title` lowercases, keeps `[a-z0-9]`,
collapses runs to `-`, trims, and truncates to 64 bytes; an empty result is
`-32602` rather than a generated identifier nobody can type.

### 5.4 `listing.verify`

```
verified = signed_record::verify_json(&envelope, &VerifyOptions::new(now))
refuse if record_type != "listing" or version != 1
parse ListingPayload; validate it
refuse if payload.listing_id != derive_listing_id(&verified.issuer, &payload.slug)
      -- the id must be derivable from the signature's own issuer, so a
         listing cannot claim another provider's identifier
return { verified: true, listing_id, issuer, conversation_address, status }
```

A failure returns `{ verified: false, reason }` inside a **success**
envelope, not an error: "this listing does not verify" is an answer, and
`D-06C-6c` requires that answer to be renderable as *unknown*, never as a
transport failure the UI might retry past.

---

## §6 `syneroym-roym-conversation` — the product's inbox and its own copy

`SCHEMA_VERSION` 1 → 2. This is the largest part of the slice.

### 6.1 Collections

| Collection | Id | Indexes | Holds |
|---|---|---|---|
| `conversations` | host `conversation-id` | `last_activity_ms` (numeric) | `ConversationRow` |
| `messages` | host `message-id` | `conversation` (string), `sender_timestamp_ms` (numeric), `state` (string) | `MessageRow` |
| `refused_messages` | host `message-id` | `at_secs` (numeric) | `{ id, conversation, author, reason, at_secs }` — **no body** (`D-C5-11`) |
| `signing_certificates` | `current` | — | `roym_core::signing`'s, mounted per `D-C5-18` |

### 6.2 The world and the sinks (`F7`)

`crates/roym_conversation/wit/world.wit` gains
`export syneroym:conversation/guest-api@0.1.0;` beside its existing
`export api;`, and `import syneroym:invocation/invocation@0.1.0;`.
`src/guest.rs` implements `ConversationGuestApiGuest` exactly as the
fixture does (`test-components/dual-build-fixture/src/guest.rs:98`), and
`src/native.rs` implements `ConversationSink`, building its host with
`CallerContext::service_system(&self.service_id)` — the same identity the
WASM delivery path uses, so an elevated caller cannot arrive with a
delivered message.

`src/bindings.rs` is checked in and regenerates from the world; the
regenerated file is part of the commit.

Two wiring lines, one per stack:

- `crates/substrate/src/runtime.rs`, in `init_roym`, beside the existing
  `factories.push(factory_conv)`:
  `factory_conv.set_conversation_sink(Arc::downgrade(&conv) as Weak<dyn ConversationSink>);`
- `crates/roym_web/tests/dual_build_parity.rs`: the same on the native
  stack, plus `wasm_conversation.set_notifier(Arc::downgrade(&wasm_engine) as Weak<dyn ConversationNotifier>)`
  on the WASM stack, which the harness does not do today and which
  `crates/app_host_native/tests/dual_build_parity.rs:696` already models.

### 6.3 `on_message` — Roym's inbox, and where block takes effect (`D-C5-11`)

One target-independent function, called from the guest export on WASM and
from `ConversationSink::on_message` natively.

```
on_message(host, msg):
  now = clock::now_secs()

  # Who is this, as far as this node can say? The host names a service
  # address; the person behind it is this product's own mapping.
  person_did = contacts row whose conversation_address == msg.author, via
               Dependency("profile") -> contacts.list                (D-C5-9)

  # Block is checked on every message, not only the first: a person who
  # blocks somebody mid-conversation means it from that moment on.
  if profile -> block.check { address: msg.author, person_did } is blocked:
      record refused_messages/<msg.id> { reason: "blocked" }         # no body
      return Ok(())

  # The rate limit is a *first contact* limit, and calling the verb that
  # enforces it consumes a budget -- so it is consulted only when this
  # node holds no conversation with this peer yet.
  if conversations has no row for msg.conversation:
      match profile -> contacts.admit-first-contact { sender_address, sender_person_did }:
          allow        => ()
          blocked      => record refused; return Ok(())
          rate-limited => record refused { reason: "rate-limited" }; return Ok(())

  upsert conversations/<msg.conversation>
  put    messages/<msg.id>  { direction: incoming, state: delivered, body }
  Ok(())
```

Three things this deliberately does **not** do, each following
`D-06C-8`:

- It never tells the sender. The host stored and acknowledged the message
  before any of this code ran (`guest-api`'s own WIT says so), so there is
  nothing left to refuse. The failure-and-security matrix's row 12
  (*"refusal is visible to the sender"*) is met on the synchronous path
  C4 shipped, and is **not** met here — stated in §17 and in the backlog
  row for the absent admit hook, not quietly claimed.
- It never deletes from the host store, because no verb can.
- It returns `Ok(())` for a refusal. An `Err` would make the host log a
  delivery failure and, on WASM, retry four times — turning a deliberate
  product decision into a repeated apparent fault.

`on_delivery_state(host, message_id, state)` updates the `messages` row's
`state` and `last_error`, or does nothing if Roym holds no such row.

### 6.4 Verb table

Every verb behind `admit::require_internal`.

| Method | Params | Returns |
|---|---|---|
| `conversation.ping` | — | unchanged |
| `conversation.open` | `person_did?` \| `address?` | `{ conversation_id, peer_address }` |
| `conversation.list` | `offset?`, `limit?` | `ConversationRow`s, newest activity first |
| `conversation.send` | `conversation`, `body`, `content_type?` | `{ message_id, state: "pending" }` |
| `conversation.history` | `conversation`, `limit?`, `cursor?` | messages from Roym's copy, in `sort_key` order, states reconciled (`D-C5-10`) |
| `conversation.delivery-status` | `message_id` | straight from the host, never from Roym's copy |
| `conversation.outbox` | — | the host's outbox |
| `conversation.retry` | `message_id` | host `retry` |
| `conversation.delete-message` | `message_id` | `{ deleted, note }` — body dropped, row kept |
| `conversation.search` | `query`, `conversation?`, `limit?` | matching messages from Roym's copy |
| `conversation.export` / `conversation.import` | as `profile.*` (`D-C5-17`) | a `Bundle` over `conversations` + `messages` |
| `conversation.signing-status`, `conversation.install-signing-certificate` | via `handle_certificate_verb` | `D-C5-18` |

`conversation.open` with a `person_did` resolves the address through
`Dependency("profile")` → `contacts.resolve-address`, which is the verb C4
shipped for exactly this and has had no caller until now.

### 6.5 `conversation.send` — never optimistic

```
refuse unless internal
message_id = AppConversation::send(host, conversation, content_type, body)?
             # the host writes durably and returns `pending`; it never
             # touches the network on this call
state      = AppConversation::delivery_status(host, message_id)?
             # read back rather than assumed: the state this row is born
             # with is the host's answer, not this service's hope
put messages/<message_id> { direction: outgoing, state, body }
return { message_id, state }
```

### 6.6 `conversation.history` — the reconciliation (`D-C5-10`)

```
rows = query messages { conversation } ordered by sender_timestamp_ms
for row in rows where row.state != Delivered and row.deleted_at_secs.is_none():
    live = AppConversation::delivery_status(host, row.id)?
    if live != row.state: row.state = live; put it back
sort by sort_key; page; return
```

A `Delivered` row is terminal, so the reconciliation cost is bounded by
the number of messages still in flight, not by history length. This is what
makes *"never shown as delivered while pending"* hold after a restart even
if an `on-delivery-state` notification was never delivered, which is the
half of R1 row 3 a cache alone would fail.

### 6.7 `conversation.delete-message` — the spec's deletion rule, verbatim

```
row = messages/<id>  -> -32602 if absent
row.body = None; row.deleted_at_secs = Some(now)
put it back
return { deleted: id, note: "The local copy is removed and a deletion record kept. \
         The other party's copy is theirs; this cannot remove it, and this \
         installation's own message store still holds what it received." }
```

The `note` is asserted as a string in the browser suite, the same way C4
pinned the block wording, so it cannot drift into a promise the product
cannot keep.

### 6.8 `conversation.search`

`$regex` over `body` restricted to rows whose `body_encoding` is `utf8`
and whose `deleted_at_secs` is absent (`F9`). The query string is escaped
for regex metacharacters before it goes into the filter — a person typing
`(` is searching for a bracket, not writing a pattern. FTS5 stays C6's, and
`conversation.search`'s response carries no ranking, only matches in
`sort_key` order.

---

## §7 The other four services

- **`roym_profile`** — no new collection, no `SCHEMA_VERSION` bump (`F15`),
  so no C4-produced bundle is invalidated. Two changes only:
  `admit::require_internal` at the top of `invoke`, and one sentence added
  to `profile.policy`'s retention text stating the duplication `D-C5-7`
  costs, in plain words: *"This installation keeps its own copy of every
  message it sends and receives, separate from the copy the substrate
  keeps for delivery. That is what an export, a search, and a delete act
  on, and it means each message is stored twice on this machine."*
- **`roym_transaction`**, **`roym_directory`** — `admit::require_internal`
  and the new world import; no verbs, no state, no certificate mount
  (`D-C5-18`).
- **`roym_web`** — `admit::require_internal` on its `api.invoke` (nothing
  legitimate calls it from the wire; its real ingress is
  `incoming-handler`, which is untouched and keeps `MethodAuth`). One
  parity scenario proves the HTTP path is unaffected.

## §8 The manifest

`crates/roym_core/app/roym.toml`, three lines:

```toml
[services.conversation]
depends_on = ["profile"]      # D-C5-9: the inbox asks whether a sender is blocked

[services.catalog]
depends_on = ["profile"]      # D-C5-9: a listing carries the person's conversation address
```

No `visibility` or `topology_visibility` value changes (`D-C5-4`).
`roym_core::router`'s `manifest_depends_on_equals_siblings` test reads
`web`'s array only and is unaffected; one new test asserts that every
service declaring `depends_on` names only services in `SIBLINGS`, and that
`conversation` and `catalog` each declare `profile`.

`init_roym` registers bindings for `web` alone today
(`save_binding(&web_id, ...)`, `crates/substrate/src/runtime.rs`); it grows
the same two edges so the native build's persisted bindings match the
manifest. `F11` means resolution already worked; this makes the two agree.

---

## §9 `roymctl`

| Command | Why |
|---|---|
| `roym enrol-signing` — extended | It enrols `profile` only today (`prefix = "profile"`, `apps/roymctl/src/commands/roym.rs`). With `D-C5-18` it must enrol `profile` **and** `catalog` **and** `conversation`, minting one certificate per service against each service's own signing key. It reports one line per service and exits non-zero if any fails. |
| `roym signing-status` — extended | The same three services, one row each. |
| `roym address` — new | Prints this installation's Roym Conversation service id and the gateway host for `web`, so the person can paste the address into `profile.set` without reading a deploy log. It reads the local endpoint registry through the existing `svc` machinery; it invents no resolution path. `F14` is why this exists at all, and the backlog row says so. |

No export/import CLI verb: `D-C5-17` leaves composition to C8, and a
per-service `roymctl` export now would be the composition surface C8 has to
replace.

---

## §10 The Hub

Three additions to the existing three-state shell
(`crates/roym_web/ui/src/main.ts`), plus the two screens the `[PRD-SAF]`
row deferred to C5/C6.

| Screen | Contents |
|---|---|
| **Messages** (new tab) | Conversation list from `conversation.list`; a thread view from `conversation.history` with each message's state shown as the word the API returned (`pending` / `delivered` / `failed`) and never inferred; a compose box calling `conversation.send`; a search box calling `conversation.search`; a delete action showing `delete-message`'s `note` verbatim before it acts. A `failed` message offers `conversation.retry`. |
| **Listings** (new tab) | `listing.list`, an editor over the seven blocks (each block collapsed until the provider opens it, so a service provider never sees product fields), `listing.set`, `listing.withdraw`, `listing.history` showing every version with its `record_id` and what it superseded, and an availability editor for `booking.mode == slots`. A "check a listing someone sent me" box calling `listing.verify`. |
| **Safety** (existing, extended) | The report form (`report.create`, `report.list`, `report.withdraw`) and the contact-limit editor (`contacts.limits` / `contacts.set-limits`) the `[PRD-SAF]` row deferred here. |
| **Backup** (existing, extended) | Both bundles — `profile.export` and `conversation.export` — shown separately, with one sentence saying the two are exported separately today and that a single signed bundle comes later. |

Every value from a listing, a profile, or a message body is inserted as a
text node, never as markup — the rule `D-06C-3` fixed for cards, applied to
every field a stranger can influence. `rpc.ts` gains `-32013` in its typed
error mapping, rendered as *"this installation refused a request that did
not come from you"*.

---

## §11 Tests

### 11.1 What each suite is for

| Suite | Proves |
|---|---|
| `crates/roym_core/src/admit.rs` unit | All three `CallerOrigin` arms, against the `#[cfg(test)]` `AppHost` stub. |
| `crates/roym_core/src/area.rs` unit | Every validation bound; `bounding_box` over-covers a circle and never under-covers; `Named` yields `None`. |
| `crates/roym_core/src/listing.rs` unit | Every one of the sixteen `ListingError` variants at and around its boundary; a payload with every block filled passes `RecordDraft::validate` (so the host would sign it); a payload carrying a float is refused **by `RecordDraft::validate`**, proving `F8` rather than assuming it; `derive_listing_id` is stable and issuer-separated. |
| `crates/roym_core/src/conversation.rs` unit | `sort_key` orders identically from opposite arrival orders; a deleted row keeps id/author/timestamp and loses its body; state conversions round-trip. |
| `crates/roym_core/src/backup.rs` unit | The two new sections through `check_integrity`, including a declared-but-absent section. |
| `crates/sandbox_wasm`, `crates/app_host_native` unit | §3.5. |
| `crates/app_host_native/tests/dual_build_parity.rs` | The fixture's new origin op, both builds. |
| `crates/roym_web/tests/dual_build_parity.rs` | §11.2. |
| `crates/substrate/tests/roym_conversation_e2e.rs` (new) | §11.3 — two substrates, real delivery, a restart on each side. |
| `crates/substrate/tests/roym_app_e2e.rs` | Unchanged assertions; one added step for `listing.set` → `listing.verify`. |
| `crates/roym_web/ui` vitest | The listing block editor's value mapping and `rpc.ts`'s new code. |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | §11.4. |

### 11.2 Parity scenarios

Appended to `crates/roym_web/tests/dual_build_parity.rs`. Three harness
changes first:

1. `conversation` is bound on both stacks; **scenario 5's unbound
   dependency becomes `transaction`** (`D-C5-12`), and its comment is
   rewritten to say why.
2. `set_conversation_sink` on the native `conversation` factory and
   `set_notifier` on the WASM engine (§6.2), so an inbound message reaches
   guest code on both stacks.
3. A driver method that reaches a service **as a wire call** — on WASM
   through `execute_wasm_json_from_wire`, natively through
   `NativeService::dispatch` with a `Delegated` stranger caller — so the
   refusal in `D-C5-3` is proven on both builds and not merely on one.

`strip_volatile` gains `stored_at_secs`, `opened_at_secs`,
`updated_at_secs`, `deleted_at_secs`, `last_activity_ms` and
`sender_timestamp_ms`; the signed listing envelope stays compared byte for
byte, because its timestamp is the host's and is pinned (`D-C4-12`).

| # | Scenario | Assertion |
|---|---|---|
| 37 | `listing.set` with no certificate installed | Same `signing-not-enrolled` refusal on both. |
| 38 | Install the certificate, then `listing.set` with all seven blocks | The two envelopes are **byte-identical**, both verify with `expected_issuer` = the owner, and `listing_id` matches `derive_listing_id`. |
| 39 | `listing.set` twice with a changed `title` | Same `supersedes` chain, same two `record_id`s, same single `listings` row, two `listing_history` rows. |
| 40 | `listing.set` with a float in `payment.amount_minor` | Same `-32602` on both, naming the field. |
| 41 | `listing.set` with `conversation_address` omitted after `profile.set` | Same address on both, taken from the profile record through the declared dependency. |
| 42 | `listing.set` with the address omitted and **no** profile record | Same `-32602` on both. |
| 43 | `listing.set-limits { window_secs: 3600, max_per_window: 2 }`, then four `listing.set` calls | `allow, allow, rate-limited, rate-limited` on both; `retry_after_secs` in `(3590, 3600]` on both. |
| 44 | `listing.withdraw` then `listing.get` | Same `status: "withdrawn"`, a third `record_id`, same chain. |
| 45 | `listing.verify` of scenario 38's envelope | Same `verified: true` and the same `conversation_address` on both. |
| 46 | `listing.verify` of an envelope whose `listing_id` was edited | Same `verified: false` and the same reason on both. |
| 47 | `availability.set` then `availability.list` | Same slot ids and same order on both. |
| 48 | `conversation.open` by address, then `conversation.list` | Same conversation id on both (it is derived from the address pair, not minted). |
| 49 | `conversation.send`, then `conversation.history` | Same `state: "pending"` on both. Never `delivered`. |
| 50 | Deliver an inbound message through the sink, then `conversation.history` | Same body, author and order on both. |
| 51 | Two inbound messages delivered in opposite order on the two stacks | Same `sort_key` order on both — the ordering rule, not the arrival order. |
| 52 | `block.add` for the sender's address, then deliver an inbound message | Same empty `conversation.history` on both, and `conversation.list` shows no new conversation. Proves R1 row 6's inbox half. |
| 53 | Scenario 52's message, then `conversation.search` for its text | No match on both — "counted nowhere" (`D-C5-11`). |
| 54 | Four inbound first contacts from one unknown sender under `max_per_window: 2` | Same two admitted and two refused on both. |
| 55 | `conversation.delete-message` then `conversation.history` | Same row with no body and the same `note` string on both. |
| 56 | `conversation.export` then `check_integrity` | Identical after `strip_volatile`; integrity passes on both. |
| 57 | `conversation.import` of scenario 56's bundle into a second empty stack of the same build | Same section counts and same message order. |
| 58 | `conversation.import` of a bundle with one message edited and the manifest untouched | Same `DigestMismatch` on both. |
| 59 | `catalog.signing-status` / `conversation.signing-status` | Same `missing` then `installed` on both (`D-C5-18`). |
| 60 | **`listing.get` as a wire call with a verified stranger** | Same `-32013` on both. |
| 61 | **`conversation.history` as a wire call with a verified stranger** | Same `-32013` on both. The scenario this slice exists for. |
| 62 | The same two verbs as a local call | Admitted on both — proving 60/61 refuse the *origin* and not the verb. |
| 63 | `POST /rpc` `listing.list` with an owner session | Same 200 and body on both — `web`'s HTTP path is unaffected by §7. |
| 64 | **Guard: every verb this slice adds** driven once with valid params through the local path | No response carries `-32601` or `-32013` on either build (`D-C5-13`). |

Scenario 64 is not a formality. `F16` found six existing scenarios that
would pass against verbs with no handler; this one fails if any verb named
anywhere in 37–63 was never implemented, and it is a single loop over a
literal list.

### 11.3 `crates/substrate/tests/roym_conversation_e2e.rs` — new

Two substrates, the WASM build, copying `conversation_e2e.rs`'s `Node::boot`
/ `deploy` / `teardown` harness and its serial lock. Node A hosts the
registry; both deploy Roym as their own owner identity and enrol signing
from that key.

```
 1. Deploy Roym on A and on B; enrol signing on both.
 2. profile.set on both, each carrying its own conversation service
    address (roymctl roym address supplies it).
 3. A: contacts.upsert for B, using B's verified profile envelope
       -> from_profile_record is set, address taken from the record
 4. A: conversation.open { person_did: B }   -> resolved through contacts
 5. A: conversation.send                      -> state "pending",
       asserted several times over several seconds while B is down
 6. Restart A's substrate and redeploy under the same identity
       -> conversation.history still shows exactly one message, still
          "pending", body intact. Roym's own copy survived, and so did
          the host's outbox.
 7. Bring B up.
 8. Wait until A's conversation.delivery-status reads "delivered", then
    conversation.history on A shows "delivered" and B's shows the message
    with the same body and the same message id.
 9. Restart B and redeploy       -> B's conversation.history is unchanged.
       This is the "on both sides" half of R1 row 3.
10. B: block.add { address: A's conversation address }.
    A: conversation.send again.
       -> the message reaches "delivered" for A (the host stored it; the
          product never claims otherwise), and B's conversation.history
          does not grow, B's conversation.search finds nothing, and B's
          refused row exists with no body.
11. B: listing.set a signed listing carrying B's conversation address.
    A: listing.verify B's envelope   -> verified, and the address it
       returns is the one A can already message. The whole engage-a-
       provider path with no directory anywhere (D-06C-6a's R1 half).
12. A: conversation.export; wipe A's app storage; conversation.import
       -> the same messages, the same order, the same states.
```

Step 10 is the one to read carefully: it is R1 row 6 and `D-06C-8` in one
step, and it asserts what the product *does not* claim as firmly as what it
does.

### 11.4 Browser cases in `roym-hub.spec.ts`

- A listing is created with three blocks filled, reloaded, and shows the
  same values; the editor never sends a decimal.
- The Messages tab shows a `pending` message as pending, and never shows
  the word "delivered" before `delivery-status` says so.
- The delete dialog's text contains `delete-message`'s `note` verbatim and
  contains no claim that the other party's copy is removed — asserted as a
  string, the way C4 pinned the block wording.
- A listing whose `title` contains `<img onerror=…>` renders as text: no
  element created, no request made.
- The Backup tab shows two bundles and the sentence saying so.
- The Safety tab files a report and edits a contact limit.

### 11.5 Failure-and-security-matrix rows C5 closes

| Row | How |
|---|---|
| **1** (a forged or absent listing signature) | `listing.verify` returns `verified: false` with a reason; parity 46; the Hub renders it as unknown, never as trusted. |
| **3** (no Directory anywhere) | The R1 half: e2e step 11 completes the find-and-engage path by direct link with no directory deployed. R2's half stays C8's. |
| **11** (a blocked sender) | **Fully, for R1 row 6**: parity 52/53 and e2e step 10 — never in a conversation, never in a search, never counted, and the product does not claim the sender was prevented. |
| **12** (flooding) | The publication half now has a caller (parity 43); the contact half is C4's, re-exercised at the inbox by parity 54. |
| **13** (import reproduces what was exported) | For the conversation sections: parity 56–58, e2e step 12. |
| **17** (restart mid-flow) | e2e steps 6 and 9, on both sides. |
| **19** (build divergence) | 28 new parity scenarios, including the two that matter most (60, 61). |

A row this slice explicitly does **not** close: **12's "refusal is visible
to the sender"** for an inbound refusal. §6.3 says why, and the backlog row
for the absent admit hook is where it lives.

---

## §12 Order of work

Each step compiles and its own tests pass before the next.

1. **`syneroym:invocation`**: the WIT package, the `wit_interfaces`
   feature and `bindgen!` entry, `AppInvocation` on `AppHost`, the guest
   impl, the native impl, `HostState.invocation_origin`,
   `execute_wasm_json_from_wire`, and the one changed line in
   `dispatch.rs` (§3). **The workspace stops compiling here until step 3**
   — `AppHost`'s supertrait list grew.
2. The fixture's world, its origin op, and its native impl; the
   `app_host_native` parity scenario (§3.4, §3.5).
3. The five Roym service worlds gain the import, and `bindings.rs` is
   regenerated for each. Workspace compiles again.
4. `roym_core`: `admit`, `area`, `listing`, `conversation`, the two backup
   section names, `RECORD_LISTING`, and their unit tests (§4). No host
   involved beyond the stub.
5. `admit::require_internal` at the top of all six services' `invoke`,
   plus `profile.policy`'s new sentence (§7). Nothing else changes yet, so
   any breakage here is the gate and only the gate.
6. `roym_catalog`: collections, verbs, `listing.set`'s signing flow,
   `listing.verify`, availability, the publication limiter, and the
   certificate mount (§5).
7. `roym_conversation`: the world's `guest-api` export, the guest and
   native sinks, `on_message` / `on_delivery_state`, the collections, and
   the verbs (§6).
8. The manifest's two `depends_on` edges and `init_roym`'s matching
   bindings and `set_conversation_sink` (§8, §6.2).
9. `roymctl`: `enrol-signing` / `signing-status` across three services, and
   `roym address` (§9).
10. The parity harness's three changes, then scenarios 37–64 (§11.2).
    Scenario 5's move to `transaction` lands with the harness change, not
    after it.
11. `roym_conversation_e2e.rs` (§11.3), and `roym_app_e2e.rs`'s one added
    step.
12. The Hub (§10), its vitest additions, and `roym-hub.spec.ts` (§11.4).
    Rebuild with `mise run build:roym-ui` and `mise run build:roym`; run
    `mise run test:roym-ui`.
13. `cargo xtask check-roym-deps`, then the full gate:
    `cargo +nightly fmt --all`,
    `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace`, `cargo audit`,
    `cargo deny check licenses`, `mise run test:e2e`.
14. Documents and backlog (§15).

Step 1 is the choke point and the riskiest: it changes a trait bound the
whole workspace sees. Steps 6 and 7 are independent of each other and can
run in either order once step 5 lands.

---

## §13 What is compared across builds, and what is not

`D-C4-12`'s rule, extended to this slice's artifacts.

- **Compared byte for byte:** the signed `listing` envelope. Its
  `issued_at_secs` is the host's `RecordClock`, which the harness pins to
  `Fixed(F)`.
- **Compared after `strip_volatile`:** every row Roym writes. The new
  volatile fields are `stored_at_secs`, `opened_at_secs`,
  `updated_at_secs`, `deleted_at_secs`, `last_activity_ms` and
  `sender_timestamp_ms` — the last two because they come from the host's
  own millisecond clock at send time, which is not the pinned signing
  clock.
- **Compared directly, because they are content-derived:** `listing_id`,
  `record_id`, `slot_id`. Each is `content_digest` over values with no
  clock in them (`D-C5-15`, `D-C4-14`).
- **Asserted separately for presence and plausibility:** every stripped
  field, so "we stopped comparing it" never quietly becomes "we stopped
  checking it exists".

---

## §14 Permitted differences (WASM vs native)

To be appended to `status.md`'s §14 list, continuing from item 9.

10. **`CallerOrigin::Anonymous` is unreachable on the native build.** A
    natively linked Roym service is registered only in the local endpoint
    registry and is never published (`roym_dispatch_id`'s own comment in
    `crates/substrate/src/runtime.rs`), so nothing can reach it from the
    network, and the native dispatch path rejects an absent caller before a
    service ever sees it. The arm exists and is unit-tested on the WASM
    side; the parity suite proves `Internal` and `Verified` on both. Same
    shape as item 9: an absent path, not a divergent answer.
11. **The inbound notification mechanism differs and the store contents do
    not.** WASM instantiates the component and calls its `guest-api`
    export with a 4-attempt retry; the native build calls a
    `Weak<dyn ConversationSink>` once, with no retry. B3's own precedent
    for `MessageSink`, restated because C5 is the first slice where a Roym
    service is on the receiving end. What the parity suite compares is the
    `messages` and `refused_messages` rows afterwards, never the timing.
12. **Guest wall-clocks stay unsynchronized** (item 7, unchanged), which
    is why §13 lists six volatile fields rather than four.

---

## §15 Documents and backlog owed

| Document | Edit |
|---|---|
| [status.md](status.md) | A C5 section: what shipped; §11.5's matrix rows including the one row C5 explicitly does **not** close; §14's three new permitted differences; the new host interface named as an addition to the app-facing surface; and `D-C5-4` stated explicitly, so a reader does not conclude that a manifest `visibility` value is protecting the API. |
| [task.md](task.md) | The open design point *"What 'area' means on the wire"* answered (`D-C5-6`, micro-degree bbox / circle / named, with the integer constraint as its reason). The open design point *"Where the app-owned message copy lives, and what it costs"* answered (`D-C5-7`, with `F10`'s numbers). The *"which service owns the person→conversation-address mapping ... how a listing embeds it"* point closed on the listing side (`D-C5-8`). The Migration-impact section's `AppHost` supertrait note updated: the list grew to ten in this slice, after C3 §18 **G** and C4 §18 **G** both recorded it as settled at nine. |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | The Records table's `listing` row gains, in its "Does not prove" column, that a listing's stated payee is not agreed terms until an `agreement-receipt` binds it. The Messaging section's deletion paragraph gains the sentence the product actually shows (§6.7's `note`). The Safety section's retention bullet gains `D-C5-7`'s duplication statement. Catalog's API column gains `listing.verify` and `listing.limits`/`set-limits`; Conversation's gains `search`, `delete-message`, `export`/`import`. |
| [deferred-backlog.md](../../deferred-backlog.md) | §10's **wire-side authorization** row moves to "Recently resolved": `syneroym:invocation` plus `admit::require_internal` refuse every Roym verb that did not arrive through a local dispatch path, with `directory`'s first genuinely wire-reachable verb named as C6's to add as the first exception. §10's `[PRD-SAF]` row: the **inbox** half moves to "Recently resolved" (`on_message` consults the block list; parity 52/53; e2e step 10) and the **publication** half is now half-resolved — the catalog-side caller ships, the directory-side one stays C6's; the report and contact-limit **UI surfacing** halves move to "Recently resolved". §5's **native subscription replay** row targeted at C5: restated with what C5 found — Roym subscribes to no messaging topic even now (conversations do not use the pub/sub broker, by ADR-0013 §6), so C5 still supplies no consumer; retarget or close it rather than let it read as C5's and unmet. §3's **`depends_on` not enforced** row: restated, not resolved — C5 declares the two edges it traverses (`D-C5-9`) and the binding check that would enforce declarations is still absent. **New rows:** (a) the host-side Tier-1 gate (`TODO(B7b / post-B7)`) is still open — `syneroym:invocation` lets a component refuse for itself, and the substrate still admits an anonymous wire caller into any deployed WASM service that does not; (b) `CallerOrigin::Internal` says the call arrived through a local dispatch path, not that the *chain* began locally — a wire caller reaching a service that then proxies to a sibling would present as internal at the sibling, which C5 avoids by construction (no wire-reachable verb proxies on a caller's behalf) and which needs a real answer before one does; (c) no host surface reports a service its own routing address (`F14`), so a person types their conversation address once and `roym address` is the crutch; (d) `refused_messages` is never pruned, the same shape as C4's `contact_attempts` row; (e) Roym's copy of a conversation body doubles the on-disk cost with no tiering (`D-C5-7`), trigger *"attachments enter the release"*; (f) `conversation.search` is a `$regex` scan with no index, replaced by C6's FTS5 work. |
| [developer-guide.md](../../../developer-guide.md) | The enrolment ceremony now covers three services, not one. `roym address` documented beside it. |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | The architecture paragraph's WIT interface list gains `invocation`. The sentence saying the five non-`web` services *"answer `<name>.ping` only"* is now wrong for `profile`, `catalog` and `conversation`. |

**No new ADR.** The new interface adds no wire format, changes no record
envelope, and is decided the way `D-06C-3` decided the card set. If a
reviewer disagrees — the argument would be that a host authorization
primitive deserves a permanent record — the ADR to write is about the
substrate's Tier-1 gate as a whole (backlog row (a)), not about this one
import.

---

## §16 What "done" means for C5

1. A provider creates a signed listing with all seven blocks, edits it,
   and the edit is a **new version** carrying `supersedes` — with the
   previous envelope still readable through `listing.history`.
2. A listing's payload carries the provider's conversation address under
   the provider's own signature, and a stranger who verifies it can start
   a conversation with no directory and no prior contact entry.
3. A message shows `pending` while it is pending, on both sides, across a
   restart of either substrate, and is never shown as `delivered` before
   the host says so.
4. Roym holds its own copy of every message it sent and received; that
   copy is what `conversation.search`, `conversation.delete-message` and
   `conversation.export` act on; and the product states in
   `profile.policy` and in the Hub that this means two copies on disk.
5. A blocked sender's message reaches no conversation, no search result
   and no count, the sender is not told, and the product never claims the
   sender could not send — **R1 row 6's acceptance test closes here**.
6. `listing.get` and `conversation.history` called over the wire by a
   verified stranger answer `-32013` on both builds; the same verbs called
   locally succeed on both builds.
7. `safety::admit_publication` has a caller, and a provider minting listing
   versions past the limit is refused with a `retry_after_secs` the caller
   can act on.
8. `handle_certificate_verb` is mounted on `catalog` and `conversation`,
   and one `roym enrol-signing` enrols all three services that need it.
9. All 28 new parity scenarios pass identically on both builds, including
   scenario 64's guard — no verb named in a scenario answers `-32601` or
   `-32013` through the local path.
10. The two-substrate e2e passes all twelve steps.
11. `cargo xtask check-roym-deps` is clean, and a grep over
    `crates/roym_*`, `crates/roym_core/app/`, `crates/wit_interfaces/wit/invocation/`
    and every file this slice touched finds **no planning identifier in
    any name *or* comment** — `M0[0-9]`, `\bR[1-4]\b`, `\bC[0-9]`,
    `D-C[0-9]`, `D-0[0-9]`, `Slice `. ADR references are the only
    permitted exception.
12. The full gate in §12 step 13 is clean.
13. §15's documents and backlog rows are written, including the row C5
    does **not** close and the three it restates rather than resolves.

---

## §17 What C5 deliberately does not build

- **The directory**, its search, its FTS5/R\*Tree index, and the
  directory-side publication limiter. C6. `listing.verify` is the piece C6
  will call on every result, built here because the no-directory path needs
  it anyway.
- **Group conversations.** C10. The `conversation-kind` enum already
  carries `group`; C5 stores `direct` rows and refuses a group id rather
  than half-handling one.
- **Cards in a conversation.** C7 produces them and C2's renderer consumes
  them; C5 carries a message body and a content type and interprets
  neither.
- **Booking anything.** `availability.*` records what a provider offers.
  The state machine, the single writer, idempotency and conflicts are C8's.
- **A signed or composed export bundle.** `D-C5-17`: two symmetric
  bundles, composition and signature in C8.
- **Enforcement of `relationship.open_to`.** The listing states the rule;
  checking a membership credential against it needs C9's credential
  verification. The Hub says which of the two it is showing.
- **A refusal signal to a blocked sender.** §6.3. Needs the inbound admit
  hook `D-06C-8` deferred, which is a substrate design with its own
  backlog row and its own trigger.
- **The substrate's own Tier-1 gate.** `syneroym:invocation` lets a
  component refuse for itself. It does not make the substrate refuse an
  anonymous wire caller into a deployed component that declines to check —
  backlog row (a).
- **Attachments, ranking, stock counting, recurring bookings** — each
  excluded by a named row of the spec's own scope table.

---

## §18 Ambiguities and staleness in the input documents

Flagged rather than guessed. **A** and **B** change what gets built.

**A. The C5-targeted backlog row asks for something the tree cannot
express.** *"Wire-side authorization on `catalog`/`conversation`/
`directory`'s `api.invoke` ... C5 owns this when it adds the first
wire-reachable write verbs"* assumes a service can decide something about
its caller. `F4` says no host surface tells a component who called it
outside inbound HTTP, and `F5` says the caller alone could not carry the
answer even if it did. `F2` closes the escape route of narrowing
`visibility`. So the row is met by adding the missing capability
(`D-C5-1`), not by writing a check against information that does not exist.
A reviewer who refuses a new host interface in a product slice must accept
one of two consequences and say which: `conversation.history` is readable
by anyone holding an address the product publishes on purpose (`F1`), or R1
row 3 does not ship in C5.

**B. The exposure is not hypothetical and does not need a stranger to
guess a DID.** The registry has no enumeration endpoint
(`crates/community_registry/src/registry.rs:306` is `GET /lookup/{service_id}`
and there is no list route), so a sweep is impossible. It does not matter:
`ProfilePayload.conversation_address` **is** the service id, it is signed
into a `profile` record and embedded in every listing so that strangers can
use it, and the spec's own journey step C10 has a stranger starting a
conversation from a listing. This is worth stating precisely because the
weaker version of the argument — "somebody might discover the DID" — invites
the answer "then keep the DID secret", which would break the product.

**C. R1 row 1's "history" half closes here, and R1 row 6's acceptance test
closes here.** Both were retargeted from C4 by `status.md` §9 and both are
in §16. R1 row 1's *identity* half stays closed as C4 proved it; what C5
adds is that a restore reproduces conversation content too (e2e step 12) —
for Roym's own copy, which is the only copy anything can write back into
(Gap 3).

**D. `task.md`'s slice table says C5 depends on C4 and that "C4 and C5
could themselves overlap once C3's envelope is frozen".** They did not
overlap, and C5 depends on C4 more than the table implies: `on_message`
calls `block.check` and `contacts.admit-first-contact`, and `listing.set`
calls `profile.get` — three verbs C4 shipped. Recorded so the dependency
edge is not read as nominal.

**E. `task.md`'s Migration-impact section says the shim's trait list is
settled.** C3 §18 **G** and C4 §18 **G** both recorded it as stopping at
nine. `D-C5-1` makes it ten. §15 has the edit; noted here so the next
slice does not re-flag a section this one changed.

**F. The spec's service table gives Conversation the API `send`, `history`,
`conversations`, `delivery-status`.** C5 ships those plus `open`, `outbox`,
`retry`, `search`, `delete-message`, `export` and `import`. Each of the
extras is required by a rule elsewhere in the same document — the deletion
rule, the export row, `D-06C-5`'s "searched, deleted and restored" — so
this is the table being a summary rather than a contract. §15 records the
edit rather than leaving the two disagreeing.

---

## §19 Open questions for the executor

Choices, not defects. Each has a recommended answer and the plan works
either way.

1. **Whether `listing.verify` should also accept a bare `record_id` and
   look it up.** The plan says no: C5 stores no stranger's listing, so
   there would be nothing to look up. C6 adds the lookup with a source and
   a freshness beside it, which is what the spec asks results to carry.
2. **The default `PublicationLimits`.** The plan takes
   `roym_core::safety`'s existing default (20 per 24 hours). A provider
   with a large catalog hits it on first import; a first import is
   plausibly a legitimate burst, and raising the default rather than
   special-casing an import is the change to make if it bites.
3. **Whether `conversation.history` should page from the host's `history`
   instead of Roym's copy.** The plan says Roym's copy, because deletion
   and search only exist there and a person must not see a message they
   deleted reappear. The cost is that a message the host holds and Roym
   refused is invisible — which is `D-06C-8` working as intended, and is
   worth re-reading before changing this.
4. **Whether `admit::require_internal` belongs on `web`'s `api.invoke`.**
   The plan says yes. Nothing legitimate calls it from the wire, and `web`
   is `internal` so nothing can today; the check costs one line and
   removes the reliance on that manifest value.

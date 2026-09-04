# M06C Slice C6 — Directory: the Search Half: Implementation Plan

> **Scope.** [task.md](task.md)'s **C6** row — R1 row 5. The SynOrg /
> Directory service and its settings (name, rules, area, categories,
> support contact, dispute path, retention policy); its member list;
> provider-initiated listing publication (the spec's journey step **S7** —
> publishing is the *provider's* action, never a directory-side pull);
> `search` by category, area and filters; results carrying **source** and
> **freshness**; a consumer's node querying each directory it was given and
> merging the answers; missing evidence rendering as **unknown**, never as
> a positive default. Gate: **C5**.
>
> **C6 also owes four things earlier slices handed it by name**
> ([deferred-backlog.md](../../deferred-backlog.md)): the directory-side
> caller of `safety::admit_publication` (`D-C4-15`, `D-C5-14`), the first
> genuinely wire-reachable Roym verb as the first named exception in
> `roym_core::admit` (`D-C5-3`), the `conversation`/`catalog` read verbs
> that scan whole collections (C5 review C5-7, trigger *"before C6 puts a
> search on top"*), and `conversation.search`'s unindexed scan.
>
> **Read §18 first if you are executing this plan.** Three claims in the
> input documents do not hold against the tree, and one of them —
> **A** — invalidates the mechanism the C6 row and Gap 7 both name. C6
> does **not** build FTS5 or R\*Tree tables, because neither
> `execute-ddl` nor `query-raw` is reachable from a service verb on either
> build. §1 `F1`–`F4` are the evidence; `D-C6-1` is the decision; §15 has
> the `task.md` correction and the backlog row that keeps the idea alive
> with a real pickup trigger.
>
> **Planning identifiers appear in this document and must not appear in
> the code it describes.** AGENTS.md forbids slice and milestone ids in
> comments and doc comments, not only in names. Every name and comment
> proposed below is written with that already applied. §16 item 19 checks
> comments, not just names.

---

## §0 What C5 handed C6, and what is missing

Verified 2026-09-04 against `feat/m06c-slice-c5` at `e57629b`.

| Handed over | Where |
|---|---|
| A signed, versioned `listing` record with a required core and seven optional blocks, and a stable content-derived `listing_id` | `crates/roym_core/src/listing.rs`; `derive_listing_id` at `listing.rs:731`'s call site in `roym_catalog` |
| `listing.verify` — a pure envelope check with no host use, answering inside a success envelope | `crates/roym_catalog/src/app.rs:698` |
| `Area` (`bbox` / `circle` / `named`, integer micro-degrees) and `bounding_box`, the one over-covering projection an index builds on | `crates/roym_core/src/area.rs:106` |
| `safety::admit_publication` + `PublicationLimits`, with a catalog-side caller and a stated withdrawal exemption | `crates/roym_core/src/safety.rs:134`; `crates/roym_catalog/src/app.rs:302` |
| `syneroym:invocation` and `admit::require_internal` — the first statement of every Roym service's `invoke`, refusing anything that did not arrive `internal` with `-32013` | `crates/wit_interfaces/wit/invocation/invocation.wit`; `crates/roym_core/src/admit.rs:21` |
| `directory` already declared `visibility = "public"` **and** `topology_visibility = "open"` — the only service with the second | `crates/roym_core/app/roym.toml` |
| `directory.` already in the routing table at `MethodAuth::Owner` | `crates/roym_core/src/router.rs:37` |
| `ProfilePayload.conversation_address`, signed into a `profile` record, and required on every `listing` | `crates/roym_core/src/person.rs`; `listing.rs:177` |
| A parity harness with a pinned `RecordClock`, a wire driver (`wire_invoke`), a `did:key:hForeign` proxy target already routed to the directory, and synthetic `unbound` / `timeout` targets | `crates/roym_web/tests/dual_build_parity.rs:363`, `:605`, `:655` |
| A two-substrate e2e harness with boot / deploy / restart, a shared registry, and per-service minted masters | `crates/substrate/tests/roym_conversation_e2e.rs:213` |
| The Hub's tab shell, `rpc.ts`'s `-32013` → `NotLocal` mapping, and the rule that every stranger-influenced value is a text node | `crates/roym_web/ui/src/main.ts:99` |

Missing, and C6's to build:

1. **No directory state at all.** `roym_directory::app::invoke` answers
   `directory.ping` and nothing else
   (`crates/roym_directory/src/app.rs`). No SynOrg settings type, no
   member list, no publication store, no search.
2. **No wire-reachable verb anywhere in the product.** `require_internal`
   has no exception mechanism — it is a two-arm match with no method
   parameter (`admit.rs:21`). Parity scenario 67 currently *asserts* that
   `directory.ping` is wire-refused
   (`dual_build_parity.rs:3047`).
3. **No client half.** Nothing on a consumer's node holds a list of
   directories, calls one, or merges answers. `web` declares
   `depends_on = [… "directory"]` already, so the ingress exists; the
   service behind it is empty.
4. **`listing.verify` drops the revocation verdict.** `verify_json`
   computes `revocation_status` (`crates/signed_record/src/verify.rs:219`)
   and `verify_listing` never returns it
   (`crates/roym_catalog/src/app.rs:741`). With `VerifyOptions::new`'s
   `EMPTY_REVOCATIONS` (`verify.rs:74`) that value is always `Unknown`, so
   the product today renders a listing as `verified: true` with no
   statement about revocation at all — precisely the positive default R1
   row 5 forbids.
5. **No directory-side publication limiter**, so `[PRD-SAF]`'s publication
   half is still open.
6. **No `search`, and no way to build the one the C6 row describes** —
   see `F1`–`F4`.

---

## §1 Findings from reading the tree

Verified 2026-09-04 against `feat/m06c-slice-c5` at `e57629b`. Each is
load-bearing for a decision in §2. Line references were checked in this
pass, not carried over from an earlier plan.

### F1 — `execute-ddl` and `query-raw` are gated by `data-layer/admin`, on **both** builds

`store::Host for HostState` refuses both unless the current
`CallerContext` holds `data-layer/admin` on this component's own resource:

- `execute_ddl` — `crates/sandbox_wasm/src/host_capabilities.rs:1323`,
  gate at `:1332`.
- `query_raw` — same file `:1345`, gate at `:1358`.
- `drop_collection` — same gate, `:953`. `create_collection` is
  deliberately **un**gated (`:940`, with the reason in its own comment).

The native shim does not re-implement these: `NativeAppHost::execute_ddl`
and `::query_raw` call straight into the same `store::Host` impl on the
same `HostState` (`crates/app_host_native/src/host.rs:187`, `:192`, with
`HostStore` imported at `:38`). So the gate is identical on both builds —
which is good for parity and bad for the plan the C6 row assumes.

The native **dispatch** path carries the same gate independently
(`crates/control_plane/src/synsvc_native.rs:1258` for `execute-ddl`,
`:1282` for `query-raw`).

### F2 — the only producer of that capability is the lifecycle hook, and Roym exports none

`CallerContext::local_elevated` (`crates/rpc/src/native.rs:92`) is the one
context carrying `Ability::DATA_LAYER_ADMIN`. Its sole production call
site is `AppSandboxEngine::invoke_lifecycle_hook`
(`crates/sandbox_wasm/src/engine.rs:1445`), which the deploy path calls
directly (`engine.rs:1018`, choosing `init` on first deploy and `migrate`
after).

`prepare_wasm_execution` — the ordinary dispatch path, used by both the
wire site and guest-to-guest proxy calls — refuses to grant it and
`debug_assert!`s that no caller forwards one
(`engine.rs:1392-1414`). The comment says why in as many words: a caller
naming their request `"init"` would otherwise self-elevate.

No Roym component exports `init` or `migrate`. Every Roym world exports
`api` and nothing else (`crates/roym_directory/wit/world.wit`,
`crates/roym_catalog/wit/world.wit`, and the other four). The collections
are created lazily at verb time through the ungated `create-collection`
(`crates/roym_catalog/src/app.rs:60`).

The native build has **no lifecycle path at all**: `init_roym` builds each
service with `move |caller| f_xxx.host_for(caller)`
(`crates/substrate/src/runtime.rs:1723` for `directory`), and nothing
anywhere calls a native equivalent of `invoke_lifecycle_hook`.

### F3 — an owner-rooted UCAN chain can never carry `data-layer/admin`

The router's chain verifier treats a per-service owner as a trusted root
only for capabilities that do **not** entail `data-layer/admin`
(`crates/router/src/route_handler/io.rs:219-224`). So there is no
credential a person, a service, or `roymctl` could present that would let
an ordinary `api.invoke` reach DDL. This closes the last escape route:
the gate is not a configuration choice, it is the boundary ADR-0015/0016
drew.

**Consequence, stated plainly:** FTS5 and R\*Tree are compiled into the
bundled SQLite — re-verified today, `-DSQLITE_ENABLE_FTS5` at
`~/.cargo/registry/src/*/libsqlite3-sys-0.36.0/build.rs:133` and
`-DSQLITE_ENABLE_RTREE` at `:137`, inside the
`bundled`/`bundled-sqlcipher` branch this workspace selects
(`Cargo.toml:130`) — and a Roym service still cannot create one, write to
one, or read one. Gap 7's conclusion (*"the Directory can create and query
its own FTS5 and R\*Tree tables through DDL … with no new host
interface"*) is wrong about **who may run the DDL**, and the C6 row
inherits the error.

Half a fix exists and does not help: adding `init`/`migrate` exports to
the `directory` world, plus a native lifecycle entry point, would let
`init` create the virtual tables and the triggers that keep them in sync.
The **read** side stays unreachable — `MATCH` and an R\*Tree join cannot be
expressed through `query`'s filter DSL, and `query-raw` is still gated.
So the half-fix buys an index nothing can read.

### F4 — the declared collection indexes are never used by `query`

`create-collection` builds an expression index over a **literal** JSON
path: `CREATE INDEX idx_<coll>_<field> ON <coll>(json_extract(payload,
'$.<field>'))` (`crates/data_db/src/sqlite.rs:119-127`).

The filter compiler binds the path as a **parameter**:
`json_extract(payload, ?) <op> ?`
(`crates/data_db/src/filter.rs:256`, `:283`, `:298`, `:312`).

SQLite matches an expression index only against a textually equivalent
expression, and a `?` parameter is not a constant. Confirmed against
SQLite directly, not inferred:

```
CREATE INDEX idx_c_status ON c(json_extract(payload, '$.status'));
EXPLAIN QUERY PLAN SELECT * FROM c WHERE json_extract(payload, '$.status') = 'active';
  -> SEARCH c USING INDEX idx_c_status (<expr>=?)
EXPLAIN QUERY PLAN SELECT * FROM c WHERE json_extract(payload, ?) = ?;
  -> SCAN c
```

So **every filtered `query` in the product is a full table scan today**,
index declarations included. This matters twice: it means C6's search is a
scan whatever shape C6 gives it, and it means the C5-7 backlog row's
proposed fix (*"each fix is one `QueryOptions.filter`"*) reduces the rows
crossing the host boundary but does **not** make the query use an index.
The row is half-right and is restated in §15 rather than closed as
written.

Two further `query` facts C6's paging depends on: results are ordered
`ORDER BY id ASC` with the cursor keyed on `id`
(`crates/data_db/src/sqlite.rs:793`), and a page is hard-capped at
`MAX_QUERY_PAGE_SIZE = 1000` (`sqlite.rs:51`). There is no sort option in
`query-options` at all
(`crates/wit_interfaces/wit/data-layer/data-layer.wit`), so **any order a
verb returns is an order the guest produced itself**.

### F5 — `$regex` is `LIKE '%…%'`, which is exactly what a token filter needs

`compile_regex` binds `format!("%{pattern}%")` and emits
`json_extract(payload, ?) LIKE ?` (`crates/data_db/src/filter.rs:296-298`).
It is a substring match, ASCII-case-insensitive by SQLite's default
`LIKE`, and `%` / `_` inside the pattern are wildcards the caller must
escape (C5 already escapes for `conversation.search`).

Matching a delimited token string — `"|plumbing|electrical|"` filtered by
`$regex: "|plumbing|"` — is therefore an exact-token match through the
existing DSL, with no new host surface. A bounding-box intersection is
four numeric predicates under `$and`. Together they cover every filter R1
row 5 names.

### F6 — a WASM guest cannot do anything in parallel

`syneroym_app_host::guest::block_on` polls the future **once** and panics
on `Poll::Pending`, with the invariant stated in its own comment: *"the
WASM build can only await host calls, which never pend"*
(`crates/app_host/src/guest.rs:380-392`).

So the spec's *"queries each directory it has been given, **in parallel**,
and merges"* (Search section, journey step C6) cannot be honoured inside a
guest component. A node-side fan-out is necessarily one call after
another. The native build *could* be concurrent, which would make it
differ from the WASM build in ordering and timing — the one thing
failure-matrix row 19 forbids. `D-C6-9` settles this.

### F6b — a guest dispatch has a five-second wall-clock budget, and waiting on a host call spends it

`build_store_and_instantiate` arms `store.epoch_deadline_trap()` and
`store.set_epoch_deadline(epoch_deadline_ticks)`
(`crates/sandbox_wasm/src/engine.rs:1354-1355`). For the ordinary dispatch
path those ticks are `dispatch_epoch_ticks`
(`engine.rs:1420`), computed as `ticks_for_secs(dispatch_timeout_secs)`
(`engine.rs:530`) from `dispatch_epoch_timeout_secs`, whose default is
**5** (`crates/core/src/config.rs:479`).

The epoch is **wall-clock, not instruction count**: one `tokio::interval`
at `EPOCH_TICK_MS = 100` calls `engine.increment_epoch()` for the life of
the process (`engine.rs:663-670`, `:368`). Nothing about that ticker knows
whether a guest is executing or suspended in a host call, so **time spent
waiting on `proxy.call` is spent against the dispatch budget**.

Two consequences, and both are why `D-C6-9` is shaped the way it is:

1. **A fan-out cannot loop inside one dispatch.** Two sources at three
   seconds each exceed the budget and the guest traps on resumption.
2. **The native build would not trap.** `NativeAppHost` runs plain Rust
   with no epoch and no deadline, so the identical scenario completes
   natively and fails on WASM — a divergence failure-matrix row 19 calls a
   bug in the shim, and one no parity scenario would have caught, because
   a parity harness with fast in-process sources never approaches five
   seconds.

The lifecycle hook gets its own, larger budget
(`lifecycle_hook_epoch_timeout_secs`, default 30, `config.rs:482`) — which
is further evidence that the ordinary five seconds is a deliberate ceiling
for a dispatch and not an oversight to design around.

### F6c — a proxy call without a valid instance certificate arrives anonymous, silently

For a guest-originated call, `ProxyRouter` attaches a delegation **only**
when the calling service has an instance certificate that is present,
unexpired, and has a recorded owner (`crates/router/src/proxy.rs:833-840`).
Otherwise it attaches nothing at all — not even a bare node pubkey, which
is what the native-dispatch arm falls back to (`:818`). The comment there
says why an expired certificate is deliberately *not* presented: the
destination hard-rejects a delegation that fails to verify, while a
missing one degrades to anonymous, which pre-certificate callers tolerate.

So the destination reads `CallerOrigin::Anonymous`, and a `VerifiedOnly`
verb refuses with `-32013` — the deliberately uninformative *"not yours to
call"*. A missing or expired certificate on the **caller's** node is
therefore indistinguishable, at the caller, from *"this directory does not
want you"*.

`deferred-backlog.md`'s row at line 86 records that `init_roym` never
calls `set_instance_cert` for any Roym service, so **a natively linked
Roym cannot publish to a directory at all** — the outbound mirror of the
inbound limit `status.md` §14 item 10 already records.

### F6d — two node limits sit directly under a parallel client loop, and neither is a queue that waits politely

**Guest-HTTP admission is four concurrent per service, with a two-second
wait and then a 503.** `handle_guest_http_request` takes a per-service
semaphore sized `max_concurrent_guest_http_per_service`, default **4**
(`crates/core/src/config.rs:496`), and a request that cannot get a permit
within `GUEST_HTTP_ADMISSION_TIMEOUT` — **2 seconds**
(`crates/sandbox_wasm/src/engine.rs:363`) — returns
`GuestHttpFailure::Unavailable`, which the router answers as a 503
(`engine.rs:2546-2551`, `crates/router/src/route_handler/http.rs:694`).
The permit is held for the whole request, not just its start.

`DEFAULT_SOURCE_TIMEOUT_MS` is also 2 000. So an unbounded client loop
over `MAX_SOURCES = 8` puts four calls in flight and leaves four waiting
almost exactly the admission timeout — **a coin flip between running and a
503**, decided by scheduling noise. Worse, a 503 is not a directory
failure: it is this node refusing to start the call, and the plan's
`SourceError` set had no arm for it.

**The RPC leg takes no permit at all.** `prepare_wasm_execution` — the
path a `CallTarget::Dependency` call into `directory` uses — goes straight
to `build_store_and_instantiate` with no semaphore anywhere
(`engine.rs:1416-1424`); `guest_http_permits` is touched only by the HTTP
path (`engine.rs:2540`). So each in-flight source holds **two** instances,
`web` and `directory`, for the life of the call, against
`max_concurrent_instances`, default **10** (`config.rs:450`). Four
concurrent sources is eight instances before any other Roym traffic, and
the config's own comment for the HTTP semaphore says why that matters:
exhausting the pool is *"a hard error at instantiation, not a wait"*
(`config.rs:491-494`).

This is `D-C6-19`'s lesson one layer up — a bound chosen against an
assumed ceiling rather than a looked-up one — so `D-C6-26` gives it the
same treatment.

### F7 — a cross-node proxy call arrives `verified`, and the verified DID is the calling **service's** instance DID

Outbound: `ProxyRouter` puts the caller service's instance certificate in
the route preamble (`crates/router/src/proxy.rs:810-818` and `:833-839`).
Inbound: the router verifies it and the one wire dispatch site forwards
the resulting caller into `execute_wasm_json_from_wire`
(`crates/router/src/route_handler/dispatch.rs:251`), which sets
`InvocationOrigin::Wire`. `invocation.caller()` then maps
`AuthLevel::Delegated | AuthLevel::Ucan` to
`CallerOrigin::Verified(caller_did)` and everything else to `Anonymous`
(`crates/sandbox_wasm/src/host_capabilities.rs:616-632`).

The DID in that arm is the **calling service's** instance DID, not the
person's master DID: `proxy::Host::call` states that a genuine
cross-service call *"acts as itself"* and inherits nobody's identity
(`host_capabilities.rs:1490-1500`). `CallTarget::Service(did)` passes
straight through from guest code (`:1449`), so a guest may address a
foreign directory by DID with no declared dependency.

**So `verified(did)` answers "which node's service is on the wire", never
"which person is publishing".** Authenticating a publishing provider must
come from the signed listing envelope's own `issuer`. `D-C6-6` says so.

### F8 — nothing in the spec's Records table is a SynOrg *settings* record

`RECORD_TYPES` (`crates/roym_core/src/record.rs:18`) carries ten types and
none of them describes a group's own name, rules, area, categories,
support contact, dispute path, or retention policy. The spec's Records
table (lines 456–475 of the spec) is the same ten. The three SynOrg-signed
types it does list — `membership-credential`, `revocation`,
`moderation-decision` — are all R3 / C9 by `D-06C-6`.

So the SynOrg's settings and its member list are **unsigned app state** in
C6, the same call `D-C5-16` made for availability. Inventing an
eleventh record type nobody asked for would also mean mounting
`handle_certificate_verb` on `directory` and enrolling a fourth signing
certificate for a slice that signs nothing.

### F9 — `listing.verify` is pure, and its verdict is incomplete

`verify_listing` binds `let _ = host;` and calls
`verify_json(&env_str, &VerifyOptions::new(now))`
(`crates/roym_catalog/src/app.rs:698-715`). It never touches storage, so
its body is a candidate to move into `roym_core` verbatim.

`VerifyOptions::new` installs `EMPTY_REVOCATIONS`
(`crates/signed_record/src/verify.rs:74`), whose every check answers
`RevocationCheck::Unknown` (`:23`, `:26`), so `revocation_status` is
always `RevocationStatus::Unknown` (`:219-225`). `verify_listing`'s
success body returns `verified`, `listing_id`, `issuer`,
`conversation_address`, `status` — and no revocation field
(`crates/roym_catalog/src/app.rs:741-747`).

### F10 — the parity harness already has the fan-out scaffolding, pointed the wrong way

`TestWasmServiceProxy` and `TestNativeServiceProxy` both special-case
`did:key:hForeign` and route it to the **directory** service
(`crates/roym_web/tests/dual_build_parity.rs:615`, `:669`), and both
synthesize `ServiceNotFound` for a target containing `unbound` and
`Timeout` for one containing `timeout` (`:621-626`).

But both route through the **local** entry point —
`engine.execute_wasm_json` on the WASM side (`:638`) and
`svc.dispatch(...)` on the native side (`:695`). A cross-directory call
driven through them therefore arrives `internal`, which `require_internal`
admits, so the new wire exception would never be exercised. The harness
needs a wire-flavoured foreign target; §11.2 specifies it.

### F11 — scenario 67 asserts today that `directory.ping` is wire-refused

`WIRE_REFUSED_VERBS` names one verb per service and picks
`directory.ping` for this one (`dual_build_parity.rs:3047-3054`). C6 makes
some directory verbs wire-reachable, so the constant must keep naming a
verb that stays refused, and a new list must name the ones that do not.

### F12 — `delete-many` exists and is not admin-gated

`delete-many(collection, filter)` returns the number deleted
(`crates/wit_interfaces/wit/data-layer/data-layer.wit`), and its host impl
sits with the ordinary write verbs, not behind the admin gate. So a
window-bounded ledger can prune itself with one call — which is what the
three open "never pruned" backlog rows all need.

### F13 — `web` reaches every sibling by declared dependency, and the routing table has no default arm

`web` resolves the owning service through `router::route` and calls
`CallTarget::Dependency(service.name)` on both of its ingress paths
(`crates/roym_web/src/app.rs:193`/`:221` and `:284`/`:296`). A prefix
absent from `ROUTES` is `-32601`
(`crates/roym_core/src/router.rs:22`). Two prefix invariants are unit
tested: every prefix maps to a service in `SIBLINGS`, and no prefix is a
prefix of another (`router.rs:64`, `:74`).

### F14 — the manifest values C6 depends on are already correct

Read, not assumed, from `crates/roym_core/app/roym.toml`: `directory` is
`visibility = "public"` and `topology_visibility = "open"`, and it is the
only service declaring the second. ADR-0018 §1 defines `public` as
*registered and propagated to parent registries*; ADR-0022 §5's amendment
defines `open` as bypassing the Tier-2 capability check only, and states
that `open` paired with `private` is refused at submit time. So
failure-matrix row 18's precondition is met by the manifest as it stands
and **C6 changes no visibility value** — the same statement `D-C5-4`
made, for the same reason.

Gateway addressing is unchanged too: ADR-0022 §7's corrected grammar is
`<nickname>-s<service-did-hash>[-i<interface-hash>].<domain>` or
`<nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>].<domain>`,
only the first DNS label is parsed, and C6 introduces no new host form.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C6-1** | **C6 builds no FTS5 table, no R\*Tree table, and issues no `execute-ddl` or `query-raw` call. Search is built on the existing filter DSL over a purpose-built projection collection.** The C6 row and Gap 7 are corrected in `task.md` rather than followed. | `F1`–`F3`: both raw-SQL verbs require `data-layer/admin`, the only producer of that capability is the deploy-time lifecycle hook, no Roym component exports one, the native build has no lifecycle path at all, and no owner-rooted credential can carry the ability. The half-fix (lifecycle exports on the world plus a native entry point) makes the **write** side reachable and leaves the **read** side unreachable, so it buys an index nothing can query. `F5` shows the DSL already expresses every filter R1 row 5 names — token match, bounding-box intersection, status, and free text — and `F4` shows the alternative was never going to use an index either, so what is actually given up is a better constant factor, not a capability. Escalating a security boundary ADR-0015/0016 drew on purpose, inside a product slice, is the one option that must not be taken quietly. |
| **D-C6-2** | **`directory` is one service with two halves, and the halves never share a collection.** The **server half** (a SynOrg's own directory: settings, members, publications, search) and the **client half** (this person's list of directories, the fan-out, the merge, the verification) live in the same crate and the same deployed service, distinguished by verb prefix and by collection, never by a mode flag. | Every installation already runs all six services (C2), and the consumer needs a node-side home for the directory list, the outbound call and the merge. `catalog` is the provider's offer and `profile` is the person's card; neither is it. A second crate would need a second manifest entry, a second certificate ceremony and a second `depends_on` graph for a service that shares its whole vocabulary with the first. The spec's *"Runs on: SynOrg's substrate"* column says where the server half **matters**, not where the component is deployed — C2 already deployed it everywhere and no test changed. Separate collections are what keeps *"a directory is a query target, not a required hub"* true structurally: a node with no `settings` row is not a SynOrg and answers `search` with nothing. |
| **D-C6-3** | **The publication store holds the whole signed envelope, byte for byte as it was published, and the search projection sits beside it as derived, rebuildable state.** `publications` is the source of record; `search_index` is an index in the ordinary sense and may be dropped and rebuilt at any time by `directory.reindex`. | `D-06C-6c` requires the consumer's own node to verify — which means the consumer must receive the exact bytes the provider signed. Any projection of a signed payload is unverifiable, so a directory that stored only a projection could not serve evidence at all. Keeping the projection separate and explicitly derived is also what answers the open design point about orphaning: there is nothing to orphan, because losing the projection loses no information. |
| **D-C6-4** | **The projection is one `search_index` row per (`listing_id`, service-area slot), never one per listing.** A listing with no geographic area gets exactly one row with the area columns absent; a listing with *n* areas gets *n* rows (`n ≤ MAX_AREAS = 8`). Categories ride each row as a delimited token string; the row also carries the original `Area` JSON, the status, the issuer, and the signed `issued_at_secs`. | `F5`: the DSL has no array operator, so a bounding box must be four scalar columns on one row and a category set must be a substring-matchable token string. One row per area is what lets a single `$and` express *"this listing serves somewhere inside the box the person asked about"* without the guest fetching and re-testing every listing. Carrying the original `Area` is what makes `D-C6-5`'s exact refinement possible without a second read. |
| **D-C6-5** | **The bounding box is the sieve and is never the answer. Every candidate row is refined exactly, in the guest, against the original `Area`.** `bbox`∩`bbox` is exact already; `circle`∩`circle` is centre distance ≤ *r₁+r₂*; `circle`∩`bbox` is the box's closest point to the centre ≤ *r*; `named` has no geometry and matches only by case-folded label equality, in both directions. A listing stating only a named area is **not** returned by a geometric query, and the result set says so rather than silently omitting it. | `area::bounding_box` over-covers deliberately (`crates/roym_core/src/area.rs:100-106`), so the sieve returns false positives by design. Showing them would make a radius search mean something different from what the person asked, which the spec's own *"Consumer sees results, and can see **why** each one appeared"* (journey step C9) forbids. Refinement is float arithmetic on unsigned data, so `D-C5-6`'s integer rule is untouched. The named-area asymmetry is a real product limit and is stated in the Hub rather than left to be discovered. |
| **D-C6-6** | **A publication is authorized by the signature it carries, never by the identity on the connection.** `directory.publish` requires a `verified(did)` caller, but that DID is only recorded as `published_by` and used as an audit line. What decides acceptance is: the envelope verifies, it is a `listing` at a known version, `listing_id` is derivable from the envelope's own issuer, and the **issuer** is inside the publication limit. | `F7`: the wire identity is the calling *service's* instance DID, which cannot name a person and which a directory has no way to relate to one. The envelope's issuer is the person, is signed, and is the same value the consumer will verify later. Requiring `verified` anyway is not decoration — it makes a publication attributable to a node, gives a future rate limiter a stable key, and refuses a fully anonymous write to somebody else's disk. |
| **D-C6-7** | **The wire exception is a table of named methods in `roym_core::admit`, not a per-service `if`. Exactly three directory methods are reachable off the node: `directory.search`, `directory.info`, `directory.publish`. Everything else on every service stays local-only.** `search` and `info` admit `verified` and `anonymous` alike; `publish` admits `verified` only. Withdrawal is a publish of a `withdrawn` version, so it needs no fourth entry. | `D-C5-3` said the exception belongs in the helper *"with the wire-reachable verb in hand"*. A table with an explicit list and a local-only default is a rule a reader can check and a test can enumerate (§11.2's guard). Reading a directory is what a directory is **for**; requiring an identity to read one would make *"a consumer's node queries each directory it was given"* depend on a certificate the directory cannot evaluate anyway, and would buy nothing, because a verified reader is granted nothing an anonymous one is not — stated explicitly so that no later reader mistakes `verified` for `trusted` (`D-06C-6c`). Writing to a directory is durable and must name a party (`D-C6-6`). |
| **D-C6-8** | **Freshness is two numbers with two owners, and the one the person sees is computed on their own clock.** A result carries the envelope's signed `issued_at_secs` (the provider's clock, under the provider's signature) and the directory's `received_at_secs` (the directory's clock, unsigned, labelled as the directory's claim). Displayed age is `consumer_now − issued_at_secs`, computed by the consumer's node. `received_at_secs` is shown beside it as *"this directory says it received this at …"* and is never used to compute age. | `D-06C-6c` in its narrowest form: a directory's assertion is never consulted for a verification result, and *"how old is this offer"* is a verification result. `issued_at_secs` is the only timestamp the consumer can attribute to anyone. Reporting both, labelled, is also what lets a person see a directory serving stale copies — which is information the spec's *"see why each one appeared"* asks for. |
| **D-C6-9** | **The fan-out loop lives outside the guest. One `directory.query-source` dispatch asks exactly one directory; the client (the Hub, or `roymctl`) runs the loop and may run it concurrently; `directory.merge` combines the results in one further dispatch. No guest dispatch ever waits on more than one source.** | Forced by `F6b`, and it is a hard limit rather than a preference: `store.epoch_deadline_trap()` is armed with `dispatch_epoch_timeout_secs`, **5 seconds by default**, and the epoch is advanced by a wall-clock `tokio::interval` — so the budget burns while the guest is suspended in a host call, and two slow sources trap the dispatch. The native build arms no epoch, so the identical scenario would *succeed* natively and trap on WASM, which failure-matrix row 19 calls a bug in the shim. Moving the loop out fixes three things at once. The epoch constraint disappears, because each dispatch makes one bounded call. **The spec's *"in parallel"* becomes literally true** — a browser issues the per-source calls concurrently — so the spec needs no edit and an earlier draft's narrowing of it is withdrawn. And every query still leaves the consumer's own node, through that node's own proxy, which is what journey step C6's *"directly, from the consumer's node"* actually asks for. The cost is that a client drives three calls instead of one; the merge and the verification stay node-side (`D-C6-10`, §6.3), so no client reimplements a rule, and exit criterion 2's second client drives the same three verbs the Hub does. |
| **D-C6-10** | **Merge is keyed on `listing_id`, keeps exactly one envelope per key, and never merges fields from two envelopes.** The kept envelope is the one with the greatest signed `issued_at_secs`; ties break on `record_id` ascending. Every source that answered for that key is listed in `sources[]` with the `record_id` it returned and its own `received_at_secs`. When two sources returned different `record_id`s, the row carries `versions_differ: true` and the Hub says two directories disagree about which version is current. | `listing_id` is content-derived from (issuer, slug) (`D-C5-15`), so it is globally unique without coordination and is the only stable key across sources. Picking by signed time uses the only clock anyone signed. A deterministic tiebreak is not cosmetic — the parity suite compares result order across builds, so an arbitrary tiebreak would be a flake. Surfacing the disagreement rather than resolving it silently is the same posture `D-06C-6c` takes everywhere else: the directory does not get to decide, and neither does the merge. |
| **D-C6-11** | **Ordering is recency *within* a source and round-robin *across* sources, with a deterministic tiebreak, and the product says it is not ranking.** Each source's verified hits sort by `issued_at_secs` descending then `listing_id` ascending; the merged page is filled by taking from each source in turn. No score, no relevance weight, no boost. | R1 row 5's Excluded column names *"Ranking; paid placement; free-text intent parsing"* explicitly, and a round-robin is a fairness rule rather than a relevance judgement, so it stays on the right side of that line. A **global** recency sort was the earlier answer and it fails `D-C6-18`'s own test one level up: a directory holding fifty validly signed, recently stamped listings fills the whole page and pushes every other source off it — the same crowding outcome, with the attacker merely paying for real signatures instead of forged ones. Interleaving is what makes the page immune to that, and it is deterministic, so the parity suite can still compare result order across builds. |
| **D-C6-12** | **SynOrg settings and the member list are unsigned app state, and `directory` mounts no signing certificate in this slice.** `directory.settings` / `set-settings` and `member.add` / `remove` / `list` write ordinary rows. | `F8`: the spec's Records table has no settings row and no roster row, and the three SynOrg-signed types it does have are all C9's by `D-06C-6`. `D-C5-18` fixed the rule — a certificate is mounted when a service first signs something — and a verb no flow exercises is what `D-C1-10` refuses everywhere. C9 mounts it, with credentials in hand. |
| **D-C6-13** | **The member roster is not enumerable over the wire.** `member.list` is local-only. `directory.info` (name, rules, area, categories, support contact, dispute path, retention policy) is wire-reachable, because a person deciding whether to trust a group must be able to read its rules before joining it. | A roster is the group's members' association, not the group's public statement, and R1 ships no consent mechanism for publishing it. Search results already reveal *which providers chose to publish there* — which is the provider's own act under `S7`, not the group's disclosure. `directory.info` going the other way is required by the spec's own policy-disclosure rule (Safety and operations) and by journey step C4, where a consumer decides which SynOrg to trust. |
| **D-C6-14** | **`directory.publish` is the directory-side caller of `safety::admit_publication`, keyed on the envelope's issuer, with limits in `settings` and read/set through `directory.limits` / `directory.set-limits`.** A publication whose payload `status` is `withdrawn` consumes no budget. The ledger prunes rows outside the window on every admit, in the same call. | `D-C4-15` and `D-C5-14` both promised this against the same function `listing.set` already calls, which is what `[PRD-SAF]` asked be fixed once. Keying on the issuer rather than on the connection follows `D-C6-6`. The withdrawal exemption is `D-C5-19` verbatim: refusing a provider the ability to take a listing down because they published too much is the worst outcome the limiter could produce. Pruning inside the admit is `F12`'s one call and stops C6 from adding a fourth unbounded ledger to the backlog. |
| **D-C6-15** | **The verification body moves into `roym_core::listing::verify_envelope` and gains the revocation verdict. `catalog.listing.verify` and the directory client call the one function, and both answers now carry `revocation_status`.** | `F9`: two verdicts on the same evidence is exactly the failure `D-06C-6c` is about, and the shorter path to it is two copies of the code. The revocation field is not an addition for its own sake — without it the product renders a stranger's listing as `verified: true` with no statement about revocation, which is R1 row 5's *"missing evidence shows as unknown, never as positive"* failing in the one place it is tested. `Unknown` is the honest value in R1 because C6 consults no revocation source, and C9 supplies one without changing the shape. |
| **D-C6-16** | **A consumer names a directory by its Roym Directory service DID, held in the client half's own `sources` collection, added by hand.** No discovery, no well-known list, no directory-of-directories. | The spec answers this itself: *"How does a consumer find a SynOrg? Out of band: word of mouth, a shared link, a referral, or a well-known list … the consumer's own decision."* The DID is the same shape as `conversation_address` (`F1` of the C5 plan) and resolves through the registry because `directory` is `visibility = "public"` (`F14`). Automatic discovery between SynOrgs is in the spec's own *Not in the first release* list. |
| **D-C6-17** | **The native shim's guest-HTTP and websocket sinks are wired through `host_for_wire`, closing the C6-targeted divergence row — as a severable work order of its own (§12 WO5), proven by a scenario in the *shim's* parity suite, not Roym's.** | The row's stated trigger (*"C6/C7's first component that puts `admit::require_internal` in an `incoming-handler`"*) has **not** fired and will not in C6 — `web` is still the only component with an HTTP handler and it gates on its session cookie. "It is cheap and it fits the theme" is not on its own a reason to put substrate wiring inside a product slice. What makes it right here is that it is **verifiable now**: `test-components/dual-build-fixture` already has an origin operation and an HTTP path (`init_dual_build_fixture`'s `set_http_sink`), so `crates/app_host_native/tests/dual_build_parity.rs` can drive a guest-HTTP request at the fixture on both builds and compare the reported origin. That makes this proven shim work rather than a blind change to a path nothing exercises — and it belongs in the shim's own suite, where the divergence lives, not in Roym's. Kept severable so that cutting it for time unwinds nothing else in C6. |
| **D-C6-18** | **No single source can fill a page — with forged listings or with genuine ones.** Three rules together: refused results live in their own list (`refused[]`, capped at `MAX_REFUSED_RESULTS`) and never in `hits[]`; a source contributes at most `MAX_HITS_PER_SOURCE` to the merged page; and the page is filled round-robin (`D-C6-11`). `MAX_SEARCH_RESULTS` is the **merged** cap, and `MAX_HITS_PER_SOURCE` is the per-source one — two constants, named apart, because an earlier draft used one for both and the ambiguity was the hole. | The forged half is the sharper attack — results sort by `issued_at_secs`, `verify_json` refuses a future timestamp (`VerifyError::IssuedInFuture`) but nothing stops a forger stamping *now*, so forged rows would sort first and a full page of them would bury every honest result invisibly. Splitting the lists fixes that one. It does **not** fix the signed version: a directory can hold fifty genuinely signed recent listings and crowd the page just as effectively, for the price of real identities. Only a per-source share fixes both, so both ship together — and the split list is still needed on its own, because the person must be told a directory served forgeries rather than have them silently dropped. |
| **D-C6-19** | **One number is derived, not chosen: `DEFAULT_SOURCE_TIMEOUT_MS` must leave headroom inside `dispatch_epoch_timeout_secs`, and a test asserts the relationship rather than trusting the constant.** `DEFAULT_SOURCE_TIMEOUT_MS = 2_000` against a 5 s default epoch. `MAX_SOURCES = 8` bounds how many calls a run makes, not any single dispatch, and there is **no** `MAX_TOTAL_SEARCH_MS` — the client owns the total, because the client owns the loop (`D-C6-9`). It does **not** own the *concurrency*: that is bounded separately, and derived from a different node limit, by `D-C6-26`. | An earlier draft set 8 sources x 3 s inside one dispatch against a 5 s epoch nobody had looked up: the same class of arithmetic error that draft's own review had already caught once, made again one layer down. The lesson is not a better constant, it is that a timeout inside a guest dispatch is only meaningful **relative to the epoch**, and nothing in the tree ties the two together. So the plan ties them: one deadline, one epoch, one assertion that the first plus verification headroom fits inside the second, failing at test time rather than as an intermittent trap in production. `MAX_SOURCES` survives because a person with 200 directories is a different problem, and because it bounds the run rows §6.1 writes. |
| **D-C6-20** | **The two halves get different method names, and no method changes meaning based on who called it.** Server half: `directory.search`, wire-reachable, answers from this node's own SynOrg. Client half: `directory.start-run`, `directory.query-source`, `directory.merge`, `directory.run-envelope` — all local-only. | `admit`'s exception table maps *method name* to wire rule. One shared name would make `("directory.search", Open)` declare the rule for the server half while the client half rode the same string, so a later mistake widening the client half would pass the guard test unnoticed. `D-C6-9` then decides how many client verbs there are — the loop is the client's, so the node offers the three steps it can perform in one bounded dispatch each. The subtle product rule this keeps visible: a node running a SynOrg does **not** include itself in its own run unless its owner adds itself as a source. |
| **D-C6-21** | **`directory.info` on a node that runs no SynOrg answers `null`, not an error — and `directory.add-source` reports that answer instead of swallowing it.** | Refusing would be indistinguishable from a network fault, which is the worse failure for the exact case this is about: someone adding an address a friend gave them. `null` also matches what `directory.settings` already answers locally, so there is one rule rather than two. The paired half matters as much: a source that probes to `null` is stored (the person may know something this node does not, and the SynOrg may be created later) but the person is told, so they do not sit waiting for results from an address that will never have any. |
| **D-C6-22** | **The directory's default publication limit stays `roym_core::safety`'s existing 20 per 24 hours — and surfacing `directory.set-limits` in the Hub's SynOrg tab is required work, not optional polish.** | The likely case is a provider with a large catalogue joining a group and being refused partway through their first publish, which is a poor first interaction with that group. But a different constant is not the fix: this limit is the **group's** policy, and the group's owner is the person who should set it. Two defaults for one function would also give `[PRD-SAF]` two stories where the row asked for one. What actually caused the bad experience is a default nobody can find, so the control is what ships beside it — §11.4 case 20 tests it, which is why the Hub item is in `D-C6`'s "done" list and not in a nice-to-have pile. |
| **D-C6-23** | **`retention_secs` is enforced, not merely displayed. Every `directory.publish` prunes publications and their index rows older than the SynOrg's stated retention, in the same pass that prunes the limiter ledger.** `received_at_secs` is carried on the `search_index` row as well as the `publications` row, so one `$lt` filter prunes both. | The slice states a retention policy in `directory.info` and shows it to a stranger deciding whether to trust the group. Storing that sentence and keeping nothing to it is precisely the failure this plan refuses everywhere else — it is why `credential` renders `unknown` rather than absent, and why the product never claims instant removal. A policy the product displays and does not keep is worse than one it does not display. Enforcement is two `delete-many` calls at a call site that already makes one, which is why there is no honest version of this deferred to a backlog row. |
| **D-C6-24** | **A free-text query is normalized, never escaped, and the product says what normalization does.** Lowercased; every character outside `[a-z0-9 -]` replaced with a space; runs of spaces collapsed. The stored `text` column is normalized identically at write time. | `compile_regex` emits `json_extract(payload, ?) LIKE ?` with **no `ESCAPE` clause** (`filter.rs:298`), so `%` and `_` cannot be escaped at all — they can only be removed or refused. Refusing is wrong (a person typing *"50% off"* deserves results, not an error), so they are removed, and searching for `50%` finds `50`. Saying *"escaped at write time"*, as an earlier draft did, described a mechanism that does not exist. The cited precedent was also wrong: C5's `escape_regex` (`crates/roym_conversation/src/app.rs:789`) escapes regex metacharacters `\^$.|?*+()[]{}` and **not** `%`/`_`, so C5's own `conversation.search` still has live wildcards — a defect this slice found and records rather than copies (§15). Normalizing both sides is also what makes matching work at all: SQLite's default `LIKE` folds ASCII only, so an unnormalized non-ASCII title would be case-sensitive; with both sides lowercased in Rust, the fold happens before SQLite sees it. |
| **D-C6-25** | **Only `active` and `withdrawn` may be published. A `draft` listing is refused at `directory.publish` with a reason.** | `directory.search` filters `status = "active"`, so a published draft would be stored, indexed, and counted against the provider's publication budget while being unreachable by any query — a silent charge for nothing. `withdrawn` is already special-cased as the removal path (§5.4 step 4). Refusing the third arm explicitly is what stops the status field having an unhandled case that behaves as a bug rather than as a decision. |
| **D-C6-26** | **The client's loop is bounded, and the bound is derived from the node's own admission limit rather than chosen. `start-run` returns the concurrency the client must honour; a node-side refusal is its own `SourceError` arm.** `MAX_CLIENT_CONCURRENCY = 3`, one below `max_concurrent_guest_http_per_service`'s default of 4, asserted against it by a test. `SourceError::NotStarted` is what a 503 from this node's own gateway becomes, and the client may retry those sources once the run's other calls have finished. | `F6d`: guest-HTTP admission is four concurrent per service and a request that waits longer than two seconds for a permit gets a 503 — and `DEFAULT_SOURCE_TIMEOUT_MS` is also two seconds, so an unbounded loop over eight sources decides four of them on a coin flip. Leaving one permit spare is what keeps the rest of the Hub alive while a search runs: a session refresh queued behind four slow directory calls would log the person out mid-search. The instance pool is the second, quieter reason — each in-flight source holds a `web` instance *and* a `directory` instance, because the RPC leg takes no permit at all, so three concurrent sources is six instances against a pool of ten that fails hard rather than waiting. **`NotStarted` restores an arm that was right and was removed for the wrong reason**: it was dropped with the sequential loop on the grounds that the client always knows which sources it asked, which is true and beside the point — the client can ask and be refused by its own node, and that is neither the directory timing out nor the directory refusing. |

---

## §3 `roym_core::admit` — the first wire exception (`D-C6-7`)

`require_internal` stays exactly as C5 shipped it. It has **six** callers
today — `catalog:213`, `conversation:392`, `directory:22`, `profile:195`,
`transaction:22`, `web:261` — and keeps five, because `directory` moves to
the new entry point beside it.

```rust
/// What a method allows from off this node. Absent from a service's
/// table means `LocalOnly`: an exception is written down or it does not
/// exist.
pub enum WireRule {
    /// Any caller, identified or not. Reading something this
    /// installation publishes on purpose.
    Open,
    /// A caller whose identity the router verified. The DID names the
    /// calling service's node, never a person -- a record's issuer is
    /// the only thing that names a person.
    VerifiedOnly,
}

/// Who is on the other end, once admission has already been decided.
pub enum Caller {
    Internal,
    Verified(String),
    Anonymous,
}

/// `Ok(caller)` admits. `Err(response)` is the refusal to return
/// unchanged, with the same code and the same wording
/// `require_internal` uses -- a stranger learns only that this method is
/// not theirs to call.
pub async fn admit<H: AppHost>(
    host: &H,
    exceptions: &[(&str, WireRule)],
    method: &str,
) -> Result<Caller, Response>;
```

Rules, in order:

1. `CallerOrigin::Internal` always admits and answers `Caller::Internal`,
   whatever the table says. A local dispatch is trusted for where it came
   from (`invocation.wit`'s own wording), and every existing local flow
   must keep working unchanged.
2. Otherwise, look up `method` in `exceptions`. Absent → the same
   `-32013` refusal `require_internal` returns, with the same message.
3. `Open` admits both wire arms. `VerifiedOnly` admits
   `CallerOrigin::Verified(did)` and refuses `Anonymous` with the same
   `-32013` — the refusal never says *why*, because *"you would be
   admitted if you had a certificate"* is information a stranger does not
   need.

`roym_directory` holds the one table in the tree:

```rust
const WIRE_REACHABLE: &[(&str, WireRule)] = &[
    ("directory.search",  WireRule::Open),
    ("directory.info",    WireRule::Open),
    ("directory.publish", WireRule::VerifiedOnly),
];
```

and calls `admit::admit(host, WIRE_REACHABLE, &req.method)` as the first
statement of `invoke`, exactly where `require_internal` sits in the other
five. The returned `Caller` is threaded to `publish` (which records
`published_by`) and dropped by the other two.

**`api.status` stays open exactly as `D-C5-3` left it** — it is not in the
table, because it is not routed through `invoke` at all.

**Unit tests** (in `admit.rs`, against C4's `TestHost`): internal admits
for a method absent from the table; an absent method refuses for both wire
arms; an `Open` method admits both wire arms and returns the right
`Caller`; a `VerifiedOnly` method admits `Verified` and refuses
`Anonymous`; and the refusal for `Anonymous` on a `VerifiedOnly` method is
byte-identical to the refusal for an unlisted method.

---

## §4 `syneroym-roym-core` — the shared vocabulary

### 4.1 `src/directory.rs` — new file

Types shared by the two halves and by the Hub. Every number is an integer,
because `settings` is unsigned today and might not stay that way
(`D-06C-1`'s field rule costs nothing to honour now).

```rust
pub const DIRECTORY_SCHEMA_VERSION: u32 = 1;   // roym_directory's SCHEMA_VERSION 1 -> 2

/// The **merged** page cap. Distinct from `MAX_HITS_PER_SOURCE` on
/// purpose: one constant serving as both was the ambiguity that let a
/// single source fill a page (`D-C6-18`).
pub const MAX_SEARCH_RESULTS: u32 = 50;
/// What any one directory may contribute to a merged page, however many
/// validly signed recent listings it holds.
pub const MAX_HITS_PER_SOURCE: u32 = 10;
/// What one directory may return for one query, before merging.
pub const MAX_HITS_PER_QUERY: u32 = 50;
/// Refused hits are carried and capped separately, so a directory serving
/// forgeries crowds nothing out (`D-C6-18`).
pub const MAX_REFUSED_RESULTS: u32 = 20;
/// Directories one person may add, and therefore the number of
/// `query-source` calls one run makes. It bounds no single dispatch
/// (`D-C6-9`).
pub const MAX_SOURCES: usize = 8;
/// How many of those calls may be in flight at once. **Derived from
/// `max_concurrent_guest_http_per_service` (default 4), not chosen**
/// (`D-C6-26`, `F6d`): one below it, so a search cannot consume every
/// admission permit this service has and stall the rest of the Hub. A
/// test asserts the relationship, exactly as one asserts the epoch
/// relationship below. `start-run` returns this value so the client does
/// not carry its own copy.
pub const MAX_CLIENT_CONCURRENCY: usize = 3;
/// Verified hits one `query-source` call stores. Twice the per-source
/// share, because the round-robin skips a listing another source already
/// contributed, so a source can be asked for more than its share's worth
/// of rows before it has contributed its share.
pub const MAX_STORED_PER_SOURCE: u32 = 2 * MAX_HITS_PER_SOURCE;
/// Per-source deadline for the one proxy call `directory.query-source`
/// makes. **Derived from the guest dispatch epoch, not chosen**
/// (`D-C6-19`, `F6b`): the dispatch traps after
/// `dispatch_epoch_timeout_secs` of wall clock -- 5 s by default -- and
/// that budget is spent while waiting on the call. A unit test asserts
/// this value plus verification headroom stays inside it.
pub const DEFAULT_SOURCE_TIMEOUT_MS: u32 = 2_000;
/// Headroom the assertion reserves for verifying a full page of
/// envelopes and writing the run rows.
pub const DISPATCH_HEADROOM_MS: u32 = 1_500;
/// A search run's rows are pruned once older than this. Runs are working
/// state, not a cache (§17).
pub const RUN_RETENTION_SECS: u64 = 3_600;

/// A SynOrg's own statement about itself. Unsigned app state
/// (`D-C6-12`); every field is what the spec's own setup step names.
pub struct SynOrgSettings {
    pub name: String,
    pub rules: String,
    pub area: Vec<Area>,
    pub categories: Vec<String>,
    pub support_contact: String,
    pub dispute_path: String,
    pub retention_secs: u64,
    pub publication_limits: PublicationLimits,
}

pub struct Member { pub did: String, pub note: String, pub added_at_secs: u64 }

/// One query. Every field optional; an empty query returns the newest
/// active publications, which is what a person landing on a directory
/// expects to see.
pub struct SearchQuery {
    pub text: Option<String>,
    pub categories: Vec<String>,
    pub area: Option<Area>,
    pub open_to: Option<OpenTo>,
    pub booking_mode: Option<BookingMode>,
    pub limit: Option<u32>,
}

/// One result, as a *directory* answers it.
pub struct SearchHit {
    pub listing_id: String,
    pub record_id: String,
    /// The bytes the provider signed. Never a projection (`D-C6-3`).
    pub envelope: String,
    pub issued_at_secs: u64,
    /// This directory's own clock, its claim, never used for age
    /// (`D-C6-8`).
    pub received_at_secs: u64,
    /// Which of the listing's areas matched, and how (`D-C6-5`).
    pub area_match: AreaMatch,
}

pub enum AreaMatch { NotQueried, Geometric { area_index: u32 }, Named { label: String }, NoAreaStated }

/// Why one source contributed nothing. `NotStarted` is the node's own
/// refusal to begin the call -- a 503 from guest-HTTP admission
/// (`F6d`) -- and is deliberately not folded into `TimedOut`: one says
/// this installation was busy, the other says that directory did not
/// answer, and only the second is the directory's fault. The client
/// constructs it from the HTTP status of its own request; every other arm
/// comes back inside a `query-source` response.
pub enum SourceError {
    NotFound,
    TimedOut,
    NotStarted,
    Refused { code: i64, message: String },
    Unreadable { reason: String },
}
```

`SynOrgSettings::validate` bounds every string, caps `categories` at
`MAX_CATEGORIES` and `area` at `MAX_AREAS`, reuses `Area::validate`, and
delegates `publication_limits` to `PublicationLimits::validate`.

### 4.2 `src/area.rs` — exact intersection (`D-C6-5`)

`bounding_box` is unchanged. Three new pure functions, unit tested against
the cases the existing tests already set up (10 km near Bengaluru, the
near-pole meridian fallback):

```rust
pub fn boxes_intersect(a: &BoundingBox, b: &BoundingBox) -> bool;
/// Exact, on the sieve's own over-covered candidates. `None` when either
/// side is `Named` -- named areas never match geometrically, in either
/// direction, and the caller must render that as its own reason rather
/// than as "no match" (`D-C6-5`).
pub fn areas_intersect(a: &Area, b: &Area) -> Option<bool>;
pub fn labels_match(a: &Area, b: &Area) -> bool;   // case-folded, trimmed
```

`areas_intersect` uses `f64` internally. Nothing it produces is signed, so
`D-C5-6`'s integer rule is untouched; a comment says so, because the next
reader will wonder.

### 4.3 `src/listing.rs` — one verification body (`D-C6-15`)

`verify_listing`'s body moves out of `roym_catalog` verbatim and becomes:

```rust
pub struct ListingVerdict {
    pub verified: bool,
    pub reason: Option<String>,
    pub revocation_status: Option<RevocationStatus>,   // Some only when verified
    pub listing_id: Option<String>,
    pub record_id: Option<String>,
    pub issuer: Option<String>,
    pub conversation_address: Option<String>,
    pub status: Option<ListingStatus>,
    pub issued_at_secs: Option<u64>,
    pub payload: Option<ListingPayload>,
}

pub fn verify_envelope(envelope: &str, now_secs: u64) -> ListingVerdict;
```

`catalog.listing.verify` becomes a two-line wrapper over it and **gains
`revocation_status` in its response** — a change to a C5 verb, listed in
§15 and covered by parity.

Also in this file, closing backlog row **N-2**: the payload fields that
are unbounded while their neighbours are capped —
`PaymentTerms::{payee, methods}`, `ProductDetail::{unit, sku}`,
`ServiceDetail::{includes, excludes, prerequisites}` and
`Area::Named::label` — get length and count caps in
`ListingPayload::validate`. The row's own note says why C6 is where it
lands: *"matters once C6 replicates a listing into a directory"*, and from
this slice a stranger's bytes sit on a SynOrg owner's disk.

### 4.4 `src/router.rs` — one new prefix

`("member.", DIRECTORY, MethodAuth::Owner)` joins the table.
`("directory.", DIRECTORY, MethodAuth::Owner)` is already there
(`F13`). The two existing prefix invariants keep passing unchanged.

Nothing becomes `MethodAuth::Public`: the wire exception is about a
*foreign node* reaching this service directly, not about the browser
reaching it without a session. Those are different doors and C6 opens only
the first.

### 4.5 `src/backup.rs` — five new section names

`D-C5-21`'s rule: every service that owns durable product state exports
its own bundle. `directory` owns five collections worth restoring, so it
adds **five** `SECTION_*` consts, not the two an earlier draft counted:

```rust
pub const SECTION_SYNORG: &str = "synorg";                    // the settings row
pub const SECTION_MEMBERS: &str = "members";
pub const SECTION_PUBLICATIONS: &str = "publications";
pub const SECTION_PUBLICATION_LOG: &str = "publication_log";
pub const SECTION_SOURCES: &str = "sources";
```

**Bare nouns, no service prefix**, matching every existing name
(`profile`, `contacts`, `blocks`, `reports`, `conversations`, `messages`,
`listings`, `availability` — `crates/roym_core/src/backup.rs:8-15`). An
earlier draft proposed `directory_publications` / `directory_sources`,
which would have been the first prefixed names in the set for no reason.

**`publication_log` is exported, and that is the point of listing it
separately.** `deferred-backlog.md`'s row at line 227 records exactly this
mistake one slice earlier: `catalog.export` drops `listing_history`,
`publications` and `settings`, so a restored catalog forgets its
rate-limit state and its limiter starts from zero. A directory that
forgot its publication log on restore would hand every provider a fresh
budget — a limiter that resets on restore is a limiter a determined
publisher restarts around. The precedent is cited in the code comment, not
just here.

**Not exported:** `search_index`, because it is derived and
`directory.reindex` rebuilds it (`D-C6-3`); and `search_runs`, because a
run is working state that expires (§6.1).

---

## §5 `syneroym-roym-directory` — the server half

`SCHEMA_VERSION` 1 → 2.

### 5.1 Collections

| Collection | Key | Holds |
|---|---|---|
| `settings` | `"synorg"` (single row) | `SynOrgSettings`. Its absence is what makes this installation *not* a SynOrg. |
| `members` | member DID | `Member`. Local-only (`D-C6-13`). |
| `publications` | `record_id` | The whole signed envelope, `published_by`, `received_at_secs`, and the derived `listing_id` and `issuer` for lookup. **Source of record** (`D-C6-3`). |
| `search_index` | `<listing_id>#<area_index>` | The projection (`D-C6-4`). Derived; droppable; rebuilt by `directory.reindex`. Carries `received_at_secs` so one retention filter prunes it and `publications` alike (`D-C6-23`). |
| `publication_log` | `<issuer digest>#<at_secs>#<record_id>` | One row per admitted publication, for the limiter. Pruned inside every admit (`D-C6-14`). |

Every collection is created lazily through `create-collection`, exactly as
`roym_catalog` does (`crates/roym_catalog/src/app.rs:60`) — the ungated
verb (`F1`). Declared indexes are added for readability and are known not
to be used by `query` (`F4`); a comment on `ensure_*` says so, so nobody
later concludes the queries are indexed.

### 5.2 The projection, and why it is shaped this way

On an admitted publish, for each entry of the payload's
`location.service_area` (or one row with no geometry when there is none):

```json
{
  "listing_id": "lst_…", "record_id": "rec_…", "area_index": 0,
  "issuer": "did:key:z…", "status": "active", "issued_at_secs": 1756900000,
  "categories": "|plumbing|emergency|",
  "text": "emergency plumber bengaluru south leaks taps 24h",
  "open_to": "anyone", "booking_mode": "slots",
  "min_lat_e6": 12900000, "max_lat_e6": 13100000,
  "min_lon_e6": 77400000, "max_lon_e6": 77600000,
  "area": { "kind": "circle", "lat_e6": 13000000, "lon_e6": 77500000, "radius_m": 10000 }
}
```

- `categories` is the delimited token string `F5`'s `$regex` matches
  exactly (`"|plumbing|"`). Nothing is escaped, because nothing needs to
  be: `ListingPayload` already constrains a category to `[a-z0-9-]`
  (`crates/roym_core/src/listing.rs`), so no `%`, `_` or `|` can reach
  the column. The **query** side is still normalized by the same function
  as the text path, so a malformed category token cannot smuggle a
  wildcard in from the caller.
- `text` is **normalized** `title + " " + summary + " " + categories` —
  lowercased, every character outside `[a-z0-9 -]` replaced with a space,
  runs of spaces collapsed (`D-C6-24`). The query is normalized by the
  identical function, in the same crate, so the two sides cannot drift.
  This is substring matching, not tokenized search, and it means
  searching for `50%` finds `50`; the Hub's help text says so.
  *"Free-text intent parsing"* is excluded by R1's own scope row anyway.

  **Why normalize rather than escape.** `compile_regex` emits `LIKE` with
  no `ESCAPE` clause (`filter.rs:298`), so a wildcard can only be removed
  or refused — it cannot be escaped, whatever an earlier draft of this
  plan said. Normalizing both sides in Rust also does the case fold
  before SQLite sees it, which matters because SQLite's default `LIKE`
  folds ASCII only: an unnormalized non-ASCII title would be
  case-sensitive.
- The four bbox columns come from `area::bounding_box`. They are absent
  for a `Named` area and for a listing with no `location` block.
- `area` is the original, for `D-C6-5`'s exact refinement.

### 5.3 Verb table

| Method | Wire | Does |
|---|---|---|
| `directory.settings` | local | Reads `SynOrgSettings`, or `null` when this node runs no SynOrg. |
| `directory.set-settings` | local | Validates and writes them. Creating them is what turns this installation into a SynOrg. |
| `directory.info` | **Open** | The public subset of settings — name, rules, area, categories, support contact, dispute path, retention — plus a `member_count` and the node's own directory DID. No roster (`D-C6-13`). |
| `member.add` / `member.remove` / `member.list` | local | The unsigned roster (`D-C6-12`). |
| `directory.publish` | **VerifiedOnly** | §5.4. |
| `directory.unpublish` | local | Keyed on **`listing_id`**, not `record_id`: a directory holds one version per listing (§5.4 step 8), and an owner removing an offer means the offer, not one signature of it. Deletes the `publications` row and every `search_index` row for that id. Never touches the provider's own copy and never claims to (journey step S9). |
| `directory.publications` | local | The owner's review list, newest first. |
| `directory.search` | **Open** | §5.5. |
| `directory.limits` / `directory.set-limits` | local | `PublicationLimits`, mirroring `listing.limits` / `set-limits` exactly (`D-C6-14`). |
| `directory.reindex` | local | Rebuilds `search_index` from `publications`. |
| `directory.export` / `directory.import` | local | `D-C5-21`'s bundle, sections per §4.5. |

Plus the client half's verbs, §6.

### 5.4 `directory.publish` — the one flow that admits a stranger's bytes

Params: `{ "envelope": "<the signed listing envelope, verbatim>" }`.

1. `admit::admit` has already returned `Caller::Verified(did)`
   (`D-C6-7`); the DID becomes `published_by`. **A caller whose own node
   holds no valid instance certificate never gets here** — it arrives
   `Anonymous` and is refused (`F6c`), which is why §11.2 and §11.3 both
   cover that case explicitly rather than assuming `verified` arrives.
2. `listing::verify_envelope(&envelope, now)`. A verdict that is not
   `verified` is refused with `-32602` and the verdict's own reason —
   the refusal is visible to the sender, which is failure-matrix row 12's
   requirement.
3. `payload.status` must be `active` or `withdrawn`. A `draft` is refused
   with a reason (`D-C6-25`): `directory.search` filters on `active`, so
   a stored draft would consume budget and index rows while being
   unreachable by any query.
4. The payload's `conversation_address` must be non-empty. It always is
   (required in the signed payload since C5), and checking it here is what
   guarantees the consumer can act on a search result with no further
   lookup — the whole point of `D-C5-8`.
5. **Withdrawal path.** If `status == withdrawn`, delete the stored
   publication for that `listing_id`, delete **every** `search_index` row
   for it, consume no budget (`D-C6-14`), and return.
6. **The limiter.** `publication_log` rows for this issuer inside the
   window → `safety::admit_publication`. `Admission::RateLimited` is
   refused with a `retry_after_secs` the caller can act on.
7. **Three prunes, in the one pass that already touches this data.**
   `delete-many` on `publication_log` for rows older than the limiter
   window; and `delete-many` on `publications` **and** on `search_index`
   for rows whose `received_at_secs` is older than the SynOrg's
   `retention_secs` (`D-C6-23`). The second pair is why `received_at_secs`
   rides the index row (§5.1) — one filter, two collections, no lookup in
   between.
8. **Replace the prior version, index rows included.** Delete every
   `search_index` row for this `listing_id` **before** writing the new
   ones, then store the envelope verbatim in `publications` keyed by
   `record_id` with `received_at_secs = clock::now_secs()`, replacing any
   prior row for the same `listing_id`.

   > **This deletion is not optional and its absence is a real bug.** Index
   > rows are keyed `<listing_id>#<area_index>`, so a listing republished
   > with fewer service areas leaves the surplus rows behind, pointing at a
   > `record_id` no longer in `publications`. `directory.search` step 6
   > would then read an envelope that is not there, and the stale row can
   > win step 4's `AreaMatch` precedence and displace the live one.
   > `delete-many` filtered on `listing_id` is the whole fix, and §11.2's
   > scenario 84b republishes with fewer areas so the fix cannot regress.

9. Write the new `search_index` rows.
10. Append the `publication_log` row.

**A directory never pulls.** There is no fetch, no crawl, no refresh
timer, and no verb that takes a provider's address. The only way a listing
enters a directory is the provider sending it (`S7`).

### 5.5 `directory.search` — one directory answering

Params: a `SearchQuery`. Answer: `{ "hits": [SearchHit], "truncated": bool,
"directory": "<this node's directory DID>", "answered_at_secs": … }`.

1. Compile one filter document: `status = "active"`, plus `$regex` on
   `categories` per requested category (all of them, `$and`), plus
   `$regex` on `text`, plus `open_to` / `booking_mode` equality, plus —
   when the query carries a geometric area — the four bbox predicates
   from `bounding_box(query_area)` intersecting the row's box.
2. Page `search_index` with `limit: 500`, following the cursor until
   `next-cursor` is `none` **or** the guest has collected
   `MAX_SEARCH_RESULTS * 4` candidate rows. The data-layer's own note
   requires paging until `next-cursor` is `none` rather than stopping at a
   short page; the extra ceiling is what stops one query walking a large
   directory, and it sets `truncated: true`.
3. Refine each candidate exactly (`D-C6-5`), recording the `AreaMatch`.
4. Collapse to one hit per `listing_id`, keeping the row whose
   `AreaMatch` is `Geometric` over one that is `Named` over
   `NoAreaStated`.
5. Sort by `(issued_at_secs desc, listing_id asc)` and take
   `min(limit, MAX_HITS_PER_QUERY)` — this is one directory's own answer,
   so the per-source and merged caps (`D-C6-18`) do not apply here; they
   apply in `directory.merge`.
6. Read each kept row's envelope from `publications` and return it
   verbatim.

**The directory never verifies anything on the consumer's behalf, and the
response carries no verdict field at all** (`D-06C-6c`). A field that said
`verified: true` here would be exactly the assertion failure-matrix row 2
forbids, and its absence is a fact a test asserts (§11.2).

---

## §6 `syneroym-roym-directory` — the client half

Separate collections, separate method names, same service (`D-C6-2`,
`D-C6-20`). **The loop is the client's; the node does one bounded step per
call** (`D-C6-9`, forced by `F6b`).

| Collection | Key | Holds |
|---|---|---|
| `sources` | directory DID | `{ did, label, added_at_secs, last_ok_secs, last_error }`. Capped at `MAX_SOURCES`. |
| `search_runs` | `<run_id>#<record_id>` | One row per stored hit: **the projection `merge` will return**, this node's verdict, the envelope verbatim (for `run-envelope` alone), the source, and `at_secs`. Working state, pruned at `RUN_RETENTION_SECS`. |
| `runs` | `<run_id>` | The run marker `start-run` writes: `at_secs` and the source list it handed out. What `query-source` validates against. |

**The projection is computed once, at `query-source` time.** The envelope
is already parsed there, to verify it — so parsing it again in `merge`
would be the same work done twice, and `merge` would be doing it up to
`MAX_SOURCES × MAX_STORED_PER_SOURCE` times inside one five-second
dispatch to emit at most fifty rows. `merge` therefore reads projections
and never touches an envelope; the envelope is read one at a time by
`run-envelope`. The epoch argument that shaped `query-source` (`F6b`)
applies to `merge` too, and this is what applies it.

| Method | Wire | Does |
|---|---|---|
| `directory.add-source` | local | Adds a directory DID by hand (`D-C6-16`). Probes it once with `directory.info` and stores the returned name as the label. The source is added either way, and the probe's outcome is reported: a failure is stored as `last_error`, and a probe that succeeds but returns `null` is reported as *"this address answered, but runs no directory"* (`D-C6-21`) rather than stored as a silent no-op. |
| `directory.remove-source` | local | Removes it. |
| `directory.sources` | local | Lists them, with `last_ok_secs` and `last_error`. |
| `directory.start-run` | local | Mints a `run_id`, writes the run marker, prunes runs older than `RUN_RETENTION_SECS`, and returns `{ run_id, sources: [did], max_concurrency }` — the list the client loops over **and the concurrency it must honour** (`D-C6-26`). The node owns that number because the node owns the limit it is derived from. |
| `directory.query-source` | local | **One source, one proxy call, one dispatch.** §6.1. |
| `directory.merge` | local | Reads the run's rows, merges, applies the caps. §6.3. No I/O. |
| `directory.run-envelope` | local | `{ run_id, record_id }` → the one envelope, for when a person opens a result. Keeps the merged page small (§6.4). |

### 6.1 One source, one dispatch (`D-C6-9`, `D-C6-19`)

```
directory.query-source { run_id, source, query }        (internal only)
  ├─ refuse unless `run_id` names a run this node minted (`runs` row)
  ├─ refuse unless `source` is in this person's own `sources` collection
  ├─ proxy.call(Service(<source>), DIRECTORY.interface, "invoke",
  │             {"method":"directory.search","params":{…}},
  │             { timeout_ms: DEFAULT_SOURCE_TIMEOUT_MS, idempotent: true })
  ├─ verify every returned envelope here, on this node (§6.2)
  ├─ write up to MAX_STORED_PER_SOURCE verified rows and
  │     MAX_REFUSED_RESULTS refused ones: projection + verdict + envelope
  └─ answer { source, verified: n, refused: m, error: option<SourceError> }
```

**Both refusals are two lines and they are not decoration.** The client
supplies `source`, so without the check `MAX_SOURCES` bounds neither what
a run contacts nor how many rows it writes — the two things `D-C6-19` and
§4.1 say it bounds. It is owner-gated, so this is not an authorization
hole; it is that §7.1's *"the one service that talks to strangers"*
boundary would be **caller-steered** rather than fixed, which is a weaker
property than the one that section claims. Validating `run_id` the same
way stops a client writing rows into a run the node never minted.

`idempotent: true` is correct and deliberate: a search is a read, and a
transport retry cannot double anything. No `idempotency-key` — the target
keeps no record to fence.

**Why the deadline is 2 s and not a preference.** This dispatch must
finish inside `dispatch_epoch_timeout_secs`, 5 s by default, and that
budget is wall-clock and is spent while the guest waits on the call
(`F6b`). `DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS` must stay
under it, and a unit test asserts exactly that rather than trusting two
constants in different crates to stay in step.

**The client's loop, and the bound on it.** The Hub calls `start-run`,
then issues `query-source` for the returned sources **concurrently, at
most `max_concurrency` in flight**, then calls `merge`. `roymctl` does the
same with `tokio`. Nothing shared is contended: each call writes rows
under its own `(run_id, record_id)` key.

The bound is the node's, not the client's invention (`D-C6-26`,
`F6d`): `web`'s guest-HTTP admission is four concurrent with a two-second
wait before a 503, `DEFAULT_SOURCE_TIMEOUT_MS` is also two seconds, and
each in-flight source holds two Wasmtime instances against a pool of ten
that fails hard rather than queuing. Three in flight is six instances and
leaves one admission permit for the rest of the Hub. A client that ignores
`max_concurrency` does not corrupt anything — it gets 503s, which is why
`SourceError::NotStarted` exists and why the Hub retries those sources
after the run's other calls drain.

So the spec's *"queries each directory it has been given, in parallel"* is
satisfied literally, and an earlier draft's proposal to reword the spec is
**withdrawn**. "In parallel" was never going to mean "all at once with no
ceiling" on a node with a four-permit door.

A source that fails contributes an error and no rows, and removes nothing
another source returned. **A run with zero sources succeeds**: `start-run`
returns an empty list, the client makes no `query-source` call, and
`merge` answers zero hits — which is the shape `D-06C-6a` needs. A person
with no directory gets an empty answer, never an error.

### 6.2 Verification, on the consumer's own node (`D-06C-6c`)

Inside `query-source`, for every hit the source returned:
`listing::verify_envelope(&hit.envelope, clock::now_secs())`. The verdict
is stored on the run row and returned by `merge` per hit:

| Field | Value |
|---|---|
| `verified` | The consumer's own verdict. Never the directory's. |
| `reason` | Present only when `verified` is false. |
| `revocation_status` | `"unknown"` in R1, always (`D-C6-15`). Never absent — an absent field is what a UI turns into a positive default. |
| `credential` | `"unknown"` in R1, always. C6 checks no membership credential because none exists (`D-06C-6`'s split). The field is present so the Hub renders unknown rather than nothing. |
| `listing_id`, `issuer`, `conversation_address`, `status` | From the verified payload, never from the directory's own framing. |
| `age_secs` | `consumer_now − issued_at_secs` (`D-C6-8`). |
| `sources[]` | `{ directory, record_id, received_at_secs }`, one per source that answered for this `listing_id`. |
| `versions_differ` | `D-C6-10`. |

A hit that fails verification is **kept, in its own list** (`D-C6-18`). It
is never dropped — the person asked which directories said what, and
silently deleting a forged answer hides that a directory served one — and
it is never in `hits[]`, for the reason `D-C6-18` gives.

### 6.3 Merge (`D-C6-10`, `D-C6-11`, `D-C6-18`)

**Per source, first.** Group that source's stored projections by `listing_id`,
sort by `(issued_at_secs desc, listing_id asc)`, and take at most
`MAX_HITS_PER_SOURCE`.

**Then across sources.** Round-robin: take the next unseen `listing_id`
from each source in turn until `MAX_SEARCH_RESULTS` is reached or every
source is exhausted. Sources are visited in DID order, so the result is
reproducible on both builds. For a `listing_id` more than one source
returned, keep the envelope with the greatest signed `issued_at_secs`,
ties by `record_id` ascending; union the `sources[]`; set
`versions_differ` when the kept `record_id` is not the only one seen.
**Never take a field from one envelope and a field from another.**

**Refused hits.** Grouped by `listing_id` the same way, so one forged
listing served by three directories is one row naming three sources,
sorted by `(directory, listing_id)` — never by anything the forger
controls — and capped at `MAX_REFUSED_RESULTS`.

`truncated` is reported per list, so a person can tell a truncated result
page from a truncated refusal list.

**Two independent caps and a per-source share are three rules doing one
job**, and all three are needed — `D-C6-18` has the argument, including
why splitting the lists alone leaves the attack standing in signed form.

### 6.4 What a merged page weighs

`merge` returns **projections, not envelopes** — and reads only
projections too, because `query-source` stored them (§6.1). A projection
is `listing_id`, `record_id`, `issuer`, `title`, `summary`, `categories`,
`conversation_address`, `status`, the verdict fields, `age_secs` and
`sources[]`. The envelope stays in `search_runs` and is fetched one at a
time by `directory.run-envelope` when a person opens a result.

This is a size decision, not a style one. A guest HTTP response is hard
capped at `MAX_GUEST_RESPONSE_BODY_BYTES = 1 MiB`
(`crates/router/src/route_handler/http.rs:118`, enforced at `:605`), and a
listing's `summary` alone may be 2 KiB (`MAX_SUMMARY_LEN`,
`crates/roym_core/src/listing.rs:19`). Fifty verified plus twenty refused
**envelopes** would sit uncomfortably close to that ceiling and would
exceed it for listings near their field limits — a failure that would
appear as a broken page for the person with the most results, which is
the worst population to break it for. Projections put the worst case
around 250 KiB. §16 item 16 states the arithmetic as a thing to keep true.

## §7 The other five services

- `catalog`: `listing.verify` becomes a wrapper over
  `roym_core::listing::verify_envelope` and gains `revocation_status`
  (`D-C6-15`). `listing.publish-to` is **not** added — see below.
- `catalog` and `conversation`: the C5-7 read-verb fix (§8).
- `profile`, `transaction`, `web`: unchanged. `require_internal` stays as
  C5 left it on all five.

One addition, on the client half, named here because it belongs to
`directory` rather than to `catalog`:

| Method | Wire | Does |
|---|---|---|
| `directory.publish-to-source` | local | Takes `{ source, listing_id }`, reads the signed envelope from `catalog` through a **new declared `directory → catalog` edge**, and calls `directory.publish` on the named source. Returns the source's own answer, refusal included. |

### 7.1 One service talks to strangers, and a test says so

Putting this verb on `catalog` with a `catalog → directory` edge is a
defensible alternative — it keeps *"the provider's offer"* verbs
together. It is refused for a reason stronger than tidiness:
**concentrating every call to a foreign node in one service is a boundary
that can be checked.** With `publish-to-source` here, `directory` is the
only Roym service that talks to a stranger's node, in either direction —
outbound through the fan-out and this verb, inbound through `admit`'s
three-method table.

So the boundary becomes an invariant rather than a convention:

> A test asserts that `CallTarget::Service` appears in no `crates/roym_*`
> crate except `roym_directory`. Every other cross-service call in the
> product is `CallTarget::Dependency`, which the host resolves inside this
> app instance and which cannot reach another node.

Whoever moves this verb must move the edge with it — **never declare
both**, which would be a cycle in the declared graph.

### 7.2 The confused-deputy invariant, stated because C6 is where it becomes reachable

`deferred-backlog.md`'s row at line 95 records that
`CallerOrigin::Internal` says a call arrived through a *local dispatch
path*, not that the *chain* began locally — so a wire caller reaching a
service that then proxies to a sibling would present as `internal` at that
sibling. C5 avoided it by construction, having no wire-reachable verb at
all.

**C6 is the first slice where one service has both wire-reachable verbs
and an outbound edge to a sibling.** The design still closes it, but now
by choice rather than by absence, so the choice is written down and
tested:

> **No wire-reachable method may cause a call to a sibling.**
> `directory.search`, `directory.info` and `directory.publish` touch only
> `directory`'s own collections. `directory.publish-to-source` — the one
> verb that traverses `directory → catalog` — is local-only.

A test enumerates `WIRE_REACHABLE` and asserts that none of those handlers
reaches `AppProxy::call`. Leaving this as a happy accident is what turns
it into a regression the first time somebody adds a convenience lookup to
`directory.search`; the backlog row stays open, because the general
capability gap is the substrate's, and this slice's own invariant is what
changes.

This is `S7` — the provider's own action — and it needs an edge, so §9
adds it in the same change as the call that traverses it (`D-C5-9`'s
rule).

---

## §8 The two backlog fixes C6 inherits

### 8.1 The whole-collection read verbs (C5 review C5-7)

The row's pickup trigger — *"before C6 puts a search on top"* — has fired.
Each named call site gains a `QueryOptions.filter` so the host does the
sieving instead of the guest:

| Site | Filter |
|---|---|
| `roym_conversation`'s `messages_of` (`app.rs:617`) | `{"conversation": <id>}` |
| `roym_conversation`'s `list` (`app.rs:483`) | unchanged shape; the cap and the cursor loop are what change |
| `roym_catalog`'s `list_listings` (`app.rs:585`) | `{"status": …}` when the caller asked for one |
| `roym_catalog`'s `listing_history` (`app.rs:648`) | `{"listing_id": <id>}` — today it parses every envelope to match one id |
| `roym_catalog`'s `publication_secs_in_window` (`app.rs:436`) | `{"at_secs": {"$gt": floor}}`, plus the `delete-many` prune `D-C6-14` adds on the directory side, applied here too |

**And the row is restated, not closed** (§15): `F4` shows a filter does not
make the query use an index, so what these fixes buy is fewer rows crossing
the host boundary and less guest memory — real, and not what the row's
title implies.

### 8.2 `conversation.search`'s unindexed scan

Re-targeted, not closed. `D-C6-1` means C6 builds no FTS5, so the row's
premise (*"FTS5 over the same copy is C6's work"*) is wrong. §15 rewrites
it into one row with the FTS5 blocker, targeted at the M6 substrate spec,
with the trigger *"a lifecycle or scoped-DDL path exists for a service to
provision and read its own virtual tables"*.

---

## §9 The manifest

One new edge:

```toml
[services.directory]
# The provider publishes their own listing to a directory they chose, so
# the directory service reads the signed envelope from the service that
# holds it.
depends_on = ["catalog"]
```

**No `visibility` and no `topology_visibility` value changes** (`F14`,
`D-C5-4`'s reasoning restated): `directory` is already `public` + `open`,
which is what failure-matrix row 18 needs, and with `admit::admit` in
force the manifest fields are a discoverability choice, not an
authorization control. `status.md` says this out loud again so a reader
does not conclude from the diff that a manifest field started protecting
the API.

`init_roym` (`crates/substrate/src/runtime.rs`) persists the new binding
beside the two C5 added. `crates/roym_core/src/router.rs`'s
`every_declared_dependency_names_a_sibling_and_the_two_edges_are_present`
test grows to three edges — and is renamed to stop counting in its own
name.

---

## §10 `roymctl`

- `roym enrol-signing` / `roym signing-status` are **unchanged**:
  `directory` signs nothing (`D-C6-12`), so it needs no certificate and
  adding one would make the ceremony claim something false.
- **New: `roym directory`**, a small group that drives the same JSON-RPC
  API the Hub uses, through the gateway — which is exit criterion 2's
  *"a second client drives the same flow through the same API with no UI
  involved"*, for R1 row 5:
  - `roym directory sources` — list.
  - `roym directory add <did> [--label]` / `remove <did>`.
  - `roym directory find [--text] [--category …] [--near lat,lon,radius_m] [--limit]`
    — the **client's own loop**, which is where the loop belongs
    (`D-C6-9`): `start-run`, then one `query-source` per source **run
    concurrently with `tokio`**, then `merge`. This is exit criterion 2's
    second client driving the same three verbs the Hub drives, and it is
    also the proof that the parallel fan-out is not a browser-only
    property. Prints one line per result with issuer, age, source, and
    the verification verdict **including the two unknowns**, because a CLI
    that prints only the good news is the same failure the UI rule is
    about. Prints `refused` and `source errors` as their own blocks
    (`D-C6-18`).
  - `roym directory publish <listing-id> --to <did>`.
  - `roym directory info <did>`.
  - `roym directory serve --name … --rules-file … --category … --support … --dispute … --retention-days …`
    — writes `SynOrgSettings`, i.e. journey step **S2**, and
    `roym directory member add|remove|list` for S4–S6's roster half.

`--near` parses a decimal and converts to micro-degrees at the boundary;
nothing decimal reaches a payload (`D-C5-6`).

---

## §11 Tests

### 11.1 What each suite is for

| Suite | Proves |
|---|---|
| Unit, `roym_core` | The rules with no host: `admit`'s table, `areas_intersect`'s exactness, `verify_envelope`'s verdicts, `SynOrgSettings::validate`, the projection builder, the merge. |
| Unit, `roym_directory` | The verb bodies against C4's `TestHost`, including the limiter and the reindex. |
| `crates/roym_web/tests/dual_build_parity.rs` | That both builds answer identically — including the new wire arms, which is where the two builds have most room to diverge. |
| `crates/app_host_native/tests/dual_build_parity.rs` | **The shim's own suite**, one added scenario: a guest-HTTP request at the dual-build fixture reports the same invocation origin on both builds (`D-C6-17`). It belongs here, not in Roym's suite, because the divergence is the shim's and no Roym component reads the origin in an HTTP handler. |
| `crates/substrate/tests/roym_directory_e2e.rs` (new) | The reference scenario across **three** real substrates, over real transports, with a real registry. |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | That the person sees source, age, and unknown — the half of R1 row 5 no Rust test can assert. |

### 11.2 Parity scenarios (74 onward)

Harness changes first:

1. **A wire-flavoured foreign target.** `did:key:hForeignWire` routes to
   the directory through `engine.execute_wasm_json_from_wire` /
   `NativeDirectory` behind a `host_for_wire`-built host, carrying a
   verified caller; `did:key:hForeignAnon` does the same with an
   unauthenticated caller. `did:key:hForeign` keeps its current local
   routing so no existing scenario changes (`F10`).
2. **A second directory target**, `did:key:hForeignWire2`, holding its own
   store, so merge and disagreement can be driven at all.
3. `WIRE_REFUSED_VERBS` is left **unchanged**, `directory.ping` included.
   `F11` raised the question; the answer is that `ping` is not in the
   exception table (§3), so it stays refused and scenario 67 keeps
   asserting exactly what it asserts today. The new reachability is
   additive, not a replacement — which is itself worth knowing, because a
   suite that had to be edited to accommodate a widening is a suite that
   stopped guarding the old rule.
4. A `WIRE_REACHABLE_VERBS` constant mirroring the service's own table,
   asserted equal to it by a unit test so the suite cannot drift from the
   rule.

| # | Scenario | What only its own handler can produce |
|---|---|---|
| 74 | `directory.set-settings` then `directory.settings` round-trips every field | the stored `retention_secs` and the category list |
| 75 | `set-settings` refuses an out-of-range area and an over-long rules text | the specific `AreaError` / validation reason |
| 76 | `directory.info` over the wire, both arms, returns name/rules/dispute path and **no roster** | `member_count` present, `members` key absent |
| 77 | `member.add` / `list` / `remove` round-trip locally | the stored note |
| 78 | `member.list` over the wire answers `-32013` on both arms | the code, and that it is identical to an unlisted method's |
| 79 | `directory.publish` from a verified wire caller stores the envelope byte-for-byte | `publications` row's envelope equals the input string |
| 80 | `directory.publish` from an anonymous wire caller answers `-32013` | the code |
| 80b | **A caller whose node holds no valid instance certificate arrives anonymous and is refused identically** — the refusal names no cause, so the scenario asserts the caller cannot tell it from "not yours to call" (`F6c`) | the byte-identical refusal for the two causes |
| 81 | `directory.publish` of a tampered envelope is refused with the verdict's own reason | the reason string from `verify_envelope` |
| 82 | `directory.publish` of a listing whose `listing_id` is not derivable from its issuer is refused | that specific reason |
| 83 | The publication limiter refuses past the budget with a usable `retry_after_secs` | the `retry_after_secs` value |
| 84 | A `withdrawn` publication consumes no budget and clears the index | budget unchanged; zero `search_index` rows |
| 84b | **Republishing with fewer service areas leaves no stale index rows.** A five-area listing republished with two leaves exactly two rows, and every surviving row's `record_id` is present in `publications` (§5.4 step 8) | the row count and the referential check |
| 84c | A `draft` listing is refused at publish, stores nothing and consumes no budget (`D-C6-25`) | the refusal reason and the unchanged budget |
| 84d | **`retention_secs` is enforced**: a publication older than the SynOrg's retention is gone from `publications` **and** from `search_index` after the next publish, and disappears from search (`D-C6-23`) | zero rows in both collections, and the empty search |
| 85 | `publication_log` is pruned inside the admit | row count after a publish past the window |
| 86 | `directory.search` by one category returns the listing; a category the listing lacks returns none | the `listing_id` |
| 87 | Search by free text matches title and summary, case-insensitively | the `listing_id` |
| 88 | Search by a bounding box that intersects the listing's own box returns it | `area_match.Geometric{area_index}` |
| 89 | **A box inside the over-covered circle projection but outside the true circle returns nothing** — the refinement, not the sieve | zero hits where a sieve-only build returns one |
| 90 | A listing stating only a named area is not returned by a geometric query, and **is** returned by a matching label query, with `AreaMatch::Named` | the `area_match` arm |
| 91 | A listing with no `location` block is returned by a query with no area and not by a geometric one | `AreaMatch::NoAreaStated` |
| 92 | Search results are ordered newest-first with the `listing_id` tiebreak | the exact order of three listings sharing the pinned clock |
| 93 | **A search response carries no verification verdict** — `verified`, `revocation_status` and `credential` are absent from a directory's own answer | the absence, asserted key by key |
| 94 | `directory.search` over the wire, anonymous, succeeds | the hits |
| 95 | `directory.reindex` rebuilds the projection after `search_index` is emptied | identical hits before and after |
| 96 | Client half: `add-source` / `sources` / `remove-source` round-trip, and `MAX_SOURCES + 1` is refused | the stored label and the refusal |
| 96b | `add-source` against a node running no SynOrg stores the source **and** reports that it runs no directory (`D-C6-21`) | the reported probe outcome, distinct from an error |
| 97 | `start-run` → `query-source` per source → `merge` over two sources yields one hit per `listing_id` with two `sources[]` entries | the two-entry array |
| 97b | **One `query-source` dispatch makes exactly one proxy call** — the harness's proxy counter increments once, whatever the source returns (`D-C6-9`) | the counter delta |
| 97c | **`DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS` is inside `dispatch_epoch_timeout_secs`** — a unit assertion, not a timing test, so it fails at build time rather than as an intermittent trap (`D-C6-19`, `F6b`) | the arithmetic |
| 97d | **`MAX_CLIENT_CONCURRENCY < max_concurrent_guest_http_per_service`**, asserted against `syneroym_core::config`'s own default rather than a copied number, and `start-run` returns that value (`D-C6-26`, `F6d`) | the arithmetic, and the returned field |
| — | **`SourceError::NotStarted` has no parity scenario, deliberately.** It is constructed by the *client* from the HTTP status of its own request; every other arm arrives inside a `query-source` response. The harness drives verbs through `both_rpc`, not through HTTP, so it cannot produce the condition on either build — a scenario would have to fabricate the arm, which would assert nothing about either stack. It is covered where it can happen: browser case 23b. | — |
| 97e | `query-source` refuses a `source` not in this person's own `sources`, and a `run_id` this node did not mint | two refusals, and zero `search_runs` rows written |
| 97f | A `query-source` call stores at most `MAX_STORED_PER_SOURCE` verified rows from a source that returned a full page, and `merge` reads no envelope (§6.1, §6.4) | the stored row count, and a merge driven against rows whose `envelope` field is blanked |
| 98 | Two sources returning different versions of one `listing_id` keep the newer envelope and set `versions_differ` | `versions_differ: true` and the kept `record_id` |
| 99 | A source that answers `ServiceNotFound` yields an error and no rows, and removes nothing another source contributed | both outcomes in one merged answer |
| 100 | A source that times out does the same, and `last_error` is stored on that source row | the stored `last_error` |
| 101 | **A run with zero sources succeeds with zero hits and no error** — `D-06C-6a` at verb level | `hits: []`, no `error` key |
| 102 | Every hit carries `verified`, `revocation_status: "unknown"`, `credential: "unknown"` and `age_secs`, and a forged hit appears in `refused[]`, never dropped | the four fields, and the forged row's presence in `refused[]` |
| 102b | **A source returning nothing but forgeries reduces `hits[]` by nothing.** Source A returns a full page of forged listings stamped `now`; source B returns ten good ones; all ten are in `hits[]` (`D-C6-18`) | `hits.len() == 10`, and the identical answer with source A absent |
| 102c | **A source returning nothing but *validly signed* recent listings also reduces `hits[]` by nothing** — the per-source share, which is what the split lists alone do not fix (`D-C6-18`, `D-C6-11`) | source A contributes exactly `MAX_HITS_PER_SOURCE`, and B's results survive |
| 102d | The round-robin order is identical on both builds, and `hits[]` / `refused[]` truncate independently | the exact merged order, and two `truncated` flags with different values |
| 102e | `merge` returns projections and no envelopes; `run-envelope` returns the one envelope byte-identical to what the source served (§6.4) | the absent `envelope` key, and the byte-equal fetch |
| 103 | `catalog.listing.verify` now returns `revocation_status`, and its verdict is identical to the directory client's for the same envelope | the two verdicts compared field by field |
| 104 | `directory.publish-to-source` reads the envelope through the declared edge and returns the source's own refusal unchanged | the propagated `retry_after_secs` |
| 105 | `directory.export` / `import` round-trip publications and sources, a tampered bundle is refused, and `reindex` restores search after import | identical hits after import |
| 106 | The C5-7 fix: `listing.history` for one `listing_id` returns only that listing's versions on a store holding two | the version count |
| 106b | **Every client-half verb is refused over the wire** — `start-run`, `query-source`, `merge`, `run-envelope` — while `directory.search` is not (`D-C6-20`) | `-32013` for the four, hits for the fifth, in one scenario |
| 107 | **The guard** (`D-C5-13`'s successor): every verb C6 adds, driven locally, answers neither `-32601` nor `-32013`; and every verb driven over the wire answers `-32013` **unless** it is in `WIRE_REACHABLE_VERBS`, in which case it does not | the two-directional assertion |
| 108 | **The stranger boundary** (§7.1): a source-level assertion that `CallTarget::Service` appears in no `crates/roym_*` crate but `roym_directory` | the grep result, as a test |
| 109 | **The confused-deputy invariant** (§7.2): every method in `WIRE_REACHABLE`, driven over the wire, makes zero proxy calls — asserted on the harness's proxy counter, so a convenience lookup added to `directory.search` later fails here | the counter delta of exactly zero |

Scenario 107 is the one that must not be skipped: `D-C5-13`'s guard proved
that nothing was *missing*; C6's must also prove that nothing was
accidentally *opened*.

### 11.3 `crates/substrate/tests/roym_directory_e2e.rs` — new

Three substrates on one shared registry, reusing
`roym_conversation_e2e.rs`'s `Node` (`:213`) with a third instance. Steps
map onto the reference scenario by number:

| Step | Reference | Asserts |
|---|---|---|
| 1 | 1 | Three nodes boot, deploy Roym, enrol signing. |
| 2 | 2 | Z creates the SynOrg through `directory.set-settings`; `directory.info` from X reads it back over a real transport. |
| 3 | 3 | Y creates a profile and a signed listing, in no directory. |
| 4 | **4** | **X reaches Y by direct link with no directory in the path**: `listing.verify` on Y's envelope, `conversation.open` on the address inside it, one message delivered. Runs **before** any publication exists anywhere. `D-06C-6a`. |
| 5 | 5 | Z adds Y to the roster (`member.add`). No credential is issued: C9's. |
| 6 | 6 | Y publishes to Z through `directory.publish-to-source`, over the wire, verified. |
| 7 | 7 | X adds Z as a source and runs the client loop — `start-run`, one `query-source`, `merge`. Asserts: one hit; `run-envelope` returns bytes identical to Y's; `sources[0].directory == Z`; `age_secs` computed on X's clock; `revocation_status == "unknown"`; `credential == "unknown"`. |
| 7b | 6 | **The certificate dependency, over a real transport** (`F6c`): with Y's `catalog`/`directory` instance certificate absent or expired, `directory.publish-to-source` fails with `-32013` and the operator-visible cause is the certificate, not the directory's policy. This is the case §16 item 12 would otherwise read as impossible. |
| 8 | 7 | X starts a conversation **from the search result's own `conversation_address`**, with no prior contact entry — closing the loop `D-C5-8` opened. |
| 9 | 12 | An anonymous wire caller (a raw JSON-RPC frame with no delegation) reaches `directory.search` and succeeds, and reaches `member.list` and gets `-32013`. |
| 10 | 12 | Y publishes past the limit and is refused with a `retry_after_secs`, visible to Y. |
| 11 | 17-adjacent | Z runs `directory.unpublish`; X's next search no longer returns it, **and X's already-held copy is untouched** — the product does not claim instant removal. The signed suspension itself is C9's; this step proves the removal half of matrix row 15 that R1 can reach. |
| 12 | 4 again | The **no-directory regression**: X removes Z as a source, and step 4's whole path is re-run and passes. This is the test `D-06C-6a` demands, and it runs at the **end**, after a directory has existed, which is when it is actually at risk. |
| 13 | — | Two directories: a second SynOrg on X's own node holds an older version of Y's listing; a search over both merges to one hit with `versions_differ: true`. |
| 13b | — | **The loop at its ceiling, over real transports**: X holds `MAX_SOURCES` sources, of which only two are live and the rest are unreachable DIDs. The run completes, the two contribute, the rest carry an error each, and nothing is reported as a directory failure that was this node's admission limit (`D-C6-26`). |
| 14 | — | Export / import of the directory bundles on Z, then `reindex`, then an identical search result. |

**The e2e uses the existing `resume()` (redeploy-after-restart) path, and
must not be written as if a bare restart rehydrates routes** — see §15's
note on the M6-spec backlog row.

### 11.4 Browser cases in `roym-hub.spec.ts`

| # | Case |
|---|---|
| 13 | Directory tab: adding a source by DID lists it with its label; removing it empties the list. |
| 14 | A search with results shows, per result, the **source directory**, the **age**, and both unknowns spelled as words — "revocation: unknown", "membership: not checked" — and never the word "verified" without a qualifier. Refused results render in their **own block**, below the results and never mixed into them (`D-C6-18`). |
| 15 | A result whose signature does not verify renders as refused evidence with its reason, is visually distinct from a verified one, and is not clickable through to a conversation. |
| 16 | A malicious listing title, summary and category arriving **through a directory** render as literal text: no element created, no request made. (C5's case 11 proved this for a locally authored listing; C6 is the first slice where the bytes came from a stranger's machine.) |
| 17 | A search with no sources added shows the empty state that says a directory is optional and how to reach a provider by direct link — the UI half of `D-06C-6a`. |
| 18 | A source that errored shows its error beside it, and the other sources' results still render — the error does not replace the page. |
| 19 | SynOrg tab: creating a SynOrg writes settings and the rules text renders as text, not markup. |
| 20 | **SynOrg tab: the publication limit is an editable control on the same screen, not a hidden default** (`D-C6-22`). Raising it lets a refused publisher succeed on the next attempt, in one flow, with no CLI. |

| 21 | **Listings tab: a provider chooses a directory and publishes a listing to it** — journey step **S7**, the step this slice is named after — and a refusal (over the limit, or a draft) is shown with its reason. |
| 22 | The results view renders progressively: results from a source that has answered appear while a slow source is still outstanding (`D-C6-9`'s parallel loop, visible). |
| 23 | **A run at the full `MAX_SOURCES`** — eight sources, several deliberately slow — completes with every source accounted for, **no 503 reaching the person**, and no source reported as timed out when this node was the one that was busy (`D-C6-26`). Run at eight, not at two: four is where the admission limit begins to bite and two would never reach it. |
| 23b | **`NotStarted` is not `TimedOut`.** With `max_concurrency` deliberately ignored so the client oversubscribes the admission limit, a source refused by this node carries the node-side arm and the directory's own `last_error` is left untouched — this installation being busy is not that directory's fault (`D-C6-26`). Drives real HTTP, which is the only place the condition exists. |

Case 21 closes a gap an earlier draft left: every other browser case
covered the *consumer* side, and S7 — the provider publishing — was
proven only by `roymctl` and the e2e. A journey step the slice is named
after should not be CLI-only.

Case 20 is not decoration. The default publication limit is 20 per 24
hours, and the *likely* case — not the edge case — is a provider with a
large catalogue joining a group and being refused partway through their
first publish. The default is not the thing to change (`D-C6-22`); a
default nobody can find is.

### 11.5 Failure-and-security-matrix rows C6 closes

| Row | How |
|---|---|
| **1** (forged or absent listing signature) | **Fully, for the directory path**: parity 81 (the directory refuses it at publish), parity 102 (a forged hit reaching a consumer is kept and marked, never shown as trusted), browser 15. C5 closed the local half. |
| **2** (a directory asserts a credential is valid) | **Structurally**: parity 93 asserts a directory's answer carries no verdict field at all, so there is no assertion to consult. The credential *content* half is C9's; C6 renders `credential: "unknown"` (parity 102, browser 14). |
| **3** (no Directory deployed anywhere) | **The R1 half, twice**: e2e step 4 (before any directory exists) and e2e step 12 (after one has). Parity 101 at verb level. R2's half stays C8's. |
| **12** (flooding) | The **publication half completes**: the directory-side caller ships (parity 83), the withdrawal exemption holds (parity 84), and the refusal is visible to the sender with a `retry_after_secs` (e2e step 10). `[PRD-SAF]` is now fixed at both call sites against one function. |
| **15** (a suspended member's cached listing) | **The removal half only**: e2e step 11 — the member leaves that directory's results and the cached copy is untouched and not claimed gone. The *signed revocation showing on next check* half needs C9's `revocation` record and is explicitly **not** closed here. |
| **18** (an unaffiliated caller resolves the Directory) | e2e step 9: an anonymous wire caller reaches `directory.search`, and `member.list` on the same connection is cleanly refused — *"a service that declares neither stays cleanly refused"* is the other half, covered by the five services that are not `open`. |
| **19** (build divergence) | 51 new parity scenarios, of which 79/80b/94/97b/97c/102b/102c/109 are the ones that matter: the new wire arms behave identically on both builds, in both directions. |

Rows C6 explicitly does **not** close: **2's credential half** and
**15's revocation half**, both C9's by `D-06C-6`'s R1/R3 split.

---

## §12 Order of work

Six work orders. Each compiles and its own tests pass before the next.
**WO5 is severable**: cutting it for time unwinds nothing else.

**WO1 — the vocabulary and the admission rule.**
1. `roym_core::admit`'s `WireRule` / `Caller` / `admit`, with its unit
   tests (§3). `require_internal` untouched.
2. `roym_core::area`'s three intersection functions (§4.2).
3. `roym_core::listing::verify_envelope` + `ListingVerdict`, and
   `catalog.listing.verify` rewritten as its wrapper with
   `revocation_status` added (§4.3). N-2's caps land here.
4. `roym_core::directory` (§4.1), `router.rs`'s `member.` prefix (§4.4),
   `backup.rs`'s two section names (§4.5).

**WO2 — the server half.**
5. `roym_directory`'s collections, the projection builder, and the verb
   table (§5.1–5.3).
6. `directory.publish` with the limiter and the prune (§5.4).
7. `directory.search` (server half), the refinement, the ordering (§5.5).
8. `directory.export` / `import` / `reindex`.
9. The `admit::admit` call and the `WIRE_REACHABLE` table at the top of
   `invoke`.

**WO3 — the client half and the wiring.**
10. `sources`, `runs`, `search_runs`, and the four client verbs — `start-run`,
    `query-source` (one source, one call), `merge`, `run-envelope` (§6).
    **The epoch and admission assertions (parity 97c, 97d) land with
    `query-source` and `start-run`, not after them**: they are the
    constraints that shaped both verbs. `query-source`'s two validity
    checks (parity 97e) land with the verb, not as a follow-up.
11. `directory.publish-to-source`, the manifest's `directory → catalog`
    edge, and `init_roym`'s matching binding (§7, §9).
12. The C5-7 read-verb fixes (§8.1).
13. `roymctl roym directory` (§10).

**WO4 — the two Roym suites.**
14. The four parity harness changes, then the §11.2 scenarios.
15. `roym_directory_e2e.rs` (§11.3).

**WO5 — the shim's own divergence (`D-C6-17`, severable).**
16. Wire the native `web` and fixture HTTP and websocket sinks, and the
    `NativeHttpAdapter`, through a `host_for_wire`-backed host
    (`crates/substrate/src/runtime.rs`, `crates/app_host_native/src/http.rs`,
    `.../factory.rs`), then add the fixture scenario to
    `crates/app_host_native/tests/dual_build_parity.rs` (§11.1). Nothing
    in WO1–WO4 depends on this, and nothing in WO6 does either except
    the one backlog row it closes.

**WO6 — the Hub, the gate, the documents.**
17. The Hub's Directory and SynOrg tabs, the Listings tab's
    publish-to-a-directory control (**S7**, browser case 21), its vitest
    additions, and `roym-hub.spec.ts` cases 13–23 (§11.4) — case 20's
    publication-limit control and case 21's publish control are both
    required, not optional (`D-C6-22`, §17). The client's parallel loop
    lives here, in `rpc.ts` — **bounded by the `max_concurrency`
    `start-run` returns, never by a number the client picked**
    (`D-C6-26`) — with case 22 proving it renders progressively and case
    23 proving it holds up at `MAX_SOURCES`. Rebuild with
    `mise run build:roym-ui` and `mise run build:roym`; run
    `mise run test:roym-ui`.
18. `cargo xtask check-roym-deps`, then the full gate:
    `cargo +nightly fmt --all`,
    `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace`, `cargo audit`,
    `cargo deny check licenses`, `mise run test:e2e`.
19. Documents and backlog (§15).

**WO1 step 1 is the choke point and the riskiest.** It is the first change
to the rule `D-C5-3` made absolute, and getting it wrong opens something.
Land it with its unit tests and parity scenario 107's wire half **before**
any directory verb exists, so that the slice's first proof is that nothing
new is reachable — then add reachability one named method at a time.

WO2 and WO3 both touch `roym_directory`; they are ordered, not parallel.
WO5 touches neither and may run any time after WO1, or not at all.

---

## §13 What is compared across builds, and what is not

`D-C4-12`'s rule, extended to this slice's artifacts.

- **Compared byte for byte:** the signed `listing` envelope, at every hop
   — as the provider signed it, as the directory stored it, and as the
  consumer received it. That the three are the same string is the whole
  premise of `D-C6-3`, and the parity suite asserts string equality, not
  structural equality.
- **Compared after `strip_volatile`:** every row `directory` writes. The
  new volatile fields are `received_at_secs`, `added_at_secs` (already in
  the list from C4), `last_ok_secs`, and `answered_at_secs` — each the
  guest's own wall clock, which is unsynchronized between builds
  (permitted difference 7).
- **Compared directly, because they are content-derived or pinned:**
  `listing_id`, `record_id`, `issued_at_secs` (the pinned `RecordClock`),
  the `search_index` key `<listing_id>#<area_index>`, and the
  `publication_log` key.
- **Compared as an ordered list, not as a set:** `hits[]` and
  `refused[]`, on both builds, because `D-C6-11`'s and `D-C6-18`'s
  orderings are both deterministic and a set comparison would hide the
  one thing most likely to break. The round-robin visits sources in DID
  order, so the merged order is reproducible without any stored counter.
- **Compared, and easy to forget:** the `search_runs` rows a
  `query-source` call writes. They are the merge's whole input, so a
  difference there shows up only as a wrong merged page, several
  scenarios later, looking like a merge bug.
- **Not compared:** `age_secs` (a difference of the guest's wall clock and
  the pinned signed clock — normalized to a bucket before comparison, or
  asserted only for its presence and its sign), the wall-clock time each
  fan-out took, and the order in which sources were contacted when the
  harness varies it.

**The parity harness's `RecordClock::Fixed(F)` must sit just ahead of
wall-now**, as C5's did. `age_secs` is `now − issued_at_secs`, and a
pinned clock far in the past makes every result look stale in a way that
hides a real bug; far in the future makes it negative and the assertion
meaningless.

---

## §14 Permitted differences (WASM vs native)

Carrying forward `status.md` §14's twelve, C6 adds:

13. **The guest dispatch epoch exists on WASM and not natively, and the
    design removes the case where that would show** (`F6b`, `D-C6-9`).
    WASM arms `store.epoch_deadline_trap()` with a 5 s wall-clock budget;
    the native shim arms nothing. With the fan-out loop outside the guest,
    every dispatch on both builds makes at most one bounded call, so the
    budget is never approached and the two builds cannot disagree.
    **Not permitted:** any node-side loop over sources, on either build —
    it would pass natively and trap on WASM, which is the divergence
    failure-matrix row 19 names, and which no parity harness would catch,
    because in-process test sources never take seconds. Parity 97b is the
    guard: one dispatch, one proxy call.
14. **Guest-HTTP admission exists on WASM and not natively, which is the
    limit `MAX_CLIENT_CONCURRENCY` is derived from** (`D-C6-26`, `F6d`).
    WASM acquires a per-service permit sized
    `max_concurrent_guest_http_per_service` and answers 503 after a
    two-second wait; the native shim dispatches straight to Tokio tasks
    with no admission limiting or queueing at all
    (`deferred-backlog.md` line 73). So on the native build there is no
    permit, no 503, and **`SourceError::NotStarted` can never occur**.
    **Not a divergence in behaviour** for a client that honours the
    `max_concurrency` `start-run` returns — that client stays inside the
    WASM limit and therefore never trips it on either stack, which is
    what makes the bound safe to derive from a WASM-only mechanism. It is
    recorded here because a reader of `D-C6-26` will reasonably ask what
    happens to that reasoning on the build with no permit, and because a
    *future* client that ignores the bound would fail on one stack and
    pass on the other. Same shape as item 13's epoch, opposite direction:
    there WASM is the stricter side and the design avoids the case; here
    WASM is the stricter side and the design stays inside it.
15. **The wire origin still has no production producer on the native
    build** (carried from `status.md` §14 item 10, and now load-bearing).
    A natively linked Roym directory is registered only in the local
    endpoint registry and is never published, so `host_for_wire`'s only
    caller stays the parity harness's wire driver. This is what makes
    scenarios 79/80/94 a genuine two-build comparison rather than a WASM
    test with a native stub. **A native deployment therefore cannot serve
    a foreign consumer's search**, and that must be said in `status.md`
    rather than discovered by someone deploying one.
16. **Guest-HTTP and websocket sinks now report the same origin on both
    builds** (`D-C6-17`). Previously a permitted difference by omission;
    from this slice it is a compared property, asserted by a scenario in
    the **shim's** own suite (`crates/app_host_native/tests/dual_build_parity.rs`)
    against the dual-build fixture — not in Roym's suite, because no Roym
    component reads the invocation origin in an HTTP handler. **If WO5 is
    cut**, this item does not apply and the divergence stays a permitted
    difference with its backlog row re-targeted to C7.

---

## §15 Documents and backlog owed

| Document | Edit |
|---|---|
| [status.md](status.md) | A C6 section: what shipped; §11.5's matrix rows **including the two it does not close**; §14's four additions (items 13–16); `D-C6-1` stated prominently as a correction to the milestone's own Gap 7, with `F1`–`F3`'s evidence, so no later slice re-plans around FTS5; and `D-C6-17`'s divergence fix recorded against the row it closes. **Plus the outbound mirror of §14 item 10:** a natively linked Roym carries no instance certificate (`init_roym` never calls `set_instance_cert`, backlog line 86), so it cannot *publish* to a directory any more than it can serve a foreign search — both limits belong in the same paragraph, and neither is discoverable from the code. |
| [task.md](task.md) | **Gap 7 is rewritten.** Its conclusion is wrong: FTS5 and R\*Tree are compiled in, and a service still cannot reach `execute-ddl` or `query-raw` from a verb on either build (`F1`–`F3`). The gap is restated as *"search has no index surface, and the raw-SQL escape hatch is admin-gated to the deploy-time lifecycle hook, which no Roym component exports and which the native build does not have"*. **The C6 slice row** loses *"built on FTS5 and R\*Tree through `execute-ddl`/`query-raw` (Gap 7)"* and gains *"built on the existing filter DSL over a derived projection, because the raw-SQL path is unreachable from a verb"*. **The open design point** *"How the Directory's FTS5 and R\*Tree tables coexist with `data-layer`'s own collection tables"* is answered: they do not exist; the projection is an ordinary collection, owned by `directory`, derived from `publications` and rebuildable, and `drop-collection` cannot orphan it because `drop-collection` is itself admin-gated (`F1`) and no verb can call it. The rule that binds **if** they ever land is recorded there too: created only from a lifecycle hook, named with a reserved prefix derived from the collection they index, external-content tables whose content table is the collection so that losing the collection leaves an empty index rather than a corrupt one, and never the source of record. **The open design point** *"What 'area' means on the wire"* gains C6's half: the over-covering projection is a sieve and every hit is refined exactly (`D-C6-5`). **The "Owed as slices land" table gains a C5 row and a C6 row** — it currently jumps from C4 to C7 (`task.md:657-668`), so two completed slices owe nothing according to the one table that records what a slice owes. C6's row: Gap 7 rewritten, the search-half half of R1 row 5 marked passed, the `[PRD-SAF]` publication half closed, and the backlog rows below moved. |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | **No edit to the Search section's *"in parallel"***, and this is a change from an earlier draft: the fan-out is genuinely parallel once the loop sits in the client (`D-C6-9`), so the spec is already correct and narrowing it would have made the document *less* true (§18 **B**). The **Directory** row of the service table gains `info`, `publish`, and the client-half verbs, and its Owns column gains *"and, on a consumer's own node, that person's list of directories"* — `D-C6-2` made visible rather than left as a surprise. The **Records** table gains no row; a note says the SynOrg's settings and roster are unsigned state and why (`F8`). |
| [deferred-backlog.md](../../deferred-backlog.md) | **Closed / moved to Recently resolved:** the `[PRD-SAF]` publication half (the directory-side caller ships, `D-C6-14`); the wire-side-authorization row's C6 clause (the first exception exists, `D-C6-7`); the native guest-HTTP/websocket `internal` row (`D-C6-17`) **if WO5 ships** — if it is cut, that row instead re-targets to **C7** with its trigger unchanged; N-2's unbounded listing fields (§4.3). **Restated, not closed:** the C5-7 read-verb row — the filters land, and `F4` is added to it, because a filter does not make the query use an index; retarget the *index-usability* half to the data-layer as its own row. **Re-targeted:** `conversation.search`'s unindexed scan, merged with a new blocker row (below) and moved off C6. **New rows:** (a) **`execute-ddl` / `query-raw` are unreachable from a service verb**, `F1`–`F3`, with the two candidate fixes (lifecycle exports on the Roym worlds plus a native lifecycle entry point, which fixes only the write side; or a scoped host surface for a service's own store, which is an ADR) — targeted **M6 spec**, trigger *"a directory or a conversation store grows past a scan"*; (b) **the collection indexes `create-collection` declares are never used**, `F4`, with the `EXPLAIN QUERY PLAN` evidence — targeted **M6 spec**, trigger the same; (c) **a guest dispatch cannot loop over network calls** — `F6b`'s 5 s wall-clock epoch plus `F6`'s single-poll `block_on`, which together are why the fan-out loop is the client's; the row carries `D-C6-19`'s assertion so whoever changes `DEFAULT_SOURCE_TIMEOUT_MS` or `dispatch_epoch_timeout_secs` knows they are coupled, trigger *"guest async lands, or a node-side fan-out service exists"*; (c2) **the two clients each own their own loop** — the Hub's and `roymctl`'s could diverge in ordering, in what a partial run shows, or in whether they honour `max_concurrency`, and nothing but review keeps them in step, trigger *"a third client appears"*; (c3) **the RPC dispatch leg takes no admission permit** — only guest-HTTP does (`engine.rs:2540`), so a sibling call into a busy service queues nowhere and competes directly for `max_concurrent_instances`, which fails hard rather than waiting; `D-C6-26` bounds Roym's own contribution and bounds nothing else's, trigger *"a second app shares the node"*; (d) **`versions_differ` is surfaced and never resolved** — two directories disagreeing about the current version is shown to the person and nothing reconciles it, trigger *"a person is asked to act on the disagreement"*; (e) **the directory keeps only the current version of a listing**, not its history, so a consumer cannot see what changed; (f) **no search caching** — every search is a live fan-out, so a person who is offline sees nothing rather than a stale-but-labelled answer, trigger *"offline search is asked for"*; (g) **`search_index` is rebuilt only on demand** — a partial write leaves it inconsistent until `directory.reindex`, which nothing runs automatically; (h) **only `category`, `area`, free text, `open_to` and `booking_mode` are filterable** — price, product, service, relationship and service-record terms are in the signed listing and not in the projection, so `task.md`'s *"and filters"* is narrower than it sounds, trigger *"a person asks to filter on price"*; (i) **a merged page cannot be paged past `MAX_SEARCH_RESULTS`** — `merge` returns one page and there is no cursor across sources, trigger *"a directory holds more than a page of relevant listings"*; (j) **`search_runs` rows are working state pruned by age, not by completion** — a run abandoned by a client lingers for `RUN_RETENTION_SECS`.

**Three rows this slice moves that an earlier draft did not record** — AGENTS.md's mandatory-update rule applies to rows a change *resolves*, not only to rows it creates: **line 225** (`publications` never pruned, and the limiter scans all of them) is **resolved** by §8.1, which applies both the `at_secs` filter and the prune at `publication_secs_in_window`; **line 352** (`topology_visibility = "open"` / `supervisor/resolve` has no dedicated Roym e2e test) is **resolved** by e2e step 9, which is exactly that test over a real transport; **line 86** (natively linked Roym services carry no instance certificate) is **restated as load-bearing** rather than resolved — `F6c` makes it the reason a native build cannot publish, and it needs the outbound-limit sentence in `status.md` beside it.

**One defect this slice found in C5 and does not inherit:** `conversation.search`'s `escape_regex` (`crates/roym_conversation/src/app.rs:789`) escapes regex metacharacters and **not** `%` or `_`, while the host compiles `$regex` to `LIKE '%…%'` with no `ESCAPE` clause (`filter.rs:298`) — so a person searching their own messages for `50%` matches everything, and `_` matches any character. Its own row, targeted **TBD**, with `D-C6-24`'s normalization named as the fix to copy. |
| [developer-guide.md](../../../developer-guide.md) | `roym directory` documented beside `roym address` and `roym enrol-signing`, including `serve` (creating a SynOrg) and `search`'s output columns. |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | The architecture paragraph's sentence about the non-`web` services answering `<name>.ping` only is now wrong for `directory` too; and the `directory` service's one-line description gains its two halves. |

**No new ADR.** C6 adds no wire format, no record type, and no host
interface; it uses the proxy, the data layer and the invocation origin
exactly as they are. The one thing that *would* deserve an ADR — a scoped
way for a service to run raw SQL against its own store — is backlog row
(a), and it is deliberately not written here, because writing it inside a
product slice is how a security boundary gets moved by a slice plan.

---

## §16 What "done" means for C6

1. A SynOrg owner creates a SynOrg with a name, rules, area, categories,
   support contact, dispute path and retention policy, and a stranger on
   another installation can read those rules over the wire before
   deciding to trust the group.
2. A provider publishes their own signed listing to a directory they
   chose, from their own node, and the directory stores the **exact
   bytes** the provider signed.
3. A directory never pulls: no verb takes a provider's address, and a grep
   for a fetch, a crawl or a refresh timer in `roym_directory` finds
   nothing.
4. A consumer adds one or more directories by DID, searches by category,
   area and filters, and gets results that carry **their source** and
   **their age on the consumer's own clock**.
5. **The consumer's own node verifies every result**, the directory's
   answer carries no verdict at all, and revocation and membership both
   render as the word "unknown" rather than as an absent field — R1 row
   5's acceptance test, both halves.
6. A forged listing served by a directory is shown as refused evidence
   with its reason, never dropped silently and never rendered as
   unknown-but-probably-fine — **and a directory serving nothing but
   forgeries pushes no honest result off the page**, because the two
   lists are capped independently (`D-C6-18`, parity 102b).
7. A geometric search returns what actually intersects, not what the
   over-covering box caught, and a listing stating only a named area says
   so rather than silently missing.
8. Two directories holding two versions of one listing produce **one**
   merged result carrying both sources and a visible disagreement.
9. A slow or dead directory produces an error beside its own name and
   removes nothing another directory returned — and results from the
   directories that did answer render while a slow one is still
   outstanding, because the loop is the client's and genuinely parallel
   (`D-C6-9`, browser case 22).
10. **No single directory can fill a page**, with forged listings or with
    validly signed recent ones (`D-C6-18`, parity 102b and 102c) — and no
    guest dispatch ever waits on more than one source, asserted against the
    dispatch epoch at build time rather than discovered as an intermittent
    trap (`D-C6-19`, parity 97b/97c).
11. **The whole R1 find-and-engage path completes with no directory
    anywhere** — proven twice in the e2e, once before any directory
    exists and once after one has been added and removed. `D-06C-6a`.
12. `directory.search`, `directory.info` and `directory.publish` are
    reachable from off the node — and every client-half verb is **not**,
    because they are different methods (`D-C6-20`);
    `directory.publish` refuses an
    anonymous caller — **including a caller whose own node holds no valid
    instance certificate, which is the same refusal for a different
    reason** (`F6c`, parity 80b, e2e 7b); **every other method on every
    Roym service still answers `-32013` over the wire on both builds**,
    proven by parity 107 in both directions; and **no wire-reachable
    method calls a sibling** (§7.2, parity 109).
13. `safety::admit_publication` has its second caller, keyed on the
    record's issuer, and its ledger prunes itself — and a SynOrg owner
    can raise the limit from the Hub, in one flow, without a CLI
    (`D-C6-22`, browser case 20).
14. `CallTarget::Service` appears in no Roym crate but `roym_directory`,
    asserted by a test rather than by review (§7.1, parity 108).
15. A republished listing leaves no stale index row, a `draft` is refused
    rather than silently charged, and `retention_secs` is a policy the
    directory keeps rather than only displays (§5.4, `D-C6-23`,
    `D-C6-25`, parity 84b/84c/84d).
16. A merged page carries projections, not envelopes, and its worst case
    is far inside the 1 MiB guest response cap (§6.4).
17. **The client's loop is bounded by a number the node supplies**, and a
    run over the full `MAX_SOURCES` completes with no 503 and no instance
    exhaustion — proven at eight sources, not at two (`D-C6-26`, parity
    97d, browser cases 23 and 23b). A source this node refused to start
    says `NotStarted`, not `TimedOut` — asserted in the browser suite,
    because that arm exists only on a real HTTP path (§11.2's note).
18. All 51 new parity scenarios pass identically on both builds, and the
    two-substrate suites grow to a three-substrate one that passes all
    sixteen steps.
19. `cargo xtask check-roym-deps` is clean, and a grep over
    `crates/roym_*`, `crates/roym_core/app/` and every file this slice
    touched finds **no planning identifier in any name *or* comment** —
    `M0[0-9]`, `\bR[1-4]\b`, `\bC[0-9]`, `D-C[0-9]`, `D-0[0-9]`,
    `Slice `. ADR references are the only permitted exception.
20. The full gate in §12 step 18 is clean.
21. §15's documents and backlog rows are written — including the two
    matrix rows C6 does **not** close, the row it restates rather than
    resolves, and the `task.md` Gap 7 correction, which is the single
    most important document edit in this slice.

---

## §17 What C6 deliberately does not build

- **Membership credentials, revocations, and moderation decisions.** C9,
  by `D-06C-6`'s R1/R3 split. C6 renders both as `unknown`, which is the
  honest value and is the same string C9 will replace with a real verdict
  without changing the shape.
- **Any FTS5 or R\*Tree table.** `D-C6-1`, `F1`–`F3`. Backlog row (a)
  carries it with a real trigger.
- **Any use of `execute-ddl` or `query-raw`, anywhere.** A grep proves it.
- **A signed SynOrg settings record or a signed member list.** `F8`,
  `D-C6-12`. `directory` mounts no signing certificate and the enrolment
  ceremony stays three services.
- **A node-side fan-out loop.** `F6b`: a guest dispatch has a five-second
  wall-clock budget that is spent while waiting on host calls, so the loop
  is the client's (`D-C6-9`). The node does one source per dispatch. This
  is not a limitation the product feels — the loop is genuinely parallel
  in a browser — but it is a shape a reader would otherwise expect to find
  and not find.
- **Filtering on price, product, service, relationship or service-record
  terms.** `SearchQuery` filters on category, area, free text, `open_to`
  and `booking_mode`. The other listing dimensions are in the signed
  envelope and not in the projection, so they are visible on a result and
  not selectable in a query. `task.md`'s *"and filters"* is narrower than
  it sounds and is narrowed here in writing, with a backlog row.
- **An application path for joining a SynOrg.** Journey steps **S4** and
  **S5** — a provider applies, the owner reviews — have no surface. The
  roster is DIDs the owner types in (`member.add`), which covers **S6**'s
  approval half only. An application is a message, and the membership
  credential that would answer it is C9's.
- **SynOrg announcements.** Journey steps **S8** and **S10** — the group
  sharing announcements, guides and local information with its members —
  are not built. They need a broadcast the roster can receive, which is
  neither search nor publication.
- **Paging a merged result past one page.** `merge` returns one page.
  There is no cursor across sources, because a cursor would have to
  encode eight sources' positions and each source's own answer is already
  capped.
- **Anything the FTS5 path would have bought.** `D-C6-1`, `F1`–`F3`.
- **Ranking, relevance scoring, paid placement, and free-text intent
  parsing** — each excluded by R1 row 5's own Excluded column.
- **Search result caching.** Every search is live. A person offline sees
  no results rather than stale ones — backlog row (f).
- **Listing history inside a directory.** A directory holds the current
  offer; versions live on the provider's own catalog — backlog row (e).
- **Automatic discovery between SynOrgs**, a well-known list, or a
  directory-of-directories — the spec's own *Not in the first release*
  list, and `D-C6-16`.
- **Shard placement, signed Publications, rendezvous hashing.**
  `D-06C-6b`. The word "publication" here names a stored envelope, not
  M8's signed Publication, and no code uses M8's vocabulary.
- **The signed suspension that removes a member from results.** C9. C6
  builds `directory.unpublish`, which is the SynOrg owner acting under
  their own rules (journey step S9), not a signed moderation decision.
- **Cards, requests, quotes, agreements.** C7 — which depends on C6 for
  the search that starts the flow, and on C5 for the conversation the
  cards appear in.

---

## §18 Ambiguities and staleness in the input documents

Flagged rather than guessed. **A** changes what gets built.

**A. Gap 7's conclusion is false, and the C6 row inherits it.** *"The
Directory can create and query its own FTS5 and R\*Tree tables through
DDL, inside the same DEK-encrypted database, with no new host interface"*
— every clause is true except the one that matters: **who may run the
DDL**. `execute-ddl` and `query-raw` both require `data-layer/admin`
(`F1`), whose only producer is the deploy-time lifecycle hook (`F2`),
which no Roym component exports and which the native build does not have
at all, and which no owner-rooted credential can substitute for (`F3`).
Gap 7 verified the SQLite build flags — correctly, and this plan
re-verified them today — and did not check the capability gate. The
mechanism is unavailable; the *capability* R1 row 5 asks for is still
deliverable without it (`F5`), which is why `D-C6-1` narrows the mechanism
rather than the scope. A reviewer who wants FTS5 in this slice must accept
one of two consequences and say which: the slice grows a host-interface
change and an ADR about a security boundary, or R1 row 5 does not ship in
C6.

**B. The spec's "in parallel" is implementable after all, and an earlier
draft of this plan was wrong to propose narrowing it.** `F6` is correct
that a WASM guest cannot await two host calls — `block_on` panics on
`Poll::Pending`. The error was concluding that the fan-out must therefore
be sequential. It only follows that the *loop* cannot be inside a guest,
and `F6b` shows the loop could not have lived there anyway: a five-second
wall-clock dispatch budget would have trapped it. With the loop in the
client (`D-C6-9`), a browser issues the per-source calls concurrently and
the spec's word is literally true. **The spec edit is withdrawn**, and
§15 no longer asks for it. Recorded here rather than quietly deleted,
because the earlier draft had already written the narrowing into `§15`
and a reader of both versions deserves to know which way it went.

**C. `task.md`'s C6 row says "its member list" without saying whose.** A
SynOrg's roster and a consumer's list of directories are different lists
with different owners, and the row's single phrase covers only the first.
Both are built; §5 and §6 keep them in separate collections and `D-C6-13`
gives them different reachability, because conflating them would make a
roster wire-readable by accident.

**D. The spec's service table says the Directory "Runs on: SynOrg's
substrate".** C2 deployed it on every installation and no test changed.
The column describes where the server half *matters*, not where the
component lives; `D-C6-2` makes that explicit and §15 records the spec
edit rather than leaving the table and the manifest disagreeing.

**E. The spec's Directory API column is `search`, `member.*`,
`credential.*`, `revocation.*`.** Bare `search` cannot be routed — the
entrypoint's table is prefix-based with no default arm (`F13`) — so the
method is `directory.search` for the server half, and the client half is four differently named verbs (`D-C6-20`, `D-C6-9`). This is the same "the table is a summary,
not a contract" finding C5 recorded as its own **F**; §15 has the edit.

**F. `status.md` §14 item 10 says the native build has no production
producer of the wire origin.** That was a footnote in C5 and is
load-bearing in C6: it means a **natively linked Roym cannot serve a
foreign consumer's search at all**. Nothing in `task.md` says the native
build must be deployable as a SynOrg, and the dual-build requirement is
about *the same suite passing on both* (exit criterion 1), which it does.
But it is a real product limit and belongs in `status.md`, not only here.

**G. The `[PRD-SAF]` row says C6 calls the same function "so `[PRD-SAF]`
is fixed once".** It is fixed at two call sites against one function,
which is what the row meant and not quite what it says. Worth restating
in the closing note so the next reader does not go looking for a single
shared caller.

---

## §19 Review outcomes, and what is still open

This section records two rounds. The seven questions an earlier draft left
to the executor were decided on review (2026-09-04); a second review of
that revision (2026-09-04) found two blocking defects and eleven further
issues, all verified against the tree before being incorporated.

### Round one — the seven open questions

| # | Question | Outcome |
|---|---|---|
| 1 | Does the native HTTP/websocket rewiring belong in C6? | **Kept, restructured.** `D-C6-17` — its own severable work order (WO5), tested in the shim's suite against the dual-build fixture. |
| 2 | One `directory.search`, or two names? | **Changed to two**, then reshaped again by round two: the client half is now four verbs, because the loop left the guest (`D-C6-20`, `D-C6-9`). |
| 3 | Publish on `catalog` or on `directory`? | **Unchanged, reason strengthened.** §7.1. |
| 4 | Is 20 publications per 24 hours the right default? | **Default unchanged; the control is now required.** `D-C6-22`. |
| 5 | `MAX_SOURCES` and the deadlines | **Corrected, then corrected again.** The first fix repaired arithmetic that did not close; round two found the numbers were still measured against the wrong ceiling (`F6b`). |
| 6 | Do failed verifications count toward the page? | **Changed**, then completed: splitting the lists was necessary and not sufficient (`D-C6-18`). |
| 7 | `directory.info` before a SynOrg exists | **Unchanged, with a paired half.** `D-C6-21`. |

### Round two — what the second review changed

| Finding | Where it landed |
|---|---|
| **Blocking: the time budget cannot run inside a WASM dispatch** | `F6b`, `D-C6-9`, `D-C6-19`, §6.1, §14 item 13, parity 97b/97c. The fan-out loop moved out of the guest entirely. |
| **Blocking: `publish` depends on an instance certificate nobody named** | `F6c`, §5.4 step 1, parity 80b, e2e 7b, and the outbound native limit added to `status.md`'s owed edits. |
| Republishing leaked `search_index` rows | §5.4 step 8 and parity 84b. |
| The merge had no per-source share | `D-C6-11`, `D-C6-18`, `MAX_HITS_PER_SOURCE`, parity 102c. |
| The `LIKE` escaping story did not match the host | `D-C6-24`, §5.2 — and the same defect found in C5, given its own backlog row. |
| No Hub surface for S7 | Browser case 21. |
| `retention_secs` enforced by nothing | `D-C6-23`, §5.4 step 7, parity 84d. |
| Filters narrowed silently | §17 and a backlog row. |
| S4/S5 and S8/S10 unmentioned | §17. |
| Backup sections did not add up, and `publication_log` was dropped | §4.5 — five sections, bare nouns, with backlog line 227 cited as the precedent to avoid. |
| Three backlog rows this slice moves and did not record | §15 — lines 225, 352 and 86. |
| The confused-deputy shape became reachable and was left implicit | §7.2 and parity 109. |
| `task.md`'s "Owed as slices land" has no C5 or C6 row | §15. |
| Response size unbudgeted; `unpublish` key unstated; a `draft` publishable but unsearchable; "five current callers" | §6.4; §5.3; `D-C6-25`; §3. |

### Round three — the redesign's own new bound

Moving the loop to the client removed a node-side ceiling and walked into
two others nobody had looked up (`F6d`): guest-HTTP admission is four
concurrent per service with a two-second wait before a 503 — the same two
seconds as `DEFAULT_SOURCE_TIMEOUT_MS` — and the RPC leg takes no permit
at all, so each in-flight source holds two instances against a pool of ten
that fails hard. `D-C6-26` bounds the client loop at three, derived from
the admission limit and asserted against it, returns that number from
`start-run` so no client carries its own copy, and restores
`SourceError::NotStarted` for the node's own refusal. Three smaller items
landed with it: `query-source` now validates `source` and `run_id`
(§6.1), the projection is stored at verify time so `merge` parses no
envelopes (§6.1, §6.4), and the coverage moved from two sources to
`MAX_SOURCES` (parity 97d–97f, browser 23, e2e 13b).

**The lesson worth carrying into C7**, since it recurred four times: a
bound was chosen against an assumed ceiling rather than a looked-up one,
or one constant was made to do two jobs. Round one's `MAX_SOURCES`
arithmetic did not close. Round two found the epoch that arithmetic had
never been measured against, and the crowding hole where one constant
served as both the per-source and the merged cap. Round three found the
admission limit sitting under the fix for the epoch — a new bound
introduced by the repair of an old one.

Where this plan now names a bound, it also names what the bound is
measured against and where a test asserts the relationship:
`DEFAULT_SOURCE_TIMEOUT_MS` against `dispatch_epoch_timeout_secs` (parity
97c), `MAX_CLIENT_CONCURRENCY` against
`max_concurrent_guest_http_per_service` (parity 97d), and
`MAX_HITS_PER_SOURCE` named apart from `MAX_SEARCH_RESULTS` so neither can
quietly become the other.

### Still open, and genuinely the executor's

1. **Whether `MAX_HITS_PER_SOURCE = 10` and `MAX_SEARCH_RESULTS = 50` are
   the right pair.** Eight sources at ten each is eighty candidates for
   fifty slots, so the round-robin is doing real work and a person with
   one good directory and seven poor ones sees at most ten from the good
   one. That is the cost of crowding resistance and it may be the wrong
   trade for R1's actual deployment shape — one SynOrg, a handful of
   members. Raising `MAX_HITS_PER_SOURCE` weakens the guarantee smoothly
   rather than breaking it, so it is a safe dial.
2. **Whether `MAX_REFUSED_RESULTS = 20` is too low to be honest.** A
   directory serving 200 forgeries shows 20 and a truncation flag. A
   *count* of what was refused, alongside the capped list, is the cheap
   middle answer and is not in the plan.
3. **Whether `directory.unpublish` should tell the provider.** Silent in
   this plan: the provider learns by searching. Buildable — the listing
   carries their conversation address — and deliberately not built,
   because a moderation *decision* is a signed record and that is C9's.
   Worth confirming that silence is acceptable for R1 rather than assumed.
4. **Whether the client's parallel loop belongs in `rpc.ts` or in a small
   shared module `roymctl` can mirror.** The two clients must agree about
   ordering, about what a partial run shows, and about honouring
   `max_concurrency`; nothing today makes them. A node-side `merge`
   protects the *result*, not the *loop*. This is the strongest remaining
   argument for a fifth client verb that runs the loop node-side — which
   `F6b` forbids, so the answer is a shared module or continued
   vigilance, not a verb.
5. **Whether `MAX_CLIENT_CONCURRENCY` should be read from config rather
   than asserted against it.** The plan hardcodes 3 and asserts it stays
   below `max_concurrent_guest_http_per_service`'s default. An operator
   who raises that config value gets no benefit until someone raises this
   constant too. Reading it at runtime would need the guest to see
   substrate config, which no interface offers — so the assertion is the
   honest version, and a comment on the config field pointing back is the
   cheap improvement.

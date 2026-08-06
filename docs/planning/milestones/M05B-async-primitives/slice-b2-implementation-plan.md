# Slice B2 Implementation Plan — The Guest-Facing Outbox, Receiver-Side Dedup, and the Proxy DLQ

**Status:** 📋 Planned (2026-08-05). Not started. Milestone:
[task.md](task.md) slice **B2**; milestone-level plan:
[implementation-plan.md](implementation-plan.md). Design of record:
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) §1, §4 and §5,
with [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§2 for the dependency-resolution rule the queued call path must not break.
Depends on **B1 — Complete 2026-08-05** ([status.md](status.md)). Gates
nothing; **M6's chat SynApp is its consumer**.

**The one-sentence summary.** B2 gives a *guest* the thing B1 gave the
supervisor — work that survives an unreachable target and a process restart —
except that a guest has none of the fences the supervisor inherited, so B2 must
also build the fence: an idempotency key on the wire, a receiver that remembers
it, and a refusal for any queued call that does not carry one.

**Read first:** the milestone plan's §0 and §1 (cross-cutting findings and
decisions D-B-1 … D-B-9), and B1's plan §0.1, §0.11 and §0.14 — B2 inherits the
*shape* of three of B1's arguments (idempotence versus queueability, payload
staleness, and "a late delivery is often success") and reaches a different
answer for two of them.

**A warning about B1's plan document.** B1 shipped materially different from
[its own plan](slice-b1-implementation-plan.md) after three rounds of
post-landing review — a two-variant `PushOutcome`, an error-classification
helper (`deploy::is_target_gone_error`), group-scoped DLQ pruning, a claim-side
counter, a third `supervisor.wit` verb (`outbox`), a fail-closed dedup guard —
and none of that is in the plan's prose. **Every claim in this document about
"what B1 built" was checked against the shipped source**, not against that plan.
Where the two disagree, this document follows the source and says so.

**This plan's own §0 changes B2's scope in four places** before any code:
`call-options` cannot carry a key to the receiver at all (§0.2); the DLQ this
slice owes cannot admit every failed call, only keyed ones (§0.1); the store the
milestone assumed (a substrate-wide proxy DLQ) would put a guest's own call
parameters in an unencrypted database (§0.3); and "the first result is returned"
has no defined meaning for a call still running, which is the exact case the
retry that asks the question is most likely to hit (§0.5).

**Review pass (2026-08-05), all findings incorporated.** Thirteen findings plus
six smaller ones; the first draft did not work in five places, and three of them
are the same class of mistake — naming a mechanism without naming what it runs
on:

- **The dedup record had no defined identity for the callers this slice is
  for** (§0.16). ADR-0023 §4 says `(caller DID, key)`; a guest-originated call
  arrives with a verified DID, with `"system:<service_id>"`, or **anonymous**,
  depending on path and certificate state. An anonymous namespace is not a
  namespace: two callers sharing it would read each other's results.
- **Every keyed call to a node-level native interface would have been refused,
  and would have created a database for a service that does not exist**
  (§0.17). `orchestrator`, `security`, and the supervisor's dispatch id are not
  deployed services, but `load_service_dek` generates a DEK on first use and
  `SERVICE_ID_REGEX` accepts their ids, so the refusal would have arrived only
  after silently writing a file.
- **The HTTP bridge is not excluded — it routes through the guarded entry
  point** (§0.6, rewritten). `handle_json_rpc_bridge` and `dispatch_native`
  both forward into `dispatch_json_rpc_once`, so the first draft's "deliberately
  excluded" was a claim about ingress design stated as a structural fact.
- **`ProxyRouter` has none of the handles this slice needs** (§0.18). "No new
  crate, no new handle" was true of `HostState` and false of the router: no
  `StorageProvider`, no `KeyStore`, no resolver, ~10 constructor call sites, two
  new crate dependencies, and a third distinct "no store" case (coordinator
  mode) that D-B2-9's fail-closed rule had not accounted for.
- **Nobody owned the worker** (§0.19). Its behavior was fully specified and its
  construction site was not named at all.

Plus: the stored payload could not actually be re-resolved or authorized at
delivery (§0.20); an encryption-disabled deployment had no rule and made test 39
false by construction (§0.4); the dedup table had a time bound but no row bound
(§0.21); the guard was a synchronous SQLite call left on an async hot path, with
the SQLCipher key derivation paid per call (§0.11); the worker's error table
named a variant that does not exist on `syneroym_rpc::ProxyError` and omitted two
that do (§0.10); the operator surface was named but not specified, against an
interface that already exports two verbs with the same names (§0.22); and an
existing backlog row names this slice as its own pickup trigger and needed an
answer (§4).

---

## §0 — What ADR-0023 §4, the failure matrix, and the shipped tree leave open, understate, or state wrongly

### 0.1 (Scope-changing, blocking) "Failed-after-retries lands in a DLQ" and "an unkeyed queued call is refused" cannot both be true, and the resolution is that a key — not a failure — is what admits an item to the DLQ

Three documents describe the same mechanism and pull in two directions:

- [task.md](task.md)'s B2 row and its exit criterion 5: "failed-after-retries
  proxy calls land in the DLQ", pointed at the markers in
  [router/proxy.rs:458](../../../../crates/router/src/proxy.rs#L458) and
  [rpc/proxy.rs:80](../../../../crates/rpc/src/proxy.rs#L80).
- [task.md](task.md)'s non-goals and failure-matrix row 7, plus milestone
  D-B-9: "A guest caller with no such fence supplies an idempotency key or is
  refused."
- ADR-0023 §5: a dead letter is **replayable**, and replay re-enqueues it for
  another delivery attempt.

Put together, the first says every exhausted call gets a durable row an operator
can replay; the second says an unfenced call must never be delivered twice. A
replayable dead letter for a call with no idempotency key **is** a second
delivery of an unfenced call. The two rules contradict each other at the exact
point the slice is supposed to close.

Reading it as "the DLQ is only for records, never replayed" does not save it:
ADR-0023 §5 makes replay the reason the DLQ exists ("a table nothing reads
converts silent loss into quiet loss").

**Resolution: an idempotency key is the admission ticket to the DLQ**
(D-B2-1). Three tiers, and every one of them keeps the promise the other
documents make:

| Call | On exhausting its retries |
|---|---|
| `call`, **no** idempotency key | Fails to its caller, exactly as today. No durable row, no DLQ. The caller is alive and holding the error — this is not silent loss, and there is nothing safe to replay |
| `call`, **with** an idempotency key | Fails to its caller **and** writes a dead letter. Replay is safe because the receiver dedups on that key |
| `enqueue` (always keyed, §0.9) | Never surfaces an error to the caller at all, so the durable outbox and, on exhaustion, the DLQ are the *only* places the failure can be seen |

So the two stale markers are resolved by **stating an invariant, not by queuing
everything**: a synchronous call still fails directly when it carries no key,
and the comment says why rather than naming a milestone. That is what closes
exit criterion 5 honestly. [task.md](task.md)'s B2 row, read literally, promises
more than is safe to build, and this slice narrows it deliberately rather than
quietly (§5).

### 0.2 (Correctness, blocking) `call-options` cannot deliver an idempotency key to the receiver — it never leaves the calling node

`call-options` ([proxy.wit:45](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L45))
is a guest→host record. Its fields are destructured in the host function
([host_capabilities.rs:1103](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1103))
into a `ProxyRequest`, and `ProxyRequest` never crosses a hop: it derives no
`Serialize` at all, and `invoke_remote_at` rebuilds what travels field by field.
*(The first draft cited its doc comment at
[rpc/proxy.rs:66](../../../../crates/rpc/src/proxy.rs#L66) for this. That comment
says `caller` is never wire-serialized, which is a narrower claim; the conclusion
is unchanged but the citation was wrong, in a document whose own preamble
promises otherwise — review finding.)*
What actually crosses a hop is two things and only two: the route preamble
(`<scheme>://<interface>.<service_id>?…`, carrying identity material) and a
`JsonRpcRequest` body of `jsonrpc`/`method`/`params`/`id`
([types.rs:10](../../../../crates/rpc/src/types.rs#L10)).

So "add a field to `call-options`" is about a third of the change. Receiver-side
dedup is impossible until the key has a carrier, and the migration-impact
paragraph in [task.md](task.md) — "a WIT change guests must recompile against" —
describes only the half a guest can see.

**Two candidate carriers, and the decision is the request body** (D-B2-2):

| Carrier | Against it |
|---|---|
| A preamble query parameter (`?idem=…`) | The preamble is per-stream routing/identity metadata, parsed and verified by `HandshakeVerifier` before any body is read. A same-node call has **no preamble at all** (`ProxyRouter::invoke_local` dispatches straight to the registry endpoint), so the local and remote paths would need two different mechanisms for one rule |
| **An optional member on `JsonRpcRequest`** | Strays from JSON-RPC 2.0's own member list. Ours is the only producer and the only consumer of this frame, `#[serde(default)]` keeps both directions compatible, and it puts the key with the call rather than with the route — which is what makes one dedup guard serve both entry points (§0.6) |

The field is `idempotency_key: Option<String>`, `#[serde(default)]`, skipped
when `None` so an ordinary call's frame is byte-identical to today's.

### 0.3 (Correctness, blocking) ADR-0023 §4's "the substrate's proxy DLQ" would put a guest's own call parameters in an unencrypted database, and failure-matrix row 13 forbids exactly that

ADR-0023 §4 names three queue owners: the supervisor's outbox, **the
substrate's proxy DLQ**, and a service's own outbox. Failure-matrix row 13 says
"a service's queue is in its DEK-encrypted database. A payload never sits in an
unencrypted store the surrounding data would not."

A queued proxy call's payload is `params` — the guest's own data, the same data
that lives in that service's SQLCipher-encrypted `state.db`. A substrate-wide
DLQ table would sit beside `substrate.db`, which is **not** encrypted (it holds
KEK-wrapped DEKs precisely because it is not). The two lines of ADR-0023 §4
disagree with each other for guest traffic.

It gets sharper against the shipped queue crate: `Queue` owns **both** `outbox`
and `dead_letters` on one connection
([async_queue/src/lib.rs:197](../../../../crates/async_queue/src/lib.rs#L197)).
Choosing where the outbox lives chooses where the DLQ lives. There is no design
in which the outbox is per-service and the DLQ is substrate-wide without either
splitting the crate or copying payloads between two stores.

**Resolution (D-B2-3): one `async.db` per service, in a SQLCipher file beside
that service's own `state.db`** —
`<db_dir>/services/<service_id>/async.db`, keyed with the same DEK, created the
same way `state.db` is
([sqlite.rs:1663](../../../../crates/data_db/src/sqlite.rs#L1663) is the pragma;
[sqlite.rs:1416](../../../../crates/data_db/src/sqlite.rs#L1416) resolves the
directory). Two small pieces of new surface, both narrow:

- `Queue::open_encrypted(dir, db_name, dek, config)` on the queue crate — the
  same body as `Queue::open` with one `pragma_update(None, "key", …)` before
  the schema runs. `rusqlite` is already `bundled-sqlcipher` workspace-wide
  (`Cargo.toml:109`), so no dependency changes.
- `StorageProvider::service_db_dir(&self, service_id) -> Result<PathBuf>` — the
  existing private `resolve_service_db_dir`, promoted. It already refuses an
  invalid id and a traversal attempt, which is why it is promoted rather than
  re-derived at the call site.

**Not inside `state.db` itself.** That database is the guest's, reachable by its
own `query-raw` (read-only, but readable). A host-owned queue there would be
guest-readable for no benefit, and `SqliteServiceStore` exposes a writer channel
and a reader pool rather than a connection the queue crate could take
([sqlite.rs:1959](../../../../crates/data_db/src/sqlite.rs#L1959)) — so sharing
it means new plumbing, where a sibling file means one constructor.

**One file, two roles, three tables — and the roles use different services'
files** (review finding; the first draft said "one queue per calling service"
and then scoped the dedup test to the *target*, which cannot both be true).
`async.db` is opened by whichever side needs it, in that side's own directory:

| Table | Whose `async.db` | Opened by |
|---|---|---|
| `outbox`, `dead_letters` (the queue crate's own) | The **calling** service's | The sender: `enqueue`, the worker, the DLQ verbs |
| `call_dedup` (new, §0.5) | The **target** service's, with the target's DEK | The receiver's guard, on the node hosting the target |

A service is a sender for its own calls and a receiver for calls made to it, so
on a node hosting both ends of a call two different files are opened. The tables
are disjoint and neither role ever reads the other's.

**Consequence for ADR-0023 §4:** its middle bullet is corrected at closeout from
"the substrate's proxy DLQ" to "each calling service's own outbox and DLQ". The
substrate keeps no proxy queue of its own, because nothing that is not a guest
can reach the durable path at all (§0.9).

### 0.4 (Correctness) Neither the outbox nor the dedup store can be opened until an operator injects the KEK, and the honest answer is to wait, not to proceed

`load_service_dek` runs `verify_encryption_mode` first
([traits.rs:30](../../../../crates/data_db/src/traits.rs#L30);
[sqlite.rs](../../../../crates/data_db/src/sqlite.rs)) and fails while the
keystore is locked. The vault is locked after **every** substrate restart until
`security/inject-kek` runs — the same constraint that made B1 defer durable
delivery for certificate-bearing actions
([B1 plan](slice-b1-implementation-plan.md) §0.11).

Two different situations, and they need opposite answers:

- **The outbox worker** finds a service whose DEK will not resolve: skip that
  service this tick, log once, try again next tick. Nothing is lost — the items
  are on disk, and a call that waits for an operator is the whole point of a
  durable queue.
- **The receiver's dedup guard** cannot open the store for a call that *carries*
  a key: **refuse the call** (D-B2-9). Executing without a dedup check is
  executing an at-least-once delivery with no fence, which is the one thing this
  slice exists to prevent. This is the same fail-closed correction B1 had to
  make to `SupervisorOutbox::already_pending`
  ([outbox.rs:163](../../../../crates/app_supervisor/src/outbox.rs#L163)) after
  its own review found the fail-open version writing the duplicate its guard
  existed to stop.

An unkeyed call is untouched by any of this: it never opens the store (§0.11).

**And "no DEK" is four different situations, not one** (review finding — the
first draft folded them together, and one of them made test 39 false by
construction):

| Situation | `load_service_dek` | Rule |
|---|---|---|
| Encryption enabled, vault locked | `Err` | Fail closed for the receiver, skip-and-retry for the worker, as above |
| Encryption enabled, vault open | `Ok(Some(dek))` | Open `async.db` with `PRAGMA key`, the ordinary case |
| **Encryption disabled for the whole deployment** | `Ok(None)` — "a deliberate per-deployment mode, not an error" ([traits.rs:26](../../../../crates/data_db/src/traits.rs#L26)) | Open `async.db` **unencrypted**, exactly as `state.db` is in that mode. Neither refuse nor pretend: the queue's protection matches the surrounding data's, which is all failure-matrix row 13 asks |
| Coordinator mode — no storage provider at all | n/a, the handle is `None` | The durable path does not exist on such a node, and neither do deployed services or guests. `enqueue` answers `internal`, the same shape the sandbox-absent arms already use (§0.18) |

Test 39 (`the_queue_file_is_unreadable_without_the_services_dek`) therefore
asserts the encryption-**enabled** deployment and is skipped otherwise, rather
than claiming a property the disabled mode does not have.

### 0.5 (Correctness) Failure-matrix row 8 does not say what happens while the first call is still running, and that is the case a retry is most likely to hit

Row 8: "A guest replays a call with a used idempotency key inside the TTL → The
first result is returned; the target is not re-executed."

That is well defined only after the first call finished. The common duplicate is
not a leisurely replay — it is the sender's outbox retrying because its
*attempt* timed out while the receiver was still working. Row 8 as written has
no answer for it, and the natural implementations are both wrong: re-executing
breaks the promise, and returning "no result yet" as a success is a lie.

**The record therefore has three states, not two** (D-B2-6):

| State | A duplicate arriving now gets |
|---|---|
| **In flight**, claim not expired | A **callee error with a reserved code** meaning "a call with this key is already running here". Not re-executed. The sender's queue treats this one reserved code as retryable and comes back later for the stored result (§0.10) |
| **Done** | The stored first outcome, returned verbatim — a success value, or the callee error the target itself produced. Both are real answers the target gave |
| **In flight, claim expired** | Re-executed, and the claim is taken again |

The third row is the honest cost of at-least-once, and it is exactly B1's
visibility timeout one level up: a receiver that died mid-call leaves a claim
nothing will ever complete, and the only alternatives are blocking that key
forever or re-executing. ADR-0023 §1 already chose — "every queued action must be
safe to apply twice" — so the claim expires and the call runs again. The claim
window is the call's own budget, not the dedup TTL: `DEFAULT_PROXY_CALL_TIMEOUT`
([rpc/proxy.rs:135](../../../../crates/rpc/src/proxy.rs#L135), 30s) × 2, so a
call that is merely slow is never re-executed underneath itself.

**Not a blocking wait.** Making the duplicate wait for the in-flight call would
hold a receiver dispatch slot for up to the full call budget and would interact
with `dispatch_epoch_timeout_secs` (5s,
[config.rs:446](../../../../crates/core/src/config.rs#L446)) in a way nothing
else in the tree does. The sender already owns a retry schedule that solves this
for free.

Failure-matrix row 8 needs one added sentence at closeout; it is not wrong, it is
incomplete (§5).

### 0.6 (Coverage, blocking) The receiver has two dispatch entry points, and a guard on one of them is not a guard

A call reaches a target service two ways:

- **Same node:** `ProxyRouter::invoke_local` → native `dispatch` or
  `execute_wasm_json` ([router/src/proxy.rs](../../../../crates/router/src/proxy.rs)).
- **Another node:** the route handler's
  `dispatch_json_rpc_once`
  ([route_handler/dispatch.rs:71](../../../../crates/router/src/route_handler/dispatch.rs#L71)),
  which has its own native and WASM arms and never passes through `ProxyRouter`
  at all.

A dedup guard placed only in the proxy would be bypassed by every remote caller —
that is, by every caller that actually needed a durable queue. Both sites call
one shared guard (D-B2-7). Both are in `syneroym-router`, so this is one module
with two call sites, not a new crate.

**The HTTP bridge is not a third site, and it is not excluded either** —
corrected in review, where the first draft claimed both. `handle_json_rpc_bridge`
forwards the client's raw body into `dispatch_json_rpc_once`
([http.rs:438](../../../../crates/router/src/route_handler/http.rs#L438)), and
`dispatch_native` does the same with a body it builds itself
([http.rs:185](../../../../crates/router/src/route_handler/http.rs#L185)). The
guard reads the *body*, so HTTP-bridge traffic passes through it already. An
external client that puts `idempotency_key` in its JSON-RPC body gets the same
semantics as a guest, including the fail-closed refusal.

**That is the right behavior and it is now a decision, not an accident**
(D-B2-7): one guard, driven by what is in the request, with no per-ingress
special case. What B2 does *not* build is an HTTP-native way to express the key
(a header, a documented contract, an idempotent-POST story) or an outbox for a
client that has no service on this node — that is a separate ingress design with
no consumer, and its backlog row is rewritten to say that rather than the
structural exclusion the first draft claimed.

### 0.7 (Scope-changing) Failure-matrix row 4 says terminal failure "raises an alert", and the substrate has no alert store to raise it in

`AlertStore` belongs to the supervisor's own database
([app_orchestration/src/alerts.rs](../../../../crates/app_orchestration/src/alerts.rs)),
keyed by `(instance_id, logical_ref, substrate_did, kind)`. B1 met row 4 through
it because B1's dead letters *are* per-instance supervisor work. A proxy dead
letter belongs to a deployed service on a substrate that may not even be
managed by a supervisor, and none of that key exists.

**B2's operator surface is therefore: a listing verb, a replay verb, metrics,
and a warning log** (D-B2-13) — not an alert row. The substrate side keeps no
alert store, and inventing one here would be a second alerting model competing
with the supervisor's.

**What this leaves open, and it matters for M6:** the *guest* still cannot learn
that its own queued call died. Chat needs a user-visible "this message could not
be delivered" state. That is a guest-facing read of the service's own dead
letters — more WIT surface than the one field and one function this milestone
promised. Backlog row, pickup trigger "M6 chat needs a user-visible failed-send
state" (§4).

### 0.8 (Correctness) A queued call must store the dependency it named, not the DID that name resolved to

`proxy::Host::call` resolves `CallTarget::Dependency(name)` host-side, before
the `ProxyRequest` exists, specifically so "a guest never holds the resolved DID
and cannot snapshot it past a re-push"
([host_capabilities.rs](../../../../crates/sandbox_wasm/src/host_capabilities.rs);
ADR-0021 §2). A queued call that stored the resolved DID would snapshot exactly
what that rule forbids — and it would do so for hours, which is longer than any
guest could have managed on its own.

**So the payload stores the intent** (D-B2-8): the dependency name plus the
routing key, or a raw DID when the guest named one; resolution happens again at
delivery. This is B1's "store intent, not payload" argument
([B1 plan](slice-b1-implementation-plan.md) §0.11) reaching the *opposite*
conclusion about feasibility, and the difference is worth stating: re-minting a
certificate needs the vault open, where re-resolving a binding needs only the
local resolver. Intent storage is cheap here, which is why it is the default
here and a deferral there.

A dependency that no longer resolves at delivery is **terminal** — straight to
the DLQ, failure-matrix row 9.

### 0.9 (Sets the boundary) `enqueue` is guest-only, which is what keeps "an unkeyed queued call is refused" enforceable at one place

`CallOrigin` has two variants ([rpc/proxy.rs:46](../../../../crates/rpc/src/proxy.rs#L46)):
`Guest`, constructed only by the sandbox host function, and `Native`, for the
FDAE relationship-proof fetch, control-plane internals, and tests. None of the
`Native` callers wants fire-and-forget delivery, and none of them has a
per-service encrypted database to put a queue in.

**So the durable path admits `CallOrigin::Guest` only** (D-B2-4). Two
consequences:

- The refusal in failure-matrix row 7 has exactly one enforcement point — the
  `enqueue` host function — rather than being a property every caller must
  remember.
- There is no substrate-wide proxy queue to build at all, which is what makes
  §0.3's per-service store complete rather than partial.

**Wiring: `enqueue` is a second method on the `ServiceProxy` trait**
([rpc/proxy.rs:138](../../../../crates/rpc/src/proxy.rs#L138)), implemented by
`ProxyRouter` (its only production implementation). `HostState` already holds a
`Weak<dyn ServiceProxy>`, so the guest-facing function needs no new handle, no
new field, and no change to `HostState`'s construction. Test fakes implementing
`ServiceProxy` gain one method.

### 0.10 (Correctness) The worker's error classification cannot be "a callee error is never retried", and B1 already learned this the expensive way

The synchronous rule — a callee answer is definitive, a transport failure is
retryable — is stated in three places
([rpc/proxy.rs:78](../../../../crates/rpc/src/proxy.rs#L78),
[router/proxy.rs:456](../../../../crates/router/src/proxy.rs#L456),
`deploy::is_callee_error`). B1's post-landing review found it too coarse for a
*queued* item and narrowed it to `deploy::is_target_gone_error`
([deploy.rs:229](../../../../crates/sdk/src/deploy.rs#L229)), because
"reached and answered" covers both "gone forever" and "busy right now".

B2 needs the same narrowing plus one addition, and unlike B1 it also needs the
opposite adjustment in one place:

**The table is written against `syneroym_rpc::ProxyError`'s actual variants**
([rpc/proxy.rs:87](../../../../crates/rpc/src/proxy.rs#L87)) — the first draft
named `DependencyNotBound`, which exists only on the *WIT-facing*
`proxy::ProxyError` built in `host_capabilities.rs`, and omitted two variants
the worker will genuinely see (review finding):

| Outcome of a queued delivery | Worker does | Why |
|---|---|---|
| `Ok(value)` | Complete, delete | Delivered |
| `Callee` with the reserved in-flight code (§0.5) | **Retry** | The receiver is running this exact item right now. Definitive-looking, temporary in fact — the narrow, documented exception |
| `Callee`, any other code | **Dead-letter** | The target answered no. Nobody will ever see it otherwise: the caller is long gone (§0.7). B1 *completes* the equivalent case because its caller reads the outcome synchronously; a fire-and-forget item has no such reader, so completing it is silent loss |
| `ServiceNotFound`, `UnsupportedTarget`, `PermissionDenied`, `UnsupportedProtocol` | Dead-letter | Terminal, failure-matrix row 9. `PermissionDenied` included deliberately: the gate is deterministic, so retrying re-asks a settled question |
| `Transport`, `Timeout` | Retry per budget | The ordinary retryable case |
| `Internal` | **Retry** per budget | Covers "sandbox engine unavailable" and "native dispatch registry gone", which are shutdown-window states rather than defects in the item. Bounded by the same attempt budget, so a genuine host defect still reaches the DLQ instead of looping |
| The worker's own re-resolution failing (§0.8, §0.20) | Dead-letter | Not a `ProxyError` at all — it happens before `invoke`. A dependency that no longer resolves is the WIT-level `dependency-not-bound` case, raised here by the worker itself |

The third row is the one a reader coming from B1 will get wrong, so it is a
decision (D-B2-11) rather than a mapping table entry.

### 0.11 (Performance) The dedup budget is met by never opening the store for a call that has no key, and that is assertable structurally

[task.md](task.md)'s budget: "Guest dedup lookup — within the existing proxy call
budget. B2 puts a read on the receiver's hot path; it must not widen the call's
own budget."

The load-bearing property is not that the read is fast. It is that **the read
does not happen at all** for the calls that make up the hot path today: every
existing guest call, every native-origin call, every HTTP-bridge call, none of
which carries a key. This mirrors B1's `a_reachable_substrate_never_touches_the_queue`,
which is asserted as "untouched", not as a timing, so it cannot pass by being
fast on a quick machine.

Two tests, both structural (D-B2-16, tests 25 and 26):

- A call with no idempotency key never opens a dedup store — asserted by
  driving the guard with a provider that panics if asked for a service directory.
- The lookup itself is one indexed probe — asserted through `EXPLAIN QUERY PLAN`
  showing `USING INDEX` and no `SCAN TABLE`, the same way
  `an_empty_queue_tick_issues_one_indexed_query_and_no_scan` already pins B1's
  idle-tick budget
  ([async_queue/src/lib.rs](../../../../crates/async_queue/src/lib.rs)).

A wall-clock assertion is deliberately not used: the store is a SQLCipher file
whose first open pays a key derivation, so a timed test would measure the
keystore, and CI noise would decide the result.

**Two things the structural argument does not cover, and both are real costs on
the hot path** (review finding — the first draft cited the SQLCipher key
derivation as a reason not to time the test, without saying that the same
derivation must not be paid per call):

- **The connection is opened once per service and cached** (D-B2-19). A
  `HashMap<String, Queue>` behind a lock, on the router, populated on first use.
  `Queue` is already `Clone` over an `Arc<Mutex<Connection>>`
  ([async_queue/src/lib.rs:159](../../../../crates/async_queue/src/lib.rs#L159)),
  so a cached handle costs one clone. The map is bounded by the number of
  services on the node; entries are dropped when a service is undeployed
  (§0.13's registry check is the same signal).
- **The guard runs on `spawn_blocking`** (D-B2-19). `Queue`'s methods are
  synchronous over a `Mutex`, and every other SQLite consumer in this tree goes
  through `spawn_blocking` for exactly that reason. Calling it inline would park
  a runtime worker thread on a file lock, on the hot path, which is a worse
  version of the problem this budget exists to prevent — and it would be
  invisible to both structural tests.

The cache gets its own assertion beside them (test 27): a second keyed call to
the same target opens no second connection.

### 0.12 (Correctness) The queue's own budget must be sized here too, and `RetryPolicy`'s defaults would dead-letter a chat message in about seven hundred milliseconds

Identical in kind to [B1 plan](slice-b1-implementation-plan.md) §0.12, and it
has to be re-decided rather than inherited, because B1's five knobs live on
`SupervisorRole` ([config.rs:607](../../../../crates/core/src/config.rs#L607))
and a substrate hosting guests may run no supervisor at all.

**Five fields on `AppSandboxRole`**
([config.rs:438](../../../../crates/core/src/config.rs#L438)), named exactly as
B1's are on its own role, with a second `From<&AppSandboxRole> for QueueConfig`
beside the existing one
([async_queue/src/lib.rs:62](../../../../crates/async_queue/src/lib.rs#L62)):

| Field | Default | Reasoning |
|---|---|---|
| `queue_tick_secs` | **5** | Same as B1's, for the same reason: recovery is bounded by a worker tick, and finer buys nothing when the wait is for a peer to come back |
| `queue_max_attempts` | **54** | The same ~10-hour window B1 chose and B1's `the_configured_defaults_give_a_ten_hour_window` pins. A chat message queued at 22:00 must still be deliverable at 07:00 — the same overnight argument, with a more literal consumer |
| `queue_max_backoff_secs` | **900** | The ceiling the curve settles at; the early retries stay sub-second, so a peer that blipped is served immediately |
| `queue_visibility_timeout_secs` | **120** | Four times `DEFAULT_PROXY_CALL_TIMEOUT`, B1's own derivation, and here the two numbers are literally the same call budget |
| `queue_dlq_max_rows` | **1000** | Per `group_key`, which for this queue is the **target** (§0.14) |

`AppSandboxRole` is the right home because `enqueue` is reachable only from a
guest, so the queue exists exactly where the sandbox does.
[task.md](task.md)'s migration-impact line "AppSandboxRole is untouched" was
written about B5's deferred `task_epoch_timeout_secs` and is corrected rather
than contradicted (§5).

**The dedup TTL is derived, not configured** (D-B2-12) — this answers the
milestone plan's §5 question 2 in the direction that question leaned. A receiver
must remember a key for at least as long as any sender might still be retrying
it, and that number is already fixed by the fields above: the nominal total
window (the sum `backoff_before_wait` produces over `max_attempts − 1` waits,
already a public helper) plus one visibility timeout of margin. Making it a
sixth field would let an operator set a TTL shorter than the retry window, which
silently converts dedup into no dedup — a knob whose only use is to break the
guarantee should not exist.

### 0.13 (Correctness) An undeployed service's queued calls outlive it, because nothing removes a service's data directory

`undeploy` removes the endpoint registrations, the owner row, and the instance
certificate; nothing in the tree removes `<db_dir>/services/<service_id>/`
(there is no `remove_dir_all` anywhere in `data_db` or `control_plane`). So
`state.db` already survives an undeploy, deliberately — and `async.db` would
inherit that, including a queue of calls the service can no longer make.

**Rule (D-B2-15): the worker only drains services the endpoint registry still
knows.** Items for a service that is gone are completed silently, not delivered
and not dead-lettered — the same call B1's post-landing review made for a
retired instance's still-queued item, and for the same reason: delivering
resurrects intent an operator withdrew, and dead-lettering raises noise about a
service nobody is going to act on.

The orphaned file itself is left in place, consistent with `state.db`'s existing
behavior, and gets a backlog row rather than a new deletion path this slice
invents (§4).

### 0.14 (Understated) The queue's two opaque keys already fit this slice, and picking them wrongly costs a correctness property

`Queue` takes a `group_key` and a `queue_key` and parses neither
([async_queue/src/lib.rs:241](../../../../crates/async_queue/src/lib.rs#L241)).
B1's choice — `(instance, logical_ref, substrate)` as the queue key, the app
instance as the group — encoded two invariants that B2 gets for free if it
chooses well and loses if it does not:

- **`queue_key` = the idempotency key.** `Queue::has_pending`
  ([:277](../../../../crates/async_queue/src/lib.rs#L277)) then makes a second
  `enqueue` of the same logical operation a no-op **at the sender**, before any
  wire traffic, and `Queue::replay`'s refusal to create a second pending row for
  one key ([:512](../../../../crates/async_queue/src/lib.rs#L512)) holds
  unchanged. A guest that retries its own `enqueue` after a crash — exactly what
  a chat client does — gets one queued message, not two.
- **`group_key` = the target** (the dependency name, or the DID for a raw
  target). The DLQ cap and its oldest-first eviction are scoped per group
  ([:442](../../../../crates/async_queue/src/lib.rs#L442)), so one permanently
  broken recipient cannot evict the dead letters of every other conversation.
  Grouping by *caller* would be useless here — the file is already per-caller.

Neither needs a queue-crate change. Worth writing down because the crate accepts
any two strings and would be silently worse with the obvious ones.

### 0.15 (House rule) The shipped B1 tree cites planning IDs in code and WIT comments, in about forty places

`M05B Slice B1`, `D-B1-5`, `M05B B1 review finding 4`, and similar appear in
[async_queue/src/lib.rs](../../../../crates/async_queue/src/lib.rs),
[app_supervisor/src/outbox.rs](../../../../crates/app_supervisor/src/outbox.rs),
`app_supervisor/src/service.rs`, [sdk/src/deploy.rs](../../../../crates/sdk/src/deploy.rs),
and [supervisor.wit](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L231).
The house rule in [AGENTS.md](../../../../AGENTS.md) forbids exactly this: these
plans get archived and renumbered, and the comment then points at nothing.

**B2 writes none**, and cleans up only the comments in the lines it actually
edits — a slice should not carry a forty-site rewrite of another slice's files.
The rest is a backlog row with a pickup trigger (§4). Recorded here because
"copy the neighbouring comment style" is the natural thing an implementer does,
and in these files that reproduces the violation.

### 0.16 (Correctness, blocking) The dedup record's identity is undefined for exactly the callers this slice is for, and one of the three possibilities is unusable

ADR-0023 §4 fixes the record as `(caller DID, key)`. At the receiver, a
guest-originated call arrives with one of three identities:

| Path | What the receiver has | Why |
|---|---|---|
| Remote, calling service holds an unexpired instance certificate | A verified caller DID | `invoke_remote_at` presents the instance key and delegation ([router/proxy.rs:429](../../../../crates/router/src/proxy.rs#L429)); `build_caller` verifies them |
| Remote, no certificate or an expired one | **Nothing** — `preamble.pubkey` stays `None`, and the WASM arm of `dispatch_json_rpc_once` admits an anonymous caller deliberately ([dispatch.rs:64](../../../../crates/router/src/route_handler/dispatch.rs#L64)) | An expired certificate is worse than none, so the proxy presents nothing rather than something unverifiable |
| Local | `"system:<service_id>"` — `CallerContext::service_system` ([host_capabilities.rs:1174](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1174)) | The self/cross-service rule the local path already applies |

D-B2-7 unified the *guard* and left the *key it looks up* undefined. The
anonymous row is not merely awkward: an anonymous namespace is shared, so two
different callers using the same key string would read each other's stored
results. That is a correctness and confidentiality failure, not a gap.

**Rules (D-B2-20):**

1. **A keyed call from an unidentified caller is refused**, with the same
   fail-closed shape as D-B2-9. There is no safe namespace to put it in.
2. **`enqueue` refuses at enqueue time when the calling service has no
   unexpired instance certificate** — checked against `registry.instance_cert`,
   which the proxy already consults on the same path. Queuing a call that every
   delivery attempt would refuse, until it dead-letters ten hours later, is a
   worse way to deliver the same "no" (loud, immediately, to a live guest).
   A certificate that expires *while* an item waits is a **retry**, not a
   terminal failure: renewal is the supervisor's ordinary work.
3. **The local `system:<id>` namespace and the remote DID namespace are
   disjoint, and that is correct rather than tolerated.** Whether a given target
   member is local or remote is a property of `(this node, that member)`, not of
   an attempt — a retry never changes it, and re-resolution to a different
   member is a different target with a different store anyway. Test 11 asserts
   the invariant that matters: one caller reaches one target under one identity
   on every attempt.

### 0.17 (Correctness, blocking) A node-level native interface has no DEK and no service directory, and asking for one would create a database for a service that does not exist

Both dispatch entry points have native arms, and native endpoints register under
two kinds of id:

- **A deployed service's own id** — every deployed service auto-registers
  `data-layer`, `vault`, `app-config`, `blob-store`, `messaging`, `http-native`
  as `NativeHostChannel` entries. These have a DEK and a directory; nothing
  special is needed.
- **The node's own service id and `SUPERVISOR_DISPATCH_ID`** for `orchestrator`,
  `security`, and `supervisor`
  ([runtime.rs:609](../../../../crates/substrate/src/runtime.rs#L609)). These are
  **not deployed services**. They have no `state.db`, no DEK, and no business
  having one.

The dangerous part is that asking anyway *works*: `SERVICE_ID_REGEX` accepts
those ids so `resolve_service_db_dir` returns a path, and `resolve_dek` falls
through to `generate_dek` on a miss
([sqlite.rs:1403](../../../../crates/data_db/src/sqlite.rs#L1403)). The
first-draft rule would therefore have minted a DEK and created
`services/<node did>/async.db` before refusing the call.

**Rule (D-B2-21): the guard opens a store only for a target that is a deployed
service**, established by `StorageProvider::service_exists` (which checks for
`state.db` without creating it) before any DEK is resolved. A keyed call whose
target is a node-level interface is **refused** — the key promises a guarantee
this receiver cannot give, and no caller sends one today (the supervisor's own
`orchestrator` traffic is fenced by generations and epochs, not by keys, which is
ADR-0023 §1's whole argument). `enqueue` refuses the same targets up front, for
§0.16's reason: fail loudly at the call, not silently ten hours later.

A guest's self-proxy into its **own** `data-layer` is unaffected — that is a
deployed service, and it is also local and always reachable, so it never needs
the durable path at all (§0.20 refuses to queue it for a different reason).

### 0.18 (Scope, blocking) `ProxyRouter` holds none of the handles this slice needs, and the first draft's "no new handle" was a claim about `HostState`

`ProxyRouter`'s fields are registry, registry client, native dispatch, sandbox
engine, hop, node identity, retry policy
([router/proxy.rs:151](../../../../crates/router/src/proxy.rs#L151)). B2 needs
three things it does not have: a `StorageProvider` and a `KeyStore` to open
`async.db`, and a `LogicalResolver` to re-resolve a queued dependency at delivery
(§0.8).

D-B2-4's "no new crate, no new handle, no new field" is true of `HostState`,
which already holds the `Weak<dyn ServiceProxy>` the `enqueue` host function
needs. It is false of the router, and the real surface is:

| Change | Size |
|---|---|
| `ProxyRouter::new` gains parameters | ~10 call sites: [route_handler.rs:220](../../../../crates/router/src/route_handler.rs#L220) plus nine in-file tests and benches. All mechanical; a `ProxyRouterDeps` struct is worth considering at seven arguments already |
| `syneroym-router` gains `syneroym-async-queue` | New dependency |
| `syneroym-router` gains `syneroym-app-orchestration` as a **real** dependency | Currently dev-only (`Cargo.toml:50`) — `LogicalResolver` lives at `app_orchestration/src/resolver.rs:523`. `syneroym-data-db` is already a real dependency, so `StorageProvider`/`KeyStore` cost nothing here |
| Coordinator mode | `RouteHandlerInner`'s `key_store`/`storage_provider` are `Option` ([route_handler.rs:118](../../../../crates/router/src/route_handler.rs#L118)) and are `None` there — **a third "no store" case**, distinct from a locked vault and an I/O error, and the one D-B2-9 had not covered (§0.4's table now does) |

Recorded as a finding rather than folded in, because "one new module, two call
sites" understates the merge by about an order of magnitude, and the
`ProxyRouter::new` churn is what phase 2 will actually spend its diff on.

### 0.19 (Scope, blocking) Nobody owned the worker

Phase 3 specified the worker's outcome mapping, its poison-pill ceiling, its
undeployed-service rule, and its shutdown behavior — and never said what
constructs it or where its future is raced. B1's worker had an obvious home
beside the supervisor's resident loop; B2's has no equivalent, because the
substrate has no per-node loop of this shape.

**Rule (D-B2-22): `RuntimeServices` owns it** — spawned in
[runtime.rs](../../../../crates/substrate/src/runtime.rs) beside the other
role-shaped tasks, its join handle raced in the same `tokio::select!` that
already carries the connection router, health, metrics, and B1's own
`queue_worker_join`, and **not** awaited on shutdown (B1's D-B1-8, plus its
post-landing correction: cancellation is raced *into* a delivery, not only
against the next tick). It is constructed only when the sandbox role is enabled,
which is the same condition that makes a guest — and therefore the queue's only
producer — possible at all.

**This adds `crates/substrate/src/runtime.rs` to B2's file list**, and the
M05C comparison in §2 is updated with it. It does not change the conclusion:
M05C's collision table names no substrate-runtime file.

### 0.20 (Correctness) The stored intent could not be re-resolved or authorized at delivery

D-B2-8 stored the dependency name and routing key. Resolution needs one more
field, and delivery needs a rule the first draft never stated:

- **`app_instance_id`.** `LogicalServiceRef` is `(app_instance_id, service_name)`,
  and the host reads the instance id from `HostState`, not from the guest
  ([host_capabilities.rs:1116](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1116)).
  A worker running long after that `HostState` is gone must have it stored.
- **The caller identity to deliver under.** The live path derives it at call
  time: the guest's own `CallerContext` when the target *is* its own service,
  and `CallerContext::service_system(component_id)` otherwise. Only the second
  is reconstructible later.

**Rules (D-B2-23):** the payload stores `app_instance_id`, the calling service
id, the dependency name (or raw DID), the routing key, interface, method,
params, the idempotency key, the protocol tag, and the timeout. The worker
rebuilds `CallerContext::service_system(caller_service_id)` and
`CallOrigin::Guest { service_id: caller_service_id }` — identical to the live
cross-service path, so authorization at the receiver is unchanged and no new
model appears.

**And `enqueue` refuses a call whose resolved target is the calling service
itself.** That is the one case whose live identity (the guest's own forwarded
caller) cannot be rebuilt — and it is also a local call to a service that is by
definition running, so durability buys it nothing. Refusing is both the safe and
the honest answer.

### 0.21 (Correctness) The dedup table has a time bound but no row bound, and failure-matrix row 12 asks for both

Row 12: "Queue growth is unbounded → It is not: completed items are deleted,
dead letters are bounded and prunable, and the bound is asserted."

The plan bounds the outbox by the attempt budget and the DLQ by
`queue_dlq_max_rows` per group. The dedup table had only a TTL — a *time* bound.
A receiver taking keyed calls faster than the TTL expires them grows without a
ceiling, and each row can also carry an arbitrarily large stored result.

**Two bounds (D-B2-24), each with its own honest consequence:**

| Bound | Value | What happens at the limit |
|---|---|---|
| Rows, per service | `dedup_max_rows`, default 10,000, pruned oldest-first by `expires_at` on write — the same "a number and a trigger" shape D-B1-9 uses | A pruned record no longer answers, so a duplicate arriving afterwards **re-executes**. That is at-least-once behaving as specified, not a new failure mode, and pruning the oldest first means the record most likely to still matter is the last to go |
| Stored result size | `dedup_max_result_bytes`, default 64 KiB | A larger result is recorded as done **without its body**. A duplicate then gets a reserved-code callee error saying the call already ran and its result was not retained — which keeps row 8's load-bearing half ("the target is not re-executed") while being truthful about the half it cannot keep |

Both are constants derived beside the queue's own knobs rather than two more
config fields, for D-B2-12's reason: the only interesting setting is one that
breaks the guarantee.

### 0.22 (Coverage) The operator surface was named but not specified, and the two verb names it wanted are already taken on another interface

D-B2-13 said "`dead-letters`/`replay` on the `orchestrator` interface" and
stopped there. Four things were missing, and one of them is a collision:
`supervisor.wit` already exports `outbox`, `dead-letters`, and `replay`
([supervisor.wit:231](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L231)),
keyed by app instance. Two same-named verb pairs on two interfaces, keyed
differently, is how an operator ends up reading the wrong list.

**The surface (D-B2-25):**

```wit
// on `orchestrator`, beside the existing per-service verbs
proxy-outbox:       func(service-id: string) -> result<list<proxy-queued-call>, string>;
proxy-dead-letters: func(service-id: string) -> result<list<proxy-dead-letter>, string>;
proxy-replay:       func(service-id: string, dead-letter-id: u64) -> result<_, string>;
```

- **`proxy-` prefixed**, so neither the WIT names nor the `roymctl` subcommands
  collide with the supervisor's per-instance ones.
- **`proxy-outbox` is included from the start.** B1 shipped without its
  equivalent and its post-landing review had to add one, because the e2e could
  not assert that an item was queued, survived a restart, and then left — the
  same three assertions test 56 needs.
- **Enumeration is by service id**, which the operator already has from
  `status`; the verbs do not scan the filesystem for queue files, so an
  undeployed service's leftover file (§0.13) is not listable and does not need
  to be.
- **`roymctl svc proxy-outbox|proxy-dead-letters|proxy-replay <service-id>`**,
  under `svc` rather than `supervisor` because these are per-service and
  node-local.
- Gated exactly as the neighbouring `orchestrator` verbs are (the deploy-grant
  admission gate), no new resource namespace.

The milestone plan's closeout list also owes `developer-guide.md` an operator
section for DLQ verbs; that is now carried in §5.

---

## §1 — B2 decisions

| ID | Decision |
|---|---|
| **D-B2-1** | **An idempotency key is what admits an item to the DLQ, not a failure** (§0.1). An unkeyed `call` that exhausts its retries fails directly, exactly as today; a keyed one also writes a dead letter; an `enqueue`d call is always keyed. This is the only reading under which failure-matrix rows 4 and 7 are both true. |
| **D-B2-2** | **The key travels as an optional member on `JsonRpcRequest`** (§0.2), `#[serde(default)]` and skipped when absent, plus a field on `ProxyRequest` for the in-process path. `call-options` alone reaches no receiver. |
| **D-B2-3** | **One queue per calling service, in `<service dir>/async.db`, SQLCipher-keyed with that service's DEK** (§0.3). Adds `Queue::open_encrypted` and promotes `resolve_service_db_dir` onto `StorageProvider`. ADR-0023 §4's "the substrate's proxy DLQ" is corrected to match at closeout. |
| **D-B2-4** | **The durable path is `CallOrigin::Guest`-only** (§0.9), and `enqueue` is a second method on `ServiceProxy`, reached through the `Weak<dyn ServiceProxy>` `HostState` already holds — no new handle **on the guest side**. The router side is not free: see §0.18 for the constructor, dependency, and coordinator-mode surface it does cost. |
| **D-B2-5** | **An `enqueue` with no idempotency key is refused at the host function**, before anything is written and before any resolution happens — failure-matrix row 7, milestone D-B-9, `[PRD-OFF]`'s "unsafe retries fail explicitly". The refusal names the missing field. |
| **D-B2-6** | **The dedup record has three states — in flight (with a claim expiry), done, expired** (§0.5). A duplicate of an in-flight call gets a reserved-code callee error and is never re-executed; a duplicate of a done call gets the stored first outcome, success or callee error alike; a claim past its expiry is retaken and the call runs again. |
| **D-B2-7** | **One dedup guard, called from both receiver entry points** — `ProxyRouter::invoke_local` and `dispatch_json_rpc_once` (§0.6) — driven by the request body, with no per-ingress special case. HTTP-bridge traffic already routes through the second of those, so it is covered rather than excluded; what B2 does not build is an HTTP-native way to express a key. |
| **D-B2-8** | **A queued call stores intent, not a resolved target** (§0.8): the dependency name and routing key, re-resolved at delivery. A target that no longer resolves is terminal. The full stored shape, including the `app_instance_id` resolution needs and the caller identity delivery needs, is D-B2-23. |
| **D-B2-9** | **The dedup guard fails closed.** A keyed call whose dedup store cannot be opened — a locked vault, an I/O error, or a node with no storage provider at all — is refused, not executed (§0.4's four-case table). **Encryption disabled for the deployment is not one of those cases**: the store opens unencrypted, exactly as `state.db` does in that mode. The outbox worker, facing the same locked vault, skips and retries — opposite answers, because one risks a double execution and the other risks only a delay. |
| **D-B2-10** | **Try-then-queue, inherited from B1** (D-B1-1, ADR-0023 §2). `enqueue` attempts the call synchronously first and writes to the outbox only on a transport failure. A reachable target costs one call and zero queue writes; the guest's fire-and-forget contract is unchanged either way. |
| **D-B2-11** | **On the queued path, a non-transport failure dead-letters rather than completing** (§0.10) — the reverse of B1's mapping, because a fire-and-forget item has no caller left to read the error. The one exception is the reserved in-flight dedup code, which retries. |
| **D-B2-12** | **The dedup TTL is derived from the queue's own retry window, never configured** (§0.12), answering the milestone plan's §5 question 2. A configurable TTL's only distinct setting is one that breaks dedup. |
| **D-B2-13** | **The proxy DLQ's operator surface is metrics, a warning log, and three verbs — not an alert row** (§0.7). The verbs are specified in D-B2-25; they are gated exactly as their neighbours on that interface are, with no new resource namespace, the mistake D-A5-24 corrected. |
| **D-B2-14** | **Replay re-enqueues; it never executes inline** (ADR-0023 §5, D-B1-7), reusing `Queue::replay` unchanged — including its refusal to create a second pending row for a key that already has one. |
| **D-B2-15** | **The worker drains only services the endpoint registry still knows**; items belonging to an undeployed service are completed silently (§0.13). |
| **D-B2-16** | **The dedup budget is asserted structurally**: no store is opened for an unkeyed call, and the keyed lookup is one indexed probe proved by `EXPLAIN QUERY PLAN` (§0.11). No wall-clock assertion. |
| **D-B2-17** | **`call-options` gains one field, not two.** `idempotency-key` is the only field ADR-0023 §4 ever names; the "two fields" in ADR-0023 §7 and [task.md](task.md)'s migration section is unexplained. A per-call delivery deadline is the plausible second field and is deferred with a backlog row rather than invented here (§5, §4). |
| **D-B2-18** | **`idempotent: bool` stays, and a key implies it.** A call carrying an idempotency key is retry-eligible whatever the bool says — a key is a strictly stronger fence than a caller's assertion. The bool is not removed: it is the only fence an unkeyed caller has, and removing a `call-options` field would break every guest that sets it. |
| **D-B2-19** | **The guard runs on `spawn_blocking`, over a per-service connection opened once and cached** (§0.11). Inline synchronous SQLite on the async hot path, or a SQLCipher key derivation per keyed call, would both defeat the budget this slice is measured against — and neither is visible to a structural test unless the test asks. |
| **D-B2-20** | **Dedup identity is the identity the receiver already verified, and an unidentified keyed call is refused** (§0.16). `enqueue` additionally refuses up front when the calling service holds no unexpired instance certificate; a certificate that expires while an item waits is a retry, not a terminal failure. The local `system:<id>` and remote DID namespaces are disjoint by construction, which is sound because a target member's locality never changes between attempts. |
| **D-B2-21** | **A store is opened only for a target that is a deployed service**, checked with `service_exists` before any DEK is resolved (§0.17). A keyed call to a node-level native interface (`orchestrator`, `security`, `supervisor`) is refused rather than dedup-ed, and `enqueue` refuses the same targets. This is what stops `load_service_dek` minting a DEK and a database for a pseudo-service. |
| **D-B2-22** | **`RuntimeServices` owns the outbox worker** (§0.19) — spawned beside the other role tasks in `runtime.rs`, raced in the same `tokio::select!`, not drained on shutdown, and constructed only when the sandbox role is enabled. |
| **D-B2-23** | **The queued payload carries `app_instance_id`, the calling service id, the target intent, the call itself, and the key** (§0.20), and the worker rebuilds `service_system(caller)` + `CallOrigin::Guest` at delivery — identical to the live cross-service path. **`enqueue` refuses a call whose target resolves to the calling service itself**: its live identity is the one that cannot be rebuilt, and a local self-call has nothing to gain from durability. |
| **D-B2-24** | **The dedup table is bounded by rows and by stored-result size, not only by TTL** (§0.21). A pruned record re-executes on a later duplicate (at-least-once, as specified); an oversized result is recorded as done-without-body and answers a duplicate with a reserved code, keeping "not re-executed" true. |
| **D-B2-25** | **`proxy-outbox`, `proxy-dead-letters`, `proxy-replay` on `orchestrator`, keyed by service id, with `roymctl svc` subcommands** (§0.22). Prefixed because `supervisor.wit` already exports three verbs with the unprefixed names, keyed by app instance. `proxy-outbox` ships in the first cut rather than being added later by a review, which is what happened to B1's. |

---

## §2 — Phase plan and merge order

Five phases. The ordering rule is that **the fence lands before the thing that
needs it**: nothing can be queued for redelivery until the receiver can refuse a
duplicate, so dedup (phase 2) precedes the outbox (phase 3), even though the
outbox is the headline.

1. **The key, end to end, reading nothing.** `idempotency-key` on
   `call-options`; `ProxyRequest.idempotency_key`; the optional
   `JsonRpcRequest` member and its serialization on both the remote hop and the
   local WASM arm; the host function passing it through. No behavior change —
   nothing consults the key yet. This is the phase that proves the carrier
   works before any semantics depend on it. Tests: 1–4.

   **Its diff is wider than three files.** `JsonRpcRequest` is built as a
   struct literal in about fourteen places (`control_plane` ×2, `router` ×3,
   `rpc`, `sandbox_wasm` plus three of its tests, `sdk`, `smoke-tests`,
   `substrate` tests). All are compile-forced and mechanical — `..Default::default()`
   is not available on that type today — but an implementer should expect the
   phase to touch every crate that speaks JSON-RPC, not just the two that
   define the field.
2. **The store, the guard, and the router's new handles.** `Queue::open_encrypted`
   and `StorageProvider::service_db_dir` (D-B2-3; the trait method gets a
   default body so the two test fakes in `sandbox_wasm/src/engine.rs` — at
   `:1858` and `:2009` — keep compiling); the `ProxyRouter::new` parameters and
   the two new crate dependencies (§0.18); the three-state record (D-B2-6), its
   two bounds (D-B2-24), the reserved error codes, the identity rule (D-B2-20),
   the deployed-service check (D-B2-21), fail-closed behavior across all four
   no-store cases (D-B2-9), the derived TTL (D-B2-12), the shared guard on both
   entry points (D-B2-7), and the off-thread cached connection (D-B2-19).
   A key that arrives is now honored; nothing yet sends one twice.
   Tests: 5–28.
3. **The guest outbox and `enqueue`.** `enqueue` on `proxy.wit` and on
   `ServiceProxy` (D-B2-4); the three refusals — no key (D-B2-5), no instance
   certificate (D-B2-20), a node-level or self target (D-B2-21, D-B2-23);
   try-then-queue (D-B2-10); the five `AppSandboxRole` fields (§0.12); the
   payload shape (D-B2-23); the worker and its home in `RuntimeServices`
   (D-B2-22), its outcome mapping (D-B2-11), its claim-count poison-pill
   ceiling (reusing `Queue::max_attempts`), its undeployed-service rule
   (D-B2-15), and its shutdown behavior (abandon, do not drain — B1's D-B1-8,
   and its post-landing correction: cancellation is raced *into* the delivery,
   not only against the tick).

   **The guest fixture lands here, not in phase 5.** `test-components/proxy-test`
   has a `call-peer` driver and no enqueue equivalent; its `wit/deps/proxy` is a
   symlink to the real interface, so the WIT change propagates on its own, but
   `wit/world.wit` and `src/lib.rs` still need an export that calls `enqueue`.
   Written here, tests 29–39 drive the real guest boundary; written in phase 5,
   they would be written against a Rust-level fake and the boundary would be
   exercised for the first time by the e2e. Tests: 29–47.
4. **The DLQ and its operator surface.** Dead-letter admission for a keyed
   synchronous call (D-B2-1); the three `proxy-*` verbs with their dispatch
   arms and gating (D-B2-25); `roymctl svc`; metrics; and the two stale markers
   rewritten to state the invariant with no milestone id (§0.1, §0.15).
   Tests: 48–55.
5. **The e2e and the documents.** A two-node scenario in the shape of
   [durable_outbox_e2e.rs](../../../../crates/substrate/tests/durable_outbox_e2e.rs):
   a guest enqueues to a dependency on a node that is down, the substrate
   restarts, the node returns, the call lands exactly once. Plus §5's document
   corrections. Tests: 56–57.

**What could move:**

- **Phase 1 can merge alone and should.** It is the only phase that touches the
  wire type, and a wire change reviewed on its own is worth one extra merge.
- **Phase 2 cannot merge after phase 3.** Shipping redelivery before dedup is
  shipping unfenced double execution, briefly, on the main branch.
- **Phase 2's `ProxyRouter::new` churn could be its own commit** inside the same
  merge, the way B1 split its dependency sweep from its refactor: ten
  mechanical call-site edits next to the guard's real logic makes the review of
  the second harder than it needs to be.
- **Phase 3 cannot be split** along "config versus worker", for B1's reason: a
  worker is untestable at its real budget without the fields, and fields nothing
  reads are not reviewable.
- **Phase 4 cannot be dropped.** Without it the DLQ is a table nothing reads,
  which is the failure ADR-0023 §5 names by name.
- **Phase 5's e2e is the only part with a real time cost** (two substrates plus
  a restart; B1's own two e2e cases run ~290s and ~400s). The in-process tests
  prove every property; the e2e proves the sequence, and a restart with a live
  queue is not provable in-process.

**Against M05C.** B2 edits `proxy.wit`, `control-plane.wit`, `rpc/proxy.rs`,
`rpc/types.rs`, `router/proxy.rs`, `router/route_handler.rs`,
`route_handler/dispatch.rs`, `sandbox_wasm/host_capabilities.rs`,
`core/config.rs`, `async_queue/src/lib.rs`, `data_db/src/{traits.rs,sqlite.rs}`,
`substrate/src/runtime.rs` (§0.19), `apps/roymctl`, and
`test-components/proxy-test`. M05C's collision table
([M05C plan](../M05C-logical-discovery-overlay/implementation-plan.md) §2) names
`app_supervisor/src/service.rs`, `store.rs`, `supervisor.wit`, and
`app_orchestration/src/models.rs` — **no file overlap**, including the runtime
file the review's finding added.

**There is a design overlap the file comparison misses**, and it is worth
naming because a file-level table is exactly what would hide it: D-B2-8 makes
the outbox worker a **second consumer of `LogicalResolver::resolve`**, and
M05C's own §0.4/D-C-6 changes how that resolver keys foreign entries in slice
S2. Under the agreed sequence (M05B first, then M05C) this costs nothing — S2
will simply find two callers instead of one. If the sequence is ever relaxed,
this is a semantic conflict no file table would catch, and M05C's
parallel-running convention note does not cover a case of this shape.

---

## §3 — B2 tests

**Numbering restarts at 1 for this slice.** B1's list used 1–34 for itself; the
milestone plan's "numbering is per-milestone" wording did not survive B1's own
usage, and continuing from 35 would be worse than restarting. e2e cases are
marked; everything else is a unit test.

**Phase 1 — the carrier:**

1. `an_idempotency_key_survives_the_json_rpc_round_trip` — serialize and parse a
   `JsonRpcRequest` carrying one
2. `a_request_without_a_key_serializes_exactly_as_it_does_today` — the field is
   skipped, not emitted as `null`; asserted on the produced bytes, so an old
   receiver sees an identical frame
3. `a_frame_from_a_sender_that_never_sends_a_key_parses` — the reverse
   direction, `#[serde(default)]`'s actual job
4. `the_host_function_passes_the_guests_key_into_the_proxy_request` — driven
   through the existing `RecordingProxy` fake in
   `sandbox_wasm/src/host_capabilities.rs`'s tests

**Phase 2 — receiver-side dedup:**

5. `a_call_with_a_fresh_key_executes_and_records_its_result`
6. `a_repeat_of_a_completed_call_returns_the_first_result_and_does_not_execute`
   — failure-matrix row 8's stated half; the target is a counter that would
   observe a second execution
7. `a_repeat_of_a_completed_call_that_failed_returns_the_same_callee_error` —
   the half row 8 does not mention: a callee error is a real first result
8. `a_repeat_while_the_first_call_is_still_running_is_refused_and_not_executed`
   — §0.5's in-flight state, with the reserved code asserted by value
9. `a_claim_whose_expiry_passed_is_retaken_and_the_call_runs_again` — the honest
   at-least-once case; asserts against a fake clock, not by sleeping
10. `a_key_is_scoped_to_its_caller` — the same key from two caller DIDs is two
    calls, not one. `(caller DID, key)` is the record's identity, per ADR-0023 §4
11. `one_caller_reaches_one_target_under_one_identity_on_every_attempt` —
    D-B2-20's invariant, and the reason the local and remote namespaces are
    allowed to differ (§0.16)
12. `a_keyed_call_from_an_unidentified_caller_is_refused` — the anonymous case,
    which has no safe namespace at all
13. `a_keyed_call_to_a_node_level_interface_is_refused_and_creates_no_database`
    — D-B2-21. Asserts the *absence* of a generated DEK and of
    `services/<node did>/`, since the dangerous half of this case is the file
    that would be written before the refusal (§0.17)
14. `a_key_is_scoped_to_its_target_service` — one service's dedup store never
    answers for another's. The store opened is the **target's**, with the
    target's DEK (§0.3's table)
15. `a_record_past_its_ttl_does_not_answer_and_is_pruned_on_the_next_write` —
    the bound as a number and a trigger, D-B1-9's discipline reused
16. `the_ttl_is_at_least_the_queues_own_total_retry_window` — D-B2-12's
    derivation, computed from the config rather than a literal, so re-tuning a
    knob cannot silently break dedup
17. `the_dedup_table_is_capped_by_rows_and_prunes_oldest_first` — D-B2-24's row
    bound, the half failure-matrix row 12 asks for that a TTL does not give
18. `an_oversized_result_is_recorded_as_done_without_its_body` — D-B2-24's
    second bound, and that a duplicate still is not re-executed
19. `a_keyed_call_is_refused_when_the_dedup_store_cannot_be_opened` — D-B2-9's
    fail-closed rule, with the vault locked
20. `a_keyed_call_works_with_encryption_disabled_for_the_deployment` — §0.4's
    third case, the one the first draft folded into the locked-vault case
21. `a_keyed_call_is_refused_on_a_node_with_no_storage_provider` — coordinator
    mode, the fourth case (§0.18)
22. `a_failure_before_the_target_ran_leaves_no_claim_behind` — a permission
    denial or an unknown service must not block a corrected retry
23. `a_remote_call_is_deduplicated_at_the_receiving_node` — the
    `dispatch_json_rpc_once` entry point (§0.6)
24. `a_local_call_is_deduplicated_by_the_proxy` — the `invoke_local` entry
    point; 16 and 17 together are what make D-B2-7 a fact rather than an
    intention
25. `a_call_with_no_idempotency_key_never_opens_a_dedup_store` — **the
    load-bearing budget test** (§0.11). Asserted as "untouched", not timed
26. `the_dedup_lookup_is_one_indexed_probe_and_no_scan` — `EXPLAIN QUERY PLAN`,
    the shape B1's idle-tick test already established
27. `a_second_keyed_call_to_one_target_opens_no_second_connection` — D-B2-19's
    cache. Without it every keyed call pays a SQLCipher key derivation, which is
    the cost §0.11 cites as the reason not to write a timing test, and which no
    other test in this list would notice
28. `the_guard_never_runs_sqlite_on_the_async_worker_thread` — D-B2-19's other
    half. Asserted structurally by making the guard's store handle
    `!Send`-unfriendly work observable through a blocking-pool probe rather
    than by timing; if that proves awkward at implementation time, it becomes a
    review item and this plan says so rather than implying coverage (the shape
    B1's own test 1 used)

**Phase 3 — the outbox and `enqueue`:**

29. `an_enqueue_without_an_idempotency_key_is_refused` — failure-matrix row 7,
    D-B2-5. Asserts the refusal names the missing field, so a future slice
    relaxing it must confront the argument
30. `an_enqueue_from_a_service_with_no_unexpired_certificate_is_refused` —
    D-B2-20's second rule: fail at the call, not ten hours later at the DLQ
31. `an_enqueue_to_the_calling_service_itself_is_refused` — D-B2-23. The one
    caller identity that cannot be rebuilt at delivery, and a target that is
    local and running anyway
32. `an_enqueue_to_a_reachable_target_delivers_synchronously_and_never_touches_the_queue`
    — D-B2-10's happy path, asserted as untouched
33. `an_enqueue_to_an_unreachable_target_lands_in_that_services_own_outbox`
34. `the_queued_payload_stores_the_dependency_name_not_the_resolved_did` —
    D-B2-8, ADR-0021 §2's no-snapshot rule
35. `a_delivered_call_carries_the_same_caller_identity_the_live_path_builds` —
    D-B2-23. Compares the worker's rebuilt `CallerContext`/`CallOrigin` against
    what `proxy::Host::call` produces for the same cross-service call, so
    authorization at the receiver cannot silently diverge between the two paths
36. `a_queued_call_resolves_its_dependency_again_at_delivery` — a binding
    re-pushed while the item waited takes effect, which is the entire reason
    for 23
37. `a_second_enqueue_for_the_same_key_while_one_is_pending_is_a_no_op` —
    `Queue::has_pending` at the sender (§0.14); the same failure B1's review
    found the hard way, prevented here by construction
38. `a_queued_item_survives_reopening_the_encrypted_queue_file` — durability
    plus D-B2-3's SQLCipher key in one assertion
39. `the_queue_file_is_unreadable_without_the_services_dek` — failure-matrix row
    13's guest half, asserted by opening the file with no key and expecting
    failure. **Scoped to an encryption-enabled deployment**; in the disabled
    mode the property does not hold and the test does not claim it (§0.4)
40. `a_transport_failure_retries_on_the_configured_schedule`
41. `a_callee_error_on_a_queued_item_dead_letters_rather_than_completing` —
    D-B2-11, the row a reader coming from B1 will expect to behave the other way
42. `an_in_flight_dedup_refusal_retries_rather_than_dead_lettering` — the one
    reserved code that is an exception to 29
43. `a_queued_call_whose_dependency_no_longer_resolves_is_terminal` —
    failure-matrix row 9, and the worker's own pre-`invoke` failure rather than
    a `ProxyError` (§0.10's last row)
44. `a_host_side_internal_error_retries_and_a_permission_denial_does_not` —
    the two rows §0.10's first draft omitted, asserted together because they are
    the pair most easily classified the wrong way round
45. `an_item_whose_claim_count_reaches_the_budget_dead_letters` — the
    poison-pill ceiling, reusing `Queue::max_attempts`; B1's finding 7 applies
    unchanged to a second consumer of the same crate
46. `an_item_for_an_undeployed_service_is_completed_not_delivered` — D-B2-15
47. `shutdown_abandons_an_in_flight_delivery_rather_than_draining` — B1's D-B1-8
    **and its post-landing correction**: the cancellation must interrupt a
    delivery that genuinely never resolves, not merely win a race against the
    next tick. B1's first version of this test was vacuous, and copying it would
    reproduce that

**Phase 4 — the DLQ and its surface:**

48. `an_unkeyed_call_that_exhausts_its_retries_writes_no_dead_letter` — D-B2-1's
    first tier, and the assertion that keeps the refusal rule coherent
49. `a_keyed_call_that_exhausts_its_retries_writes_a_dead_letter_and_still_returns_its_error`
    — the second tier: the DLQ row is additional, never a substitute for the
    caller's own error
50. `proxy_dead_letters_lists_what_the_services_queue_holds`
51. `proxy_outbox_lists_an_item_before_it_lands` — D-B2-25's third verb, the
    one B1 had to add after the fact because its e2e could not see the queue
52. `replay_re_enqueues_and_does_not_execute_inline` — D-B2-14
53. `a_replayed_call_is_deduplicated_at_the_receiver_if_the_first_one_landed` —
    the property that makes replay safe at all, and the reason D-B2-1 requires a
    key. Drives replay against a target that already executed the original
54. `the_dlq_cap_is_scoped_per_target` — §0.14's `group_key` choice; one broken
    recipient cannot evict another conversation's dead letters
55. `the_new_verbs_are_refused_without_the_gate_their_neighbours_use` —
    failure-matrix row 14, extending the existing gating test rather than
    inventing a second list

**Phase 5 — e2e:**

56. **(e2e)** `a_queued_guest_call_to_an_offline_node_lands_after_it_returns` —
    two substrates; the dependency's node is stopped, the caller enqueues, the
    **calling substrate restarts**, the node returns, the call arrives exactly
    once. The restart is the step no in-process test can cover, and "exactly
    once" is asserted at the receiver, not inferred from the sender. Each stage
    is asserted through `proxy-outbox` — queued, still queued as the same item
    after the restart, gone once delivered — rather than inferred, which is the
    correction B1's own review had to make to its equivalent test
57. **(e2e)** `a_permanently_unreachable_target_lands_in_the_dlq_and_replays` —
    the terminal half, mirroring B1's step-8 case

**Every test above belongs to a phase**, and every failure-matrix row this slice
owns has one: row 7 → test 29, row 8 → tests 6/7/8/9 for its stated halves and
17/18 for the in-flight and no-body cases §0.5/§0.21 added, row 9 → tests 43 and
44, row 12's bound → tests 15, 17 and 54, row 13's guest half → test 39, row 14
→ test 55, row 4's listing half → tests 49/50/51, row 5 → tests 52/53.

---

## §4 — What closing B2 closes, and what it deliberately leaves open

**Closes:**

- **[task.md](task.md) exit criterion 5** — the `no DLQ (M5)` markers in
  [router/proxy.rs:458](../../../../crates/router/src/proxy.rs#L458) and
  [rpc/proxy.rs:80](../../../../crates/rpc/src/proxy.rs#L80), rewritten to state
  the invariant D-B2-1 settles, with no milestone id in either.
- **[task.md](task.md) exit criterion 13** — the first WIT change in this
  milestone, so there is finally something for a guest component to rebuild
  against.
- **Failure-matrix rows 7 and 8**, which B1's own exit criterion 2 explicitly
  left to this slice.
- **`[PRD-OFF]`'s second clause** — "unsafe retries fail explicitly" — which
  milestone D-B-9 named and nothing could implement before now.
- **ADR-0023 §4's guest half**: the outbox on the substrate hosting the calling
  service, in that service's encrypted database, with an idempotency key as the
  fence §1's inherited fences could not supply.

**Does not close:** `[PLT-ASY]`'s matrix row (needs B3 and B4 as well),
scheduling, sagas, or anything about long-running tasks (D-B-2).

**An existing backlog row names this slice as its own pickup trigger, and the
answer is "it stays open, with one trigger removed."** The row is *the
queued-delivery terminal decision narrows on error message text, not a
dedicated wire code* — B1's `deploy::is_target_gone_error`, which matches the
literal string `write_bindings_impl` emits for a gone target. Its trigger reads
"…or B2's guest-facing proxy DLQ needs the same terminal/retryable distinction
and re-derives it independently."

B2 does re-derive it (§0.10) and reaches a **different** answer: on the guest
path every callee error dead-letters except one reserved code, so no message
matching is needed and B2 introduces no second instance of the fragility. The
row therefore keeps its first trigger (a wording change to that one string) and
**loses the B2 clause**, edited to record that B2 answered it rather than left
silently. The underlying `control_plane` error-taxonomy question is untouched by
this slice.

**New backlog rows this slice's own decisions create** — each with a pickup
trigger, per the *Mandatory Deferred-Backlog Update* rule:

| Row | Why it is deferred | Pickup trigger |
|---|---|---|
| **A guest cannot read its own dead letters** | The operator can list them; the calling service cannot, so a chat client has no way to show "this message was never delivered" (§0.7). Needs guest-facing WIT surface beyond the one field and one function this milestone committed to | M6 chat needs a user-visible failed-send state |
| **No per-call delivery deadline on `call-options`** | ADR-0023 §7 and task.md promise "two fields" and name only one (D-B2-17). A `deliver-by` field would let a caller declare its own staleness rule, which is the guest-side answer to the payload-expiry problem B1 hit with certificates | A consumer needs a queued call to stop being worth delivering — chat's "don't send this if it is older than N minutes" is the likely first |
| **A substrate has no alert store, so a proxy dead letter raises none** | Failure-matrix row 4's alert half is met by metrics and a log here, not by a row an operator can clear (§0.7) | A second substrate-side condition needs operator-visible state; then it is worth one store, not two mechanisms |
| **An undeployed service's queue file and dedup records are never removed** | Nothing removes a service's data directory at all, so this inherits `state.db`'s existing behavior rather than inventing a deletion path (§0.13) | A service-data removal verb exists |
| **B1's planning-doc ids in code and WIT comments** | About forty sites across five files (§0.15). B2 cleans only the lines it edits; a whole-file sweep belongs to a pass that is not also changing behavior | The next non-behavioral pass over `async_queue`/`app_supervisor`, or milestone closeout, whichever comes first |
| **The HTTP bridge has no idempotency path *of its own*** | Corrected after review: HTTP-bridge traffic already reaches the guard, because both of its paths forward into `dispatch_json_rpc_once` (§0.6). What is missing is an HTTP-native way to express a key and an outbox for a client with no service on this node | An external client needs safe retries against a substrate |
| **A service with no unexpired instance certificate cannot use the durable path** | D-B2-20 refuses `enqueue` for one, because an anonymous caller has no dedup namespace and would be refused at every delivery anyway. Ordinarily invisible — the supervisor renews certificates — but on a node with no supervisor a service can reach this state permanently | A deployment runs deployed services with no certificate-renewing supervisor, or an offline-first identity story makes a durable call possible without one |

---

## §5 — Where the source documents disagree, and what this slice does about each

[task.md](task.md) says the requirements and architecture docs are starting
points and that implementation is expected to deviate and reconcile. Six places
need reconciling, and five of them are in documents written *before* B1 shipped.

| Document | What it says | What this plan does |
|---|---|---|
| ADR-0023 §4 | Three queue owners, the middle one "the substrate's proxy DLQ" | **Corrected at closeout.** Per-service, in the calling service's encrypted database (§0.3). A substrate-wide table would contradict failure-matrix row 13 in the same document set, and the shipped `Queue` puts the outbox and its DLQ on one connection anyway |
| ADR-0023 §7 + [task.md](task.md) migration | "`call-options` gains two fields" | **Corrected to one.** Only `idempotency-key` is ever named; the second is unspecified. A delivery deadline is the plausible candidate and becomes a backlog row (D-B2-17) |
| [task.md](task.md) B2 row + exit criterion 5 | "failed-after-retries proxy calls land in the DLQ" | **Narrowed.** Only keyed calls do (D-B2-1). Unkeyed ones fail directly, because a replayable dead letter for an unfenced call is precisely what the same document's non-goals refuse |
| [task.md](task.md) failure matrix row 8 | "The first result is returned" | **One sentence added** covering the in-flight and expired-claim cases (§0.5). Not wrong, incomplete — and incomplete in the case a retry hits most |
| [task.md](task.md) failure matrix row 4 | "an alert is raised" | **Scoped.** True for B1's supervisor DLQ; the substrate has no alert store, so the proxy DLQ answers with metrics, a log, and a listing verb (§0.7) |
| [task.md](task.md) migration section | "`AppSandboxRole` is untouched" | **Corrected.** Written about B5's deferred epoch field; B2 adds five queue fields there, for the same reason B1 added five to `SupervisorRole` (§0.12) |

| [status.md](status.md) lines 8 and 24 | B2 is "planned (sketch only; owes its own `§0`)" | **Updated when this plan lands** — the `§0` exists now, and a status doc that still says it does not is how a reader concludes there is nothing to read |
| The milestone plan's §3 closeout list | Owes `developer-guide.md` "an operator section for the DLQ verbs" | **Carried here**, extended to the three `proxy-*` verbs (D-B2-25) alongside the supervisor's, since an operator reading that section will otherwise find two DLQs described as one |

One further disagreement is with the **shipped tree** rather than with a
document: B1's plan describes a mapping in which a non-transport failure
*completes* a queued item ([B1 plan](slice-b1-implementation-plan.md) §0.14,
D-B1-15), and that is right for B1 and wrong for B2 (§0.10, D-B2-11). An
implementer copying `deliver_queued_item`'s structure — which is otherwise the
right thing to copy — inherits the wrong classification.

**And one house-rule sweep this slice does owe** beyond §0.15's B1 files:
[proxy.wit](../../../../crates/wit_interfaces/wit/proxy/proxy.wit) itself cites
`M04A Slice A1`, `A.5`, and `A.7` in the comments B2 edits. Those lines are in
scope by §0.15's own rule (clean what you touch), and the interface's own
header comment is one of them.

---

## §6 — Questions for the requester

1. **Should a keyed synchronous `call` really write a dead letter?** D-B2-1's
   middle tier is the one place this plan chooses coverage over minimalism: the
   caller already has the error, so the row buys operator visibility and an
   optional replay, at the cost of a write on a failure path. Dropping it would
   make B2 smaller and would leave [task.md](task.md)'s exit criterion 5 resting
   entirely on `enqueue`. My recommendation is to keep it, because it is what
   makes the rewritten markers describe a mechanism that exists rather than one
   that was decided against.
2. **Is one substrate-wide outbox worker acceptable, or should draining be per
   service?** Now that the worker has an owner (D-B2-22: one task under
   `RuntimeServices`), the question is only about fairness: one service with a
   permanently unreachable target shares its tick budget with every other
   service on the node. A per-service task removes that at the cost of a task
   per deployed service. Recommendation: one worker, with a per-service claim
   limit each tick, revisited only if a measured case appears.
3. **Confirming the dedup TTL is derived, not configured** (D-B2-12, the
   milestone plan's §5 question 2). This plan answers it "derived", which is the
   direction that question already leaned; it is recorded here so the answer is
   attached to the slice that implements it.

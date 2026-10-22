# M06D slice C5 — Sender path: implementation plan

*Reader: the developer (or AI) who implements slice C5. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready, revised 2026-10-22 for C4b and C4c. The owner confirmed
Q1 to Q14 on 2026-10-19, and Q15 to Q17 (new with this revision) on
2026-10-22. You can start Step 1.**

**Base.** C5 builds on C4b (signed preamble, merged as #314) and C4c
(identity inside the encrypted channel, branch
`feat/m06d-c4c-implementation`). Start the C5 branch from `main` after C4c
has merged. The code facts and line numbers in this plan were checked
against the C4c branch on 2026-10-22.

**What changed in this revision.** C4b and C4c changed the router's
outbound path. A node now never presents another party's proof as its own
identity; it signs every preamble it writes, per attempt, inside
`IrohHop::call`; it passes a caller's signed proof on in `on_behalf`; and
`invoke_remote` looks a service up again on `not_hosted_here` and on a
host mismatch. So these parts of this plan changed: the "Other facts" list
(section 2), C5-3, C5-4 and C5-7 (section 3), Step 4.2 (no new preamble
helper: reuse `outbound_identity`), Step 4.4 (the direct attempt, today's
path, the sender identity, and the instance caller), the Step 4.8 tests,
Step 9, and the call-site table (section 5). Every other step is
unchanged: C4b and C4c did not touch `async_queue`, the dedup code, the
registry storage, the deploy code, the Conversation store or the outbox
workers.

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (run it in the background, it takes
  about 20 minutes), the import clean-up pass, the deferred backlog, no
  planning references in code (no "M06D", "C5", "S-3", "R-7", "D-C1-5",
  "C5-4" in code, comments or test names; ADR ids are fine), simple English
  in comments, the 800-line file cap, the 100-line function cap, short match
  arms, the module layout rule (`foo.rs` beside `foo/`, never `mod.rs` under
  `src/`), and no hard-coded ports in substrate tests.
  **This plan names its decisions `C5-1` to `C5-16`. Those ids are for this
  plan only. When a step says "for the reason in C5-n", write the reason
  itself in the comment, never the id.**
- `task.md` in this folder: §1 (X6), §3 slices C1, C4 (forward note), C4b,
  C4c, C5 (both forward notes), C6 (R-7).
- `docs/planning/changes/signed-route-preamble/change.md`: Design (all of
  it) and `## Deviations` `D-C4b-6`, `D-C4b-8`, `D-C4b-9`, `D-C4b-12`,
  `D-C4b-19`, `D-C4c-7`, `D-C4c-8`, `D-C4c-10`, `D-C4c-12`, `D-C4c-15`.
  Also `slice-c4b-implementation-plan.md` §7 (its notes for C5).
- `docs/planning/changes/service-mailbox/change.md`: criteria S-1 to S-9 and
  R-7; Design "Sealed message", "Sender flow", "Forwarded proofs"; Open
  questions; `## Deviations` `D-C1-1` to `D-C1-7`, `D-C2-7`, `D-C2-9`,
  `D-C2-11`, `D-C3-7`, `D-C3-8`, `D-C3-10`, `D-C4-6`, `D-C4-16`, `D-C4-20`.
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) Decisions 5
  and 6, amendment A1;
  [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §4 (the
  receiver fence);
  [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
  §6 and Amendments 2 and 3 (`on_behalf`: a caller's proof passed along a
  call chain, and what it may be used for);
  [ADR-0002](../../../decisions/0002-handshake-authorization-point.md)
  Amendment 1 (proof of possession, forwarding, encryption);
  [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1
  (instance key and certificate).
- The code this slice builds on (read first):
  - `crates/router/src/proxy.rs`, `proxy/router.rs` (`invoke_inner`,
    `invoke_remote`, `resolve_peer_target`, `invoke_rehoming`,
    `outbound_identity`, `instance_identity`, `invoke_remote_at`,
    `invoke_local_guarded`, `enqueue_call`), `proxy/hop.rs` (`RemotePeer`,
    `RemoteHop::call`, `IrohHop::call`, `e2e_err`),
    `preamble/proof.rs` (`OutboundIdentity`),
    `proxy/outbox_forwarding.rs` (`drain_one_outbox`),
    `proxy_outbox.rs` (`disposition_of`), `net_iroh.rs` (`resolve_peer`,
    `record_of`), `peer_records.rs`, `call_dedup.rs`,
    `route_handler.rs:240-290` (`build_proxy`).
  - `crates/async_queue/src/queue.rs`, `queue/parked.rs`, `queue/types.rs`,
    `dedup.rs`.
  - `crates/mailbox_format/src/{lib.rs,message.rs,receipt.rs,error.rs}`
    (`seal`, `Sealed`, `SentDeposit`, `Receipt::verify_signed`).
  - `crates/core/src/mailbox.rs` (wire types and `codes`).
  - `crates/mailbox/src/caller.rs` (`instance_of`: what a deposit and a
    `status` call must present).
  - `crates/rpc/src/proxy.rs` (`ServiceProxy`, `ProxyRequest`,
    `ProxyError::HostMismatch`, `NOT_HOSTED_HERE_RPC_CODE`,
    `STREAM_UNAUTHENTICATED_RPC_CODE`), `crates/rpc/src/native.rs`
    (`CallerContext` with `on_behalf_did`, `CallerContext::verified`,
    `CallerProof` with `signed` and `on_behalf`, `CallerProof::to_pass_on`),
    `crates/rpc/src/conversation.rs` (`ConversationDeliveryState`).
  - `crates/conversation/src/outbox.rs`, `transport.rs`,
    `transport/group_sync.rs`, `store.rs`, `store/message.rs`,
    `store/backup.rs`, `group.rs:370-440`.
  - `crates/core/src/local_registry.rs`, `crates/core/src/storage.rs`,
    `crates/data_db/src/registry_store.rs` (how per-service facts such as
    the owner are persisted).
  - `crates/control_plane/src/service/orchestration/{types.rs,
    deploy/admission.rs, deploy/commit.rs, lifecycle.rs}` (the mailbox
    check at deploy, and undeploy clean-up).

### Terms used in this plan

- **Direct attempt:** one call to the receiver over the normal proxy path.
- **Deposit:** a sealed copy of the call, handed to the receiver's mailbox
  with the mailbox method `deposit`.
- **Parked row:** an outbox row in the `async_queue` parked state (built in
  C1). The crate calls it "parked"; the outboxes show it as `deposited`
  (`D-C1-1`).
- **Deposit context:** the bytes C5 keeps in a parked row
  (`park_context`): the mailbox id and the `SentDeposit`. Never the sealed
  bytes (`D-C2-9`).
- **Receiver fence:** the receiver-side idempotency memory
  (`CallDedupGuard`, ADR-0023 §4).

## 1. What C5 builds, in one paragraph

A sender that calls a receiver with a mailbox now makes one direct attempt
with a short timeout. If the receiver cannot be reached, the sender seals
the call (`syneroym_mailbox_format::seal`) and deposits it at the mailbox
named in the receiver's signed record. The outbox row is then parked
(`deposited`): no more direct attempts, only a `status` poll on its own
interval. A `delivered` receipt that the receiver signed completes the row.
A signed final rejection fails it. Anything else (expired, unknown,
unsigned, a signature that does not verify, a revoked receiver key, an
answer that does not parse) sends the row back to direct attempts. The same
code in `ProxyRouter` serves the proxy outbox and the Conversation outbox
(1:1 `deliver` and group `group-push`). Conversation gets a new delivery
state `deposited`, in Rust, in WIT and in the Hub UI. A receiver with no
mailbox sees no change. C5 also carries R-7 (a longer receiver fence for a
service with a mailbox), because the serial order puts C5 before C6 and C5
alone already creates the double-run risk (see C5-1).

## 2. Points for the owner

These are places where `task.md` or the change doc is ambiguous, or is
stale against the current code. Each one has a recommendation. **The owner
accepted every recommendation, Q1 to Q14, on 2026-10-19.** C4b and C4c do
not change any of those fourteen answers (checked 2026-10-22). **Q15 to
Q17 are new with the C4b/C4c revision; the owner accepted them on
2026-10-22.** The rest of the plan follows the recommendations. Write each one that changes a criterion or a
design text into the change doc as Step 1 and Step 10 say.

| # | Finding | Decision (Q1–Q14 confirmed 2026-10-19; Q15–Q17 confirmed 2026-10-22) | Steps it changes |
| --- | --- | --- | --- |
| Q1 | **R-7 order.** `task.md` says C5 "needs R-7 (C6) in `main` first, or carries R-7 itself", but the serial order is C5 then C6. C5 alone already creates a double run: a direct attempt that ran at the receiver but lost its answer leads to a deposit; nobody pulls it until C6, so it expires after 7 days; the row then goes back to direct attempts, and the receiver's fence (about 10.5 hours today) has forgotten the key. | **C5 carries R-7** (C5-1, Step 3). C6 then only feeds the queue's real `max_ttl_secs` from `register` into the same field. | 3 |
| Q2 | **R-7 row limit "for a service".** R-7 says "the row limit for such a service is 100,000 (the default is 10,000)". In the code the 10,000 cap is **per caller** inside one service (`DedupStore::prune`, which counts `WHERE caller = ?1`), on purpose, so one caller cannot evict another's records. | Keep the per-caller meaning: 100,000 rows per caller for a service with a mailbox. Write this as a deviation. | 3 |
| Q3 | **R-7 sizing source.** C6 says "size the idempotency memory from the queue's own `max_ttl_secs` from `register`, not from a 7-day constant". C5 runs before any `register`. | C5 stores, per service, the mailbox id and a `max_ttl_secs` that starts at `OPEN_MAX_TTL_SECS`. C6 overwrites `max_ttl_secs` from the `register` answer. See C5-2. | 3 |
| Q4 | **Where a method declares mailbox eligibility** (the first task of C5). The sender cannot read the receiver's manifest or WIT. Only the receiver can check the opt-in (R-5, in C6). WIT has no per-function metadata that reaches the runtime, and every Roym service sends all its methods through one WIT function (`invoke`), so a WIT-level flag would mark every Roym method at once. | **Declare it in the app manifest, per service**: `mailbox_methods = ["<interface>#<method>", ...]`, carried in the plan and the deploy manifest. **C6 builds the field and the R-5 check.** C5 only writes the decision into the change doc. On the sender side, eligibility is the *kind of call* (C5-3): a proxy-outbox row (from `enqueue`) or a Conversation `deliver` / `group-push`. A synchronous `call` never deposits. | 1, 10 |
| Q5 | **`D-C1-4` is stale.** It says `unpark` keeps `attempts`, "which is what makes a proxy-outbox row that keeps getting `expired` receipts end". But a deposit never calls `Queue::fail`, so `attempts` never grows across deposit → expired → unpark → deposit. Such a row would cycle every 7 days for ever. | Give the proxy outbox an **age limit of 30 days** from the row's `created_at` (the same as Conversation's `conversation_max_pending_age_secs` default). Expose `created_at` on `QueueItem`. For a target without a mailbox the limit never fires (the attempt budget ends the row after about 10 hours), so S-8 holds. See C5-9. | 2, 6 |
| Q6 | **S-1 "returns no data" for `group-push`.** S-1 names Conversation `deliver` as the one exception. `group-push` also returns data (`GroupPushAck { accepted }`), which the sender parses but does not use. | Extend the exception to `group-push`. Write it in the change doc. | 10 |
| Q7 | **`group-push` freshness on the mailbox path (C6 problem, found now).** The receiver refuses a `group-push` whose `PeerAssertion` time is more than `conversation_max_clock_skew_secs` (24 hours) from its clock (`group_sync.rs:94-100`). A push that waits more than a day at the mailbox will be refused when C6 delivers it. | Not C5 code. Add a forward note for C6: a refused `group-push` from the mailbox should be acked as `delivered` (the call ran; group sync repairs a missed entry), or C6 checks freshness against the sealed send time. Record it in the backlog. | 10 |
| Q8 | **Group message state.** S-6 asks for the new state in "the conversation delivery state". A group message has one message-level state and one state per member (`message_recipients`). The worker deletes any outbox row whose message is not `pending` (`outbox.rs:190`), so setting a *group message* to `deposited` would drop the rows of every other member. | `deposited` is a **member-level** state for groups. The group message stays `pending` until every member settles. 1:1 messages use `deposited` at message level. See C5-11. | 8 |
| Q9 | **Open question: parked rows and `max_pending_rows`.** | Keep the bound (parked rows count, as C1 built it). A parked row holds a payload, which is what the bound protects. Write the answer into the change doc. | 10 |
| Q10 | **Open question: a parked row past its deadline.** | Conversation: past the 30-day age limit, fail; else back to direct (`unpark`). Proxy outbox: past the 30-day age limit (Q5), dead-letter; else back to direct. See C5-8. | 6, 8 |
| Q11 | **Open question: a second claim racing a slow poll.** | No race in this design: each outbox has one worker loop per node, and the loop resolves every row it claimed before it claims again. Every mailbox call is bounded (10 s) well under the visibility timeout (120 s for the proxy outbox). Write the answer into the change doc, and never call non-terminal `fail` on a parked row. | 10 |
| Q12 | **One direct attempt (S-2) and the enqueue probe.** `enqueue_call` already makes one direct attempt with a 2 s budget before it writes the row. The worker's first pass then makes its own direct attempt before it deposits. | Accept two direct attempts for a proxy-outbox row (probe, then worker). The probe's 2 s budget is too short for a deposit (record lookup, address lookup, seal and the mailbox call). Write it as a deviation. | 5 |
| Q13 | **The service-record lookup bound.** `task.md` asks to bound the registry lookup of the service record in `record_of`. The sender can only know a peer has a mailbox when it already holds a stored record for it. | Bound the lookup (3 s) only when the peer record store holds a record with a mailbox for that peer. A sender that has never seen the peer still waits for the full lookup. Write it as a deviation. | 4 |
| Q14 | **Poll and timeout values.** No value is given anywhere. | Direct attempt 5 s; every mailbox call 10 s; poll every 60 s (the receiver pulls every 60 s, R-1); deadline grace 1 hour after the deposit's TTL. Constants, not config, like `ENQUEUE_PROBE_BUDGET`. | 4 |
| Q15 | **New: which refusals of the direct attempt lead to a deposit.** C4b and C4c added three errors that come *before* any dispatch, so nothing ran at the receiver: `-32097` (`STREAM_UNAUTHENTICATED_RPC_CODE`: the receiver did not accept the sender's proof, for example its clock is more than 15 minutes off, or its anchor lookup timed out), `ProxyError::HostMismatch` (the node that answered the handshake is not the host the record names), and `-32096` (`not_hosted_here`: the node no longer hosts the service). The C4b plan (§7) says the mailbox decision must treat `-32097` as "may succeed later", not as final. Today both outboxes retry all three, and each retry spends an attempt (`D-C4b-19`), so a phone with a skewed clock makes messages dead-letter after the attempt budget even though it has a mailbox. | **Deposit on all three**, like `Transport` and `ServiceNotFound`. Each is a failure of the direct attempt (S-2) before dispatch, the sealed message does not depend on the receiver's preamble window (the receiver checks the sealed send time with 24 hours of skew, R-4), and the receiver fence runs the call once if a later direct attempt also lands. A real callee answer (any other `Callee` code) is still returned as it is. | 4 |
| Q16 | **New: re-homing on the direct attempt.** `invoke_remote` now calls `invoke_rehoming`: on `-32096` or a host mismatch it looks the service up again and calls once more (`D-C4b-9`, `D-C4c-8`). S-2 asks for one direct attempt. | The direct attempt on the mailbox path makes **one call and does not re-home**. A `-32096` or a host mismatch there leads to a deposit (Q15). The receiver pulls from its mailbox wherever it is now hosted, and the next direct attempt (after a `Retry` status) resolves the service again anyway. A receiver with no mailbox keeps today's path, re-homing included (S-8). | 4 |
| Q17 | **New: the 5 s direct attempt and the 10 s handshake.** On an indirect connection (the record's address is a forwarder, not the host) `IrohHop::call` also runs the end-to-end handshake, whose own timeout is 10 s (`e2e_channel::HANDSHAKE_TIMEOUT`, `D-C4c-9`). The 5 s direct-attempt budget (Q14) covers connect, handshake, request and answer together. | **Keep 5 s.** A normal handshake is one extra round trip. On a link slow enough to pass 5 s the message is deposited, which is safe: it is delivered from the mailbox, and the fence runs it once. Write it as a deviation so a later tuning pass sees the link. | 4 |

Other facts that this plan relies on (checked in the code on the C4c
branch, 2026-10-22):

- `ProxyRouter::resolve_peer_target` returns `ServiceNotFound` when the
  peer has a mailbox but no address (`proxy/router.rs:306-314`). That is the
  "receiver asleep" case today. `net_iroh::ResolvedPeer` now also carries
  `host_key`, the node key of the host its record names
  (`net_iroh.rs:165-174`). A remote call needs both, as a `RemotePeer
  { addr, host_key }` (`proxy/hop.rs:19-22`).
- `invoke_remote` is `invoke_rehoming` over `resolve_peer_target`
  (`proxy/router.rs:302-349`). On `-32096` or `ProxyError::HostMismatch` it
  resolves again and calls `invoke_remote_at` once more; if the fresh peer
  is the same, a `-32096` becomes `ServiceNotFound` and a host mismatch is
  returned as it is.
- `invoke_remote_at` retries keyed calls up to `retry_policy.max_attempts`
  times (`proxy/router.rs:396-449`). S-2 needs exactly one attempt, so the
  direct attempt on the mailbox path must not use that loop.
- **Who a remote call presents** is decided by `outbound_identity`
  (`proxy/router.rs:366-393`), once per call. A `CallOrigin::Native { service_id:
  Some(s) }` call presents `instance_identity(s)` (the derived instance key
  and the unexpired certificate), else the node key. A `Guest` call
  presents `instance_identity` or nothing. A node **never presents another
  party's proof as its own identity** (`D-C4b-12`); a native call puts
  `req.caller.proof.to_pass_on()` in `on_behalf` instead.
- `CallerProof::to_pass_on` (`crates/rpc/src/native.rs`) returns the
  carried `on_behalf`, else the caller's own proof **only when it is
  signed** (`signed: Some`), else `None`. So an unsigned proof that this node
  builds itself never leaves the node (`D-C4b-6`).
- `IrohHop::call` applies the identity and signs the preamble itself, on
  every attempt (`OutboundIdentity::apply`, `proxy/hop.rs:107`); on an
  indirect connection it runs the end-to-end handshake against
  `peer.host_key` and sends the identity encrypted. No caller builds or
  signs a preamble. `E2eError::HostKeyUnknown` becomes `ServiceNotFound` and
  `BadHostSignature` becomes `ProxyError::HostMismatch` (`e2e_err`,
  `proxy/hop.rs:69-75`). A refusal before dispatch arrives as
  `ProxyError::Callee` with `-32096`, `-32097` or `-32601`
  (`handle_refused_hop`).
- The mailbox reads the sender only from a `CallerContext` with
  `auth = Delegated` (or `Ucan`), `caller_did = master DID`, and a proof
  holding the instance public key and the instance certificate
  (`crates/mailbox/src/caller.rs:26-71`, unchanged by C4b and C4c).
  `CallerContext::service_system` (what both outboxes use today) has no
  proof, so a same-node deposit with it is refused `unauthenticated`. On a
  remote call the mailbox node's router builds this context from the
  signed preamble, so the proof is checked there (SP-2). On a same-node call
  nothing is signed: the context C5 builds is the proof (C5-4).
- A remote mailbox call with `CallOrigin::Native { service_id: Some(sender)
  }` and an unsigned instance proof in `caller` therefore presents the
  sender's instance key and certificate, signed by `IrohHop`, with **no**
  `on_behalf`. That is exactly what the mailbox needs.
- `ProxyError` has a new variant `HostMismatch(String)`. Every exhaustive
  `match` on `ProxyError` that C5 writes must name it.
- Both outboxes already classify `-32096`, `-32097` and `HostMismatch` as
  `Retry` (`proxy_outbox.rs:102-130`, `conversation/src/transport.rs:632-666`,
  `D-C4b-19`, `D-C4c-8`), and `ServiceNotFound` as `Retry` (proxy outbox)
  or `Unreachable` (Conversation).
- The Conversation worker deletes a row whose message is not `pending`
  (`outbox.rs:190-194`). `ConversationStore::set_state`,
  `set_recipient_state` and `restart_pending` lock the connection that
  `Queue::transaction` already holds (`D-C1-5`).
- `recipients_remaining` counts only `state = 'pending'`
  (`store/message.rs:672`). A `deposited` member must count as remaining.
- `async_queue/src/dedup.rs` is 789 lines, with its tests inline. Step 3
  must move the tests out before it adds code.
- `RegistryClient` implements `MasterAnchorResolver`
  (`crates/core/src/dht_registry/client.rs:590`), so `ProxyRouter` can check
  a receiver's revoked keys with the client it already holds.

## 3. Decisions this plan makes (check them before you start)

- **C5-1. C5 carries R-7.** Reason in Q1.
- **C5-2. The mailbox of a hosted service is a persisted registry fact.**
  `EndpointRegistry` gets `set_mailbox` / `mailbox_of` / `remove_mailbox`,
  persisted like the owner (`service_mailboxes` table). The control plane
  writes it at deploy commit from the verified record, and removes it at
  undeploy and at a deploy without a mailbox. The receiver fence reads it on
  every keyed call (a `DashMap` read). C6 reuses it for its pull list.
  Rejected: reading `hosted_apps/<id>.json` (a file read per call, and no
  signal when a redeploy changes it); an in-memory flag only (lost at
  restart); giving every service the long window (8-31 days of fence rows on
  services that never need them, and 10 times the row cap).
- **C5-3. Sender eligibility is the kind of call, plus checks.** A request
  may deposit only when all hold: it comes from a proxy-outbox row or a
  Conversation `deliver`/`group-push` row (the only callers of the new
  method); it has an idempotency key; `req.caller.proof` is `None`; its
  origin names a sending service;
  that service holds an unexpired instance certificate and a recorded owner;
  the receiver's record names a mailbox; and `seal` succeeds (a
  `SealError::BadCertificate` or `TooLarge` keeps the call on direct
  delivery). The receiver-side opt-in is checked by the receiver (C6, R-5),
  which answers with a signed `not_eligible`. That rejection is final, so a
  sender learns it once.
  *Why no caller proof.* A native call that carries a caller's signed proof
  passes it on in `on_behalf` (ADR-0016 Amendment 2), so the receiver can
  answer `resolve-relation` about that person (Amendment 3). A sealed
  message carries only the sending service's certificate, so on the mailbox
  path the receiver would lose the person. Such a call stays on direct
  delivery. Neither outbox sends one today (both use
  `CallerContext::service_system`, which has no proof), so this rule only
  keeps a future caller from losing the person silently.
- **C5-4. One instance caller for every mailbox call.** `deposit` and
  `status` calls carry `CallerContext::verified(master DID,
  AuthLevel::Delegated, Vec::new())` with `proof: Some(CallerProof {
  pubkey_hex: instance public key, delegation_json: Some(certificate JSON),
  ..Default::default() })` (`signed: None`, `on_behalf: None`), and
  `CallOrigin::Native { service_id: Some(sender) }`. The same request works
  both ways. For a same-node mailbox, `invoke_local` hands this context to
  the mailbox, which reads the proof (`instance_of`). For a remote mailbox,
  `outbound_identity` presents `instance_identity(sender)` from the origin,
  `IrohHop` signs it per attempt, and `to_pass_on()` drops the unsigned proof,
  so no `on_behalf` is sent (`D-C4b-6`). This plan never builds or signs a
  preamble itself.
- **C5-5. One shared path, in `ProxyRouter`.** Two new `ServiceProxy`
  methods: `deliver_or_deposit` and `poll_deposits`. Both outboxes call
  them. Each outbox keeps its own row state. The trait lives in
  `syneroym-rpc`, which does not depend on the mailbox crates, so the
  deposit context crosses the trait as opaque bytes.
- **C5-6. The router owns the timing.** `deliver_or_deposit` returns a
  `DepositTicket` with the context, the poll interval and the deadline. The
  outboxes park with those values. One place to tune them.
- **C5-7. Only "not delivered, and nothing ran" errors lead to a deposit.**
  After the direct attempt these deposit: `Transport`, `Timeout`,
  `ServiceNotFound` (this includes `HostKeyUnknown`), `HostMismatch`, and
  `Callee` with `SERVICE_NOT_FOUND_RPC_CODE` (`-32601`),
  `NOT_HOSTED_HERE_RPC_CODE` (`-32096`) or
  `STREAM_UNAUTHENTICATED_RPC_CODE` (`-32097`) (Q15). Every other error (a
  callee's real answer, `CALL_ALREADY_RUNNING_RPC_CODE`, a gate refusal,
  `UnsupportedTarget`, `UnsupportedProtocol`, `PermissionDenied`,
  `Internal`) is returned as it is, and the outbox classifies it as today.
  The direct attempt does not re-home (Q16).
- **C5-8. A failed deposit returns the direct attempt's error.** Then both
  outboxes keep their present retry behaviour (S-4: back to direct, deposit
  again on a later attempt). The one exception: `ttl_too_long` (`-32050`)
  seals once more with `data.max_ttl_secs` and deposits again in the same
  attempt (`D-C4-16`).
- **C5-9. Proxy-outbox age limit, 30 days.** Reason in Q5. The shared
  constant `MAILBOX_SENDER_MAX_AGE_SECS = 30 * 24 * 3600` lives in
  `syneroym_core::mailbox`. The receiver fence for a service with a mailbox
  lasts `max(max_ttl_secs, MAILBOX_SENDER_MAX_AGE_SECS) +
  MAX_CLOCK_SKEW_SECS`, because after an `expired` receipt a sender may make
  direct attempts for the rest of that age, and the fence must outlast the
  sender's whole window (the rule `DedupConfig::derive` already follows).
- **C5-10. Status results map to four outcomes.** `Waiting` (re-park),
  `Delivered` (complete), `Rejected { code }` (fail, final), `Retry { reason
  }` (back to direct). The mapping is in Step 4.6. A transport failure or
  a refusal of the `status` call itself is `Waiting`: the mailbox may be
  down for a while, and the deadline bounds it. An answer that does not
  parse is `Retry` (ADR-0026 A1).
- **C5-11. Group state.** Reason in Q8.
- **C5-12. Status calls are batched per mailbox,** at most
  `MAX_STATUS_BATCH` (100) ids per call. Many messages to one sleeping phone
  is the common case.
- **C5-13. A receipt is accepted only if the receiver's key is not
  revoked.** After `verify_signed` succeeds, read the receiver master's
  anchor and refuse a receipt whose `receiver_certificate.temporary_did` is
  in `revoked_keys` (as `Retry`). If the anchor cannot be read, the result
  is `Waiting` (ask again later), never an accept.
- **C5-14. The TTL of a deposit** is `min(OPEN_MAX_TTL_SECS, max_wait_secs)`,
  where the caller passes `max_wait_secs` = how long the row may still live
  (Conversation: until `conversation_max_pending_age_secs`; proxy outbox:
  until the 30-day limit). The router clamps to the open limit; a dedicated
  mailbox (C8) may raise it later.
- **C5-15. The enqueue probe stays direct-only.** Reason in Q12.
- **C5-16. The relay push stays live.** `drain_relay_pending_once` pushes a
  group entry to a sample of members with no outbox row (`outbox.rs:96-
  133`). There is no row to park, so it never deposits. Group sync repairs
  a missed push.

## 4. Step-by-step changes

Follow the order. Each step builds and tests on its own.

### Step 1 — Write the decisions down first (docs only)

In `docs/planning/changes/service-mailbox/change.md`:

1. `## Open questions`: replace the first, fourth, fifth and sixth bullets
   with their answers (Q4, Q9, Q10, Q11), each one sentence plus "Decided
   (2026-10-19)". Leave the `pull-now` and the values bullets.
2. S-1 row: add `group-push` to the exception (Q6). Change "carries no
   forwarded caller proof" to "is not made on behalf of another caller (it
   carries no caller proof that would travel in `on_behalf`)".
3. Design, paragraph **Forwarded proofs** (line 169) is stale since C4b:
   no node forwards a caller's proof as its own identity any more
   (ADR-0016 Amendment 2). Rewrite it: a native call made for a caller
   passes that caller's signed proof on in `on_behalf`, and the receiver
   uses it to answer `resolve-relation` about that caller (Amendment 3). A
   sealed message carries only the sending service's certificate, so such
   a call is not eligible (S-1). Rename the paragraph **Calls made for
   another caller**.
4. `## Deviations`: add `D-C5-1` … at the end of the slice (Step 11), not
   now. Keep a scratch list while you work.

### Step 2 — `async_queue`: expose `created_at`

File `crates/async_queue/src/queue.rs`, `queue/types.rs`.

- `QueueItem` gets `pub created_at: i64` (Unix ms, when the row was first
  written; a replayed dead letter gets a new one, as `replay` already
  writes).
- `item_columns!()` appends `, created_at` (column index 9).
- `read_item` reads `created_at: row.get(9)?`.
- `QueueItem` is built only in `read_item` (checked), so no other code
  changes.
- Test in `queue/tests.rs`: `claim_due` and `claim_parked_due` return the
  `created_at` the row was written with, and `park`/`unpark` keep it.

### Step 3 — R-7: the receiver fence for a service with a mailbox

#### 3.1 `crates/core`: the persisted mailbox fact

`crates/core/src/mailbox.rs`, add:

```rust
/// How long a sender keeps trying a message to a receiver with a mailbox,
/// direct and by deposit. The receiver fence for such a service lasts at
/// least this long, plus the clock skew allowance.
pub const MAILBOX_SENDER_MAX_AGE_SECS: u64 = 30 * 24 * 3600;

/// The mailbox a hosted service pulls from, as its signed record names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedMailbox {
    /// The node DID of the mailbox host.
    pub mailbox: String,
    /// The longest time-to-live of this service's queue. The open limit
    /// until the receiver has registered and learned the real one.
    pub max_ttl_secs: u64,
}
```

`crates/core/src/storage.rs`, `EndpointStorage` gets three required methods
(no default body: a store that forgets them would silently lose R-7):

```rust
async fn load_all_mailboxes(&self) -> Result<Vec<(String, HostedMailbox)>>;
async fn save_mailbox(&self, service_id: &str, mailbox: &HostedMailbox) -> Result<()>;
async fn remove_mailbox(&self, service_id: &str) -> Result<()>;
```

Implement them in `MockStorage` (a `DashMap`, like the owner map). Also in
the two test stores that implement the trait:
`crates/control_plane/src/service/orchestration/tests/helpers.rs:322`
(`FailingEndpointStorage`) and `crates/router/tests/service_ownership.rs:152`
(`RemoveOwnerFailingStorage`); both delegate to their inner store.

`crates/core/src/local_registry.rs`: a field
`service_mailboxes: Arc<DashMap<String, HostedMailbox>>`, filled in
`load_from_db`, and:

```rust
pub async fn set_mailbox(&self, service_id: String, mailbox: HostedMailbox) -> Result<()>;
#[must_use]
pub fn mailbox_of(&self, service_id: &str) -> Option<HostedMailbox>;
/// Idempotent.
pub async fn remove_mailbox(&self, service_id: &str) -> Result<()>;
```

Same shape as `set_owner` / `owner_of` / `remove_owner`. `local_registry.rs`
is 511 lines; this adds about 35.

#### 3.2 `crates/data_db/src/registry_store.rs`

A table beside `service_owners`:

```sql
CREATE TABLE IF NOT EXISTS service_mailboxes (
    service_id   TEXT PRIMARY KEY,
    mailbox      TEXT NOT NULL,
    max_ttl_secs INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
)
```

Upsert on `save_mailbox` (`ON CONFLICT(service_id) DO UPDATE`), delete on
`remove_mailbox`. No migration: the product is not released. The file is
690 lines; if it would pass 800, move the new table's three methods into a
sibling `registry_store/mailbox.rs` (`impl SqliteEndpointStorage` there,
called from the trait impl). Tests in `registry_store/tests.rs`: save, load
after reopen, upsert, remove.

#### 3.3 `crates/control_plane`: write and clear the fact

- `types.rs`: `ValidatedDeploy` gets `pub(super) mailbox: Option<String>`
  (the mailbox node DID from the verified record).
  `validate_mailbox` returns `Result<Option<(MailboxSecretKey, String)>,
  String>`: the key and `record_mailbox.service_id`. Update its one caller
  (`deploy/admission.rs:113-120`) and its tests
  (`tests/mailbox.rs:91-160`).
- `deploy/commit.rs`, in the same `match` that stores or clears the key
  (lines 64-84):

  ```rust
  Some((key, mailbox)) => {
      store_mailbox_key(...)  // unchanged, with its rollback
      registry.set_mailbox(service_id, HostedMailbox {
          mailbox: mailbox.clone(),
          max_ttl_secs: OPEN_MAX_TTL_SECS,
      })  // on Err: the same undeploy_and_rollback_late_failure, step name "mailbox record"
  }
  None => {
      clear_mailbox_key(service_id).await;
      if let Err(e) = registry.remove_mailbox(service_id).await { warn!(...) }
  }
  ```

  A redeploy to the **same** mailbox must keep a `max_ttl_secs` that C6
  already learned: write `max_ttl_secs` from the existing
  `registry.mailbox_of(service_id)` when its `mailbox` is the same, else
  `OPEN_MAX_TTL_SECS`.
- `lifecycle.rs` `remove_deploy_registry_rows` (line 442): add
  `registry.remove_mailbox(service_id)` with a warning on error, like the
  others.
- Tests (`tests/mailbox.rs`): a deploy with a mailbox sets the fact; a
  redeploy without one removes it; undeploy removes it; a redeploy to the
  same mailbox keeps a raised `max_ttl_secs`.

#### 3.4 `crates/async_queue/src/dedup.rs`: a per-call window

First move the inline `#[cfg(test)] mod tests { ... }` (from about line
470 to the end) to `crates/async_queue/src/dedup/tests.rs`, with
`#[cfg(test)] mod tests;` at the end of `dedup.rs`. No other change in that
move (separate commit).

Then:

```rust
/// How long a record answers, and how many one caller may hold. Chosen per
/// target service on each call, so one store can serve both windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupWindow {
    pub ttl_ms: u64,
    pub max_rows: u32,
}

impl DedupConfig {
    #[must_use]
    pub fn window(&self) -> DedupWindow {
        DedupWindow { ttl_ms: self.ttl_ms, max_rows: self.max_rows }
    }
}
```

- `DedupStore::begin(&self, caller, key, now)` becomes
  `begin(&self, caller, key, now, window: DedupWindow)`. Inside, replace
  `self.config.ttl_ms` with `window.ttl_ms` and the `max_rows` passed to
  `prune` with `window.max_rows`.
- `DedupStore::finish(.., now)` gains `window: DedupWindow` and uses
  `window.ttl_ms` for `expires_at`.
- `claim_window_ms` and `max_result_bytes` stay in `DedupConfig`.
- Update the moved tests to pass `config().window()`.
- New tests: a record written with a long window outlives the short
  window; the per-caller cap uses the window's `max_rows`.

#### 3.5 `crates/router/src/call_dedup.rs`: choose the window

`crates/router/Cargo.toml`: add `syneroym-mailbox-format` (for
`MAX_CLOCK_SKEW_SECS`; `syneroym-core` already pulls it in).

```rust
/// The row cap per caller for a service with a mailbox. A message may reach
/// such a service directly and again from its mailbox, days apart.
const MAILBOX_DEDUP_MAX_ROWS: u32 = 100_000;

impl CallDedupGuard {
    /// The fence window for `target_service`: the node's own default, or a
    /// longer one when the service has a mailbox. A sender may still try a
    /// message to it, directly or by deposit, for its whole age limit.
    fn window_for(&self, target_service: &str) -> DedupWindow {
        let base = self.config.window();
        let Some(mailbox) = self.registry.mailbox_of(target_service) else { return base };
        let wait_secs = mailbox.max_ttl_secs.max(MAILBOX_SENDER_MAX_AGE_SECS);
        let ttl_ms = wait_secs.saturating_add(MAX_CLOCK_SKEW_SECS).saturating_mul(1000);
        DedupWindow {
            ttl_ms: ttl_ms.max(base.ttl_ms),
            max_rows: MAILBOX_DEDUP_MAX_ROWS.max(base.max_rows),
        }
    }
}
```

- In `begin`, compute `let window = self.window_for(target_service);`
  before the `spawn_blocking`, pass it to `store.begin(..)`, and keep it in
  the `DedupClaim` (`window: DedupWindow` field) so `settle` passes it to
  `finish`.
- Both entry points already call `CallDedupGuard::begin` with the
  *resolved* service id (`invoke_local_guarded`, and the route handler's
  `dispatch_json_rpc_once`), so no call site changes.
- Tests in `call_dedup/tests.rs` (448 lines): a keyed call to a service with
  `registry.set_mailbox(..)` is recorded with `expires_at ≈ now + 31 days +
  24 h`; one without stays at the default; removing the mailbox goes back
  to the default for new records.

### Step 4 — `ProxyRouter`: deposit and poll

#### 4.1 `crates/rpc/src/proxy.rs`: types and trait methods

```rust
/// What a delivery that may use the receiver's mailbox did.
#[derive(Debug, Clone, PartialEq)]
pub enum Delivery {
    /// The receiver answered a direct attempt.
    Direct(Value),
    /// The receiver could not be reached; a sealed copy waits at its mailbox.
    Deposited(DepositTicket),
}

/// How an outbox waits for a deposit. `context` is opaque to the outbox: it
/// stores it and gives it back to `poll_deposits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositTicket {
    pub context: Vec<u8>,
    pub poll_interval: Duration,
    /// Unix ms. After this, a deposit that is still waiting is treated as
    /// lost, and the message goes back to direct attempts.
    pub deadline_at_ms: i64,
}

/// What the receiver's mailbox says about one deposit, already checked
/// (ADR-0026 A1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepositStatus {
    /// Still at the mailbox, or the mailbox could not be asked now.
    Waiting,
    /// The receiver signed a delivery.
    Delivered,
    /// The receiver signed a final rejection.
    Rejected { code: String },
    /// Expired, unknown, unsigned, or an answer that cannot be trusted:
    /// deliver directly again.
    Retry { reason: String },
}
```

Export all three from `crates/rpc/src/lib.rs`.

`ServiceProxy` gets two methods with default bodies (a proxy with no
mailbox support delivers directly and never has a deposit to poll):

```rust
/// A delivery for an outbox row that may wait at the receiver's mailbox.
/// `max_wait_secs` is how much longer the row may live.
async fn deliver_or_deposit(
    &self,
    request: ProxyRequest,
    max_wait_secs: u64,
) -> Result<Delivery, ProxyError> {
    let _ = max_wait_secs;
    self.invoke(request).await.map(Delivery::Direct)
}

/// One status per context, in the same order.
async fn poll_deposits(&self, sender_service_id: &str, contexts: Vec<Vec<u8>>) -> Vec<DepositStatus> {
    let _ = sender_service_id;
    contexts
        .iter()
        .map(|_| DepositStatus::Retry { reason: "this proxy cannot read a mailbox".to_string() })
        .collect()
}
```

Every other `ServiceProxy` implementation (test doubles in
`sandbox_wasm`, `app_host_native`, `conversation`, `roym_core`) keeps
compiling through the default bodies. Grep `impl ServiceProxy for` and
`impl syneroym_rpc::ServiceProxy for` to confirm.

#### 4.2 Two small remote helpers (no behaviour change)

`crates/router/src/proxy/router.rs` (640 lines). C4b and C4c already split
*who* a call presents (`outbound_identity`, once per call) from *signing*
(`IrohHop::call`, on every attempt). Build on that. Do **not** add a
function that builds or signs a preamble, and never sign one in the proxy
and reuse it: a retry, a re-home or a handshake must sign again at the
moment it writes (`D-C4b-8`, the C5 forward note in `task.md`).

```rust
/// The JSON-RPC request every remote call sends. Moved out of
/// `invoke_remote_at` so a single attempt sends the same bytes.
fn outbound_request(req: &ProxyRequest) -> JsonRpcRequest;

/// Exactly one call to `peer`, with no retry and no re-homing, bounded by
/// `timeout`. It presents the same identity as any remote call; the hop
/// signs it when it writes the preamble.
pub(super) async fn invoke_remote_once(
    &self,
    peer: &RemotePeer,
    req: &ProxyRequest,
    timeout: Duration,
) -> Result<Value, ProxyError> {
    let route = RoutePreamble::binary_json_rpc(&req.target_service, &req.interface);
    let identity = self.outbound_identity(req);
    self.hop.call(peer, &route, identity.as_ref(), &Self::outbound_request(req), timeout).await
}

/// `invoke_remote` for a peer the caller has already resolved: the same
/// retry loop and re-homing, without a second lookup first.
pub(super) async fn invoke_remote_resolved(
    &self,
    first: RemotePeer,
    req: &ProxyRequest,
) -> Result<Value, ProxyError> {
    let mut first = Some(first);
    self.invoke_rehoming(req, || {
        let ready = first.take();
        async move {
            match ready {
                Some(peer) => Ok(peer),
                None => self.resolve_peer_target(&req.target_service).await,
            }
        }
    })
    .await
}
```

`invoke_remote_at` keeps its signature and its retry loop, and uses
`outbound_request`. `invoke_remote` is unchanged. If `router.rs` would pass
800 lines, put the two new functions in `proxy/mailbox.rs` (Step 4.4)
instead. The existing `remote_dispatch.rs`, `rehome.rs` and `on_behalf.rs`
tests must pass unchanged.

#### 4.3 `crates/router/src/net_iroh.rs`: bound the service-record lookup

```rust
/// How long a sender waits for the registry when it already holds a stored
/// record with a mailbox for the peer. That record can answer at once, and a
/// registry that does not answer would otherwise add its full timeout and a
/// DHT try to the short direct attempt.
pub const MAILBOX_PEER_RECORD_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);
```

`record_of` becomes:

```rust
let stored = match peer_records { Some(s) => s.recall(service_id).await, None => None };
let lookup = registry_client.lookup(service_id, false);
let looked_up = if stored.is_some() {
    time::timeout(MAILBOX_PEER_RECORD_LOOKUP_TIMEOUT, lookup)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("registry lookup of '{service_id}' timed out")))
} else {
    lookup.await
};
match looked_up {
    Ok(record) => { /* remember, as today */ Ok(record) }
    Err(e) => stored.ok_or(e),
}
```

`recall` returns only a record that names a mailbox and still verifies
(`peer_records.rs:159`), so the bound applies only to peers known to have a
mailbox. Test in `net_iroh/tests.rs`: with a stored record and a registry
that never answers, `resolve_peer` returns within about 3 s and gives the
stored mailbox. (Use a local TCP listener that accepts and never answers;
its port comes from binding `127.0.0.1:0`, never a literal.)

#### 4.4 New module `crates/router/src/proxy/mailbox.rs`

Add `mod mailbox;` in `proxy.rs`. This file holds the sender path. If it
passes about 450 lines, put the poll half in `proxy/mailbox/poll.rs`
(`proxy/mailbox.rs` beside `proxy/mailbox/`).

```rust
use syneroym_core::{
    dht_registry::{EndpointMailbox, MasterAnchorResolver},
    mailbox::{self, StatusAnswer, StatusParams, codes, MAX_STATUS_BATCH, MAILBOX_INTERFACE},
};
use syneroym_mailbox_format::{
    MAX_CLOCK_SKEW_SECS, MessageContent, OPEN_MAX_TTL_SECS, Receipt, Recipient, SealError,
    Sender, SentDeposit, SignedOutcome, seal,
};
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, NOT_HOSTED_HERE_RPC_CODE, SERVICE_NOT_FOUND_RPC_CODE,
    STREAM_UNAUTHENTICATED_RPC_CODE,
};

use crate::net_iroh::ResolvedPeer; // and `super::RemotePeer` from proxy/hop.rs

/// One direct attempt at a receiver that has a mailbox. Short: if it fails,
/// the message waits at the mailbox instead.
const MAILBOX_DIRECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// Every call to a mailbox (`deposit`, `status`). Well under the outbox
/// visibility timeout, so a slow mailbox never lets a second claim start.
const MAILBOX_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a sender asks about a deposit. The receiver pulls about this
/// often, so asking more often learns nothing new.
const DEPOSIT_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Time after a deposit's time-to-live before a deposit that still reads
/// `stored` is treated as lost. The mailbox sweeps expired messages on its
/// own schedule.
const DEPOSIT_DEADLINE_GRACE_MS: i64 = 3_600_000;
/// Reading the receiver master's anchor before a receipt is accepted.
const RECEIVER_ANCHOR_TIMEOUT: Duration = Duration::from_secs(5);

/// What the sender keeps in the parked row.
#[derive(Debug, Serialize, Deserialize)]
struct DepositContext {
    mailbox: String,
    sent: SentDeposit,
}

/// The sending instance: what it seals with and what it shows the mailbox.
struct DepositSender {
    service_id: String,
    key: Arc<Identity>,
    certificate: DelegationCertificate,
}
```

Functions (all `impl ProxyRouter`):

```rust
/// The sending service of `req`, when `req` may use a mailbox at all.
fn deposit_sender(&self, req: &ProxyRequest) -> Option<DepositSender> {
    req.idempotency_key.as_ref()?;
    // A call made for a person passes that person's proof on; a sealed
    // message cannot carry it, so the receiver would lose the person.
    if req.caller.proof.is_some() { return None; }
    let service_id = match &req.origin {
        CallOrigin::Guest { service_id } => service_id,
        CallOrigin::Native { service_id: Some(id) } => id,
        CallOrigin::Native { service_id: None } => return None,
    };
    self.sender_identity(service_id)
}

/// Also used by `poll_deposits`, where only the service id is known. The
/// same key and certificate a remote call of this service presents
/// (`instance_identity`), so the seal and the transport always agree.
fn sender_identity(&self, service_id: &str) -> Option<DepositSender> {
    let identity = self.instance_identity(service_id)?;
    let certificate = identity.delegation?;
    Some(DepositSender { service_id: service_id.to_string(), key: identity.key, certificate })
}

/// The caller every mailbox call carries. A same-node mailbox reads the
/// proof from this context, because there is no stream to sign. On a
/// remote call the proof is unsigned, so it is never passed on; the hop
/// presents the instance identity of the origin and signs it.
fn instance_caller(sender: &DepositSender) -> CallerContext {
    let mut caller = CallerContext::verified(
        &sender.certificate.master_did,
        AuthLevel::Delegated,
        Vec::new(),
    );
    caller.proof = Some(CallerProof {
        pubkey_hex: hex::encode(sender.key.public_key().to_bytes()),
        delegation_json: sender.certificate.to_json().ok(),
        ..Default::default() // signed: None, on_behalf: None
    });
    caller
}

fn mailbox_request(sender: &DepositSender, mailbox: &str, method: &str, params: Value) -> ProxyRequest {
    ProxyRequest {
        target_service: mailbox.to_string(),
        interface: MAILBOX_INTERFACE.to_string(),
        method: method.to_string(),
        params,
        caller: Self::instance_caller(sender),
        origin: CallOrigin::Native { service_id: Some(sender.service_id.clone()) },
        protocol: ProxyProtocol::JsonRpcV1,
        idempotent: false,
        idempotency_key: None,
        timeout: Some(MAILBOX_CALL_TIMEOUT),
    }
}

/// Only an error that says "not delivered, and nothing ran" is worth a
/// deposit: the receiver could not be reached, or it refused the stream
/// before any dispatch. Anything else is an answer, and the outbox handles
/// it as today.
fn worth_a_deposit(error: &ProxyError) -> bool {
    match error {
        ProxyError::Transport(_)
        | ProxyError::Timeout(_)
        | ProxyError::ServiceNotFound(_)
        | ProxyError::HostMismatch(_) => true,
        ProxyError::Callee { code, .. } => matches!(
            *code,
            SERVICE_NOT_FOUND_RPC_CODE | NOT_HOSTED_HERE_RPC_CODE | STREAM_UNAUTHENTICATED_RPC_CODE
        ),
        ProxyError::UnsupportedProtocol(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::PermissionDenied(_)
        | ProxyError::Internal(_) => false,
    }
}
```

The `match` names every variant, so a new `ProxyError` variant is a compile
error here instead of silently "not worth a deposit".

`deliver_or_deposit_inner` (pseudo-code; keep each branch short, move bodies
to helpers):

```rust
pub(super) async fn deliver_or_deposit_inner(
    &self,
    req: &ProxyRequest,
    max_wait_secs: u64,
) -> Result<Delivery, ProxyError> {
    // The same gates as invoke_inner, in the same order.
    if req.protocol != ProxyProtocol::JsonRpcV1 { return Err(UnsupportedProtocol(..)); }
    self.check_native_capability_gate(req)?;
    metrics::counter!("substrate.proxy.calls").increment(1);

    // A receiver on this node is never asleep.
    if let Some((endpoint, iface)) = self.registry.lookup(&req.target_service, &req.interface) {
        return self.invoke_local_guarded(req, endpoint, iface).await.map(Delivery::Direct);
    }
    let peer = net_iroh::resolve_peer(&self.registry_client, self.peer_records.as_deref(), &req.target_service)
        .await
        .map_err(|_| ProxyError::ServiceNotFound(req.target_service.clone()))?;

    self.deliver_to_peer(req, peer, max_wait_secs).await
}

/// Split out so tests can pass a synthetic `ResolvedPeer`.
pub(super) async fn deliver_to_peer(
    &self,
    req: &ProxyRequest,
    peer: ResolvedPeer,
    max_wait_secs: u64,
) -> Result<Delivery, ProxyError> {
    let remote = peer.addr.map(|addr| RemotePeer { addr, host_key: peer.host_key });

    // No mailbox, or a call that may not use one: today's path, with its
    // retry loop and its re-homing.
    let (Some(mailbox), Some(sender)) = (peer.mailbox.as_ref(), self.deposit_sender(req)) else {
        let remote = remote.ok_or_else(|| ProxyError::ServiceNotFound(req.target_service.clone()))?;
        return self.invoke_remote_resolved(remote, req).await.map(Delivery::Direct);
    };

    // One direct attempt: one call, no retry, no re-home.
    let direct_error = match &remote {
        Some(remote) => match self.invoke_remote_once(remote, req, MAILBOX_DIRECT_ATTEMPT_TIMEOUT).await {
            Ok(value) => return Ok(Delivery::Direct(value)),
            Err(e) if !worth_a_deposit(&e) => return Err(e),
            Err(e) => e,
        },
        None => ProxyError::ServiceNotFound(req.target_service.clone()),
    };

    let ttl_secs = OPEN_MAX_TTL_SECS.min(max_wait_secs).max(1);
    match self.deposit(req, &sender, mailbox, ttl_secs).await {
        Ok(ticket) => { metrics::counter!("substrate.proxy.mailbox.deposited").increment(1); Ok(Delivery::Deposited(ticket)) }
        Err(why) => {
            metrics::counter!("substrate.proxy.mailbox.deposit_failed").increment(1);
            warn!(target = %req.target_service, error = %why, "could not deposit at the receiver's mailbox; will try directly again");
            Err(direct_error)
        }
    }
}
```

Note: do not record metrics or a duration histogram twice; the delivery
outcome metrics stay in the outboxes.

`deposit`:

```rust
/// Seals `req` for its receiver and hands it to the receiver's mailbox.
/// Each call seals again, with a new deposit id and send time.
async fn deposit(
    &self,
    req: &ProxyRequest,
    sender: &DepositSender,
    mailbox: &EndpointMailbox,
    mut ttl_secs: u64,
) -> Result<DepositTicket, DepositFailure> {
    let content = MessageContent {
        interface: req.interface.clone(),
        method: req.method.clone(),
        params: req.params.clone(),
        idempotency_key: req.idempotency_key.clone().unwrap_or_default(), // Some by deposit_sender
    };
    let recipient = Recipient { service_id: &req.target_service, mailbox_key: &mailbox.key };
    let seal_as = Sender { instance: &sender.key, certificate: &sender.certificate };
    // At most two tries: the second only for a mailbox that allows a
    // shorter time-to-live than asked.
    for _ in 0..2 {
        let now = now_secs();
        let sealed = seal(&content, recipient, seal_as, ttl_secs, now).map_err(DepositFailure::Seal)?;
        let params = serde_json::to_value(&sealed.deposit).map_err(|e| DepositFailure::Encode(e.to_string()))?;
        let call = Self::mailbox_request(sender, &mailbox.service_id, mailbox::METHOD_DEPOSIT, params);
        match self.invoke_inner(&call).await {
            // Any answer means the mailbox took it; `status` tells the rest.
            Ok(_) => return Ok(self.ticket(&mailbox.service_id, sealed.sent, ttl_secs)?),
            Err(ProxyError::Callee { code, data, .. }) if code == codes::TTL_TOO_LONG => {
                ttl_secs = shorter_ttl(data.as_ref(), ttl_secs).ok_or(DepositFailure::Refused(code))?;
            }
            Err(e) => return Err(DepositFailure::Call(e)),
        }
    }
    Err(DepositFailure::Refused(codes::TTL_TOO_LONG))
}

/// `data.max_ttl_secs` of a `ttl_too_long` answer, when it is above zero
/// and shorter than what was asked.
fn shorter_ttl(data: Option<&Value>, asked: u64) -> Option<u64>;

fn ticket(&self, mailbox: &str, sent: SentDeposit, ttl_secs: u64) -> Result<DepositTicket, DepositFailure> {
    let deadline_at_ms = (sent.sent_at_secs.saturating_add(ttl_secs) as i64)
        .saturating_mul(1000)
        .saturating_add(DEPOSIT_DEADLINE_GRACE_MS);
    let context = serde_json::to_vec(&DepositContext { mailbox: mailbox.to_string(), sent })
        .map_err(|e| DepositFailure::Encode(e.to_string()))?;
    Ok(DepositTicket { context, poll_interval: DEPOSIT_POLL_INTERVAL, deadline_at_ms })
}

#[derive(Debug, thiserror::Error)]
enum DepositFailure {
    #[error("cannot seal: {0}")] Seal(SealError),
    #[error("cannot encode: {0}")] Encode(String),
    #[error("mailbox call failed: {0}")] Call(ProxyError),
    #[error("mailbox refused with code {0}")] Refused(i32),
}
```

`invoke_inner` is safe to reuse for the mailbox call: the mailbox target is
a node DID with interface `mailbox`, so it resolves locally
(`NativeHostChannel`) or as a substrate record, never through this deposit
path again. The mailbox call carries no idempotency key, so it skips the
fence (the mailbox deduplicates on `(sender, deposit_id)` itself,
`D-C4-20`).

The `park_context` limit is 4096 bytes (`MAX_PARK_CONTEXT_BYTES`). A
`DepositContext` is about 350 bytes. Add a unit test that a context built
from the longest DIDs stays under 1024 bytes.

#### 4.5 `poll_deposits_inner`

```rust
pub(super) async fn poll_deposits_inner(
    &self,
    sender_service_id: &str,
    contexts: &[Vec<u8>],
) -> Vec<DepositStatus> {
    let mut out = vec![DepositStatus::Waiting; contexts.len()];
    // A sender with no valid certificate cannot ask now; the supervisor
    // renews it, and the deadline bounds the wait.
    let Some(sender) = self.sender_identity(sender_service_id) else { return out };

    // Group by mailbox, keeping each entry's index.
    let mut by_mailbox: BTreeMap<String, Vec<(usize, SentDeposit)>> = BTreeMap::new();
    for (i, bytes) in contexts.iter().enumerate() {
        match serde_json::from_slice::<DepositContext>(bytes) {
            Ok(ctx) => by_mailbox.entry(ctx.mailbox).or_default().push((i, ctx.sent)),
            Err(_) => out[i] = retry("the stored deposit context is unreadable"),
        }
    }
    for (mailbox, entries) in by_mailbox {
        for chunk in entries.chunks(MAX_STATUS_BATCH) {
            self.poll_chunk(&sender, &mailbox, chunk, &mut out).await;
        }
    }
    out
}

async fn poll_chunk(&self, sender, mailbox, chunk: &[(usize, SentDeposit)], out: &mut [DepositStatus]) {
    let params = StatusParams { deposit_ids: chunk.iter().map(|(_, s)| s.deposit_id).collect() };
    let call = Self::mailbox_request(sender, mailbox, mailbox::METHOD_STATUS, json(params));
    let answer = match self.invoke_inner(&call).await {
        Ok(value) => value,
        // The mailbox is down or refused us now: ask again later.
        Err(e) => { debug!(...); return; }
    };
    let Ok(answer) = serde_json::from_value::<StatusAnswer>(answer) else {
        for (i, _) in chunk { out[*i] = retry("the mailbox status answer does not parse"); }
        return;
    };
    let receipts: HashMap<DepositId, Receipt> = answer.receipts.into_iter().map(|e| (e.deposit_id, e.receipt)).collect();
    let now = now_secs();
    for (i, sent) in chunk {
        out[*i] = self.judge(receipts.get(&sent.deposit_id), sent, now).await;
    }
}
```

#### 4.6 `judge`: one receipt to one status (ADR-0026 A1)

```rust
async fn judge(&self, receipt: Option<&Receipt>, sent: &SentDeposit, now: u64) -> DepositStatus {
    let receipt = match receipt {
        None => return retry("the mailbox has no receipt for this deposit"),
        Some(Receipt::Stored { .. }) => return DepositStatus::Waiting,
        Some(r) => r,
    };
    // Only a receipt the receiver signed, for exactly this message, ends it.
    let outcome = match receipt.verify_signed(sent, MAX_CLOCK_SKEW_SECS, now) {
        Ok(outcome) => outcome,
        Err(e) => return retry(&format!("receipt not accepted: {e}")),
    };
    match self.receiver_key_revoked(receipt, sent).await {
        Ok(false) => {}
        Ok(true) => return retry("the receiver key that signed the receipt is revoked"),
        Err(_) => return DepositStatus::Waiting,
    }
    match outcome {
        SignedOutcome::Delivered => DepositStatus::Delivered,
        SignedOutcome::Rejected(code) => DepositStatus::Rejected { code: code.as_str().to_string() },
    }
}

/// Whether the instance key that signed `receipt` is in the receiver
/// master's revoked keys. `verify_signed` checks only the receipt time
/// against the certificate window.
async fn receiver_key_revoked(&self, receipt: &Receipt, sent: &SentDeposit) -> anyhow::Result<bool> {
    let proof = match receipt {
        Receipt::Delivered { proof, .. } | Receipt::Rejected { proof: Some(proof), .. } => proof,
        _ => return Ok(false), // unreachable after verify_signed
    };
    let anchor = time::timeout(RECEIVER_ANCHOR_TIMEOUT,
        self.anchor_resolver.resolve_master_anchor(&sent.receiver_service_id)).await??;
    Ok(anchor.revoked_keys.contains(&proof.receiver_certificate.temporary_did))
}
```

`Expired`, `Unknown` and an unsigned `Rejected` fail `verify_signed` with
`Unsigned` or `NotSignable`, so they become `Retry`, as A1 requires.

Metrics: `substrate.proxy.mailbox.polled` with a `result` label
(`waiting`, `delivered`, `rejected`, `retry`).

#### 4.7 Wiring

- `ProxyRouter` gets a field `anchor_resolver: Arc<dyn MasterAnchorResolver>`,
  set in `new` to `registry_client.clone()` (an `Arc<RegistryClient>`
  coerces), and `#[must_use] pub fn with_anchor_resolver(mut self,
  resolver: Arc<dyn MasterAnchorResolver>) -> Self` for tests. This adds no
  constructor argument, so no other call site changes.
- `impl ServiceProxy for ProxyRouter` (`proxy/router.rs:597`) adds:

  ```rust
  async fn deliver_or_deposit(&self, request: ProxyRequest, max_wait_secs: u64) -> Result<Delivery, ProxyError> {
      self.deliver_or_deposit_inner(&request, max_wait_secs).await
  }
  async fn poll_deposits(&self, sender_service_id: &str, contexts: Vec<Vec<u8>>) -> Vec<DepositStatus> {
      self.poll_deposits_inner(sender_service_id, &contexts).await
  }
  ```

  No dead letter is written here: both callers are outboxes, which own
  their failure history (the same reason `deliver_queued` calls
  `invoke_inner`).

#### 4.8 Router unit tests: `crates/router/src/proxy/tests/mailbox.rs`

Add `crates/router` dev-dependency `syneroym-mailbox` (no cycle: it depends
only on core, identity, mailbox-format, passport and rpc). Register a real
`MailboxService::open(..)` in the native dispatch table under the node DID
on interface `mailbox` (look at how `RecordingNativeService` is registered
in `helpers.rs:396`), with an anchor resolver double. Add a helper
`mailbox_node()` in `helpers.rs` (or a new `tests/mailbox_helpers.rs` if
`helpers.rs`, 677 lines, would pass 800). The tests drive `deliver_to_peer`
(Step 4.4) through a `MockHop` for the receiver and a synthetic
`ResolvedPeer` (set `host_key: None`; the `MockHop` does no handshake).
`MockHop` already records every preamble it was asked to send, with the
identity applied and signed (`all_preambles`), and has
`MockOutcome::HostMismatch` (`helpers.rs:444-449`); the identity tests read
those preambles, as `proxy/tests/on_behalf.rs` does.

| Test | Checks |
| --- | --- |
| `a_receiver_that_answers_directly_is_never_deposited` | `MockHop` success → `Direct`; mailbox has no deposit. |
| `an_unreachable_receiver_is_deposited_once_with_one_direct_attempt` | `MockHop` transport error → `Deposited`; `hop.call_count() == 1`; the mailbox `status` (as the sender) answers `stored`. |
| `a_receiver_with_no_address_is_deposited_without_a_direct_attempt` | `peer.addr = None` → `Deposited`, zero hop calls. |
| `a_refusal_before_dispatch_is_deposited` | one case each for `Callee -32097`, `Callee -32096` and `MockOutcome::HostMismatch` → `Deposited`, `hop.call_count() == 1` (no retry, no re-home: Q15, Q16). |
| `a_callee_answer_is_returned_and_not_deposited` | `MockHop` callee `-32010` → `Err(Callee)`; no deposit. |
| `the_direct_attempt_presents_the_sender_instance_and_no_on_behalf` | the preamble the hop got has `pubkey` = the instance key, `delegation` = the instance certificate, a `sig`, and no `on_behalf`. |
| `a_call_with_a_caller_proof_is_never_deposited` | `req.caller.proof = Some(..)` (signed) → today's path, and its preamble carries the proof in `on_behalf`. |
| `a_call_without_a_key_is_never_deposited` | `idempotency_key = None` → today's path. |
| `a_sender_with_an_expired_certificate_is_never_deposited` | expired cert → today's path. |
| `a_receiver_without_a_mailbox_keeps_todays_path` | `peer.mailbox = None` → `invoke_remote_resolved`: the retry loop (call count equals `max_attempts`), and on `-32096` one fresh lookup (with no registry it ends as `ServiceNotFound`), as `rehome.rs` checks for `invoke_remote`. |
| `a_remote_mailbox_call_is_signed_by_the_sender_instance_and_carries_no_on_behalf` | build the request with `mailbox_request(..)` and send it with `invoke_remote_once` to a `MockHop` (a unit test has no registry to resolve the mailbox node, and this is the same hop `invoke_remote` uses): the preamble has the instance `pubkey`, `delegation` and `sig`, and no `on_behalf`, although `caller.proof` is set. The end-to-end version is in Step 9. |
| `a_message_too_large_to_seal_stays_on_direct_delivery` | params over 64 KiB → `Err(direct error)`. |
| `a_ttl_the_mailbox_refuses_is_sealed_again_shorter` | mailbox role with `max_ttl_secs` below the asked TTL → one retry, `Deposited`. |
| `a_mailbox_that_cannot_be_reached_returns_the_direct_error` | no mailbox registered locally and no registry → `Err(Transport ..)` or `ServiceNotFound`, as the direct attempt said. |
| `a_same_node_deposit_carries_the_instance_proof` | the local `invoke_local` path reaches `MailboxService` and it accepts (`D-C4` forward note). |
| `poll_waits_while_the_deposit_is_stored` | `Waiting`. |
| `poll_completes_on_a_signed_delivery` | ack the deposit as the receiver (open with the receiver's `MailboxSecretKey`, `OpenedMessage::sign_delivered`, mailbox `ack`) → `Delivered`. |
| `poll_fails_on_a_signed_not_eligible_rejection` | → `Rejected { code: "not_eligible" }`. |
| `poll_retries_on_an_unsigned_rejection_and_on_expiry` | `Retry` for both. |
| `poll_retries_when_the_receiver_key_is_revoked` | anchor double lists the receiver instance DID → `Retry`. |
| `poll_waits_when_the_receiver_anchor_cannot_be_read` | anchor double errors → `Waiting`. |
| `poll_retries_on_a_receipt_for_another_message` | a receipt signed for a different hash → `Retry`. |
| `poll_waits_when_the_mailbox_is_unreachable` | no mailbox → all `Waiting`. |
| `poll_retries_on_an_unreadable_context` | garbage bytes → `Retry`. |
| `poll_splits_more_than_one_hundred_ids_into_batches` | 101 contexts → two `status` calls (count with a recording mailbox double), all statuses in order. |
| `a_deposit_context_stays_small` | under 1024 bytes. |

### Step 5 — Proxy outbox

File `crates/router/src/proxy/outbox_forwarding.rs` (215 lines) and
`proxy_outbox.rs`.

#### 5.1 The age limit and the wait

`proxy_outbox.rs`:

```rust
/// How long a queued call may wait in all, as Unix ms since it was written.
/// Only a call to a receiver with a mailbox can come near it: the attempt
/// budget ends any other call in about ten hours.
pub(crate) const MAX_ROW_AGE_MS: i64 = (MAILBOX_SENDER_MAX_AGE_SECS as i64) * 1000;

/// What is left of a row's age limit, in whole seconds.
pub(crate) fn max_wait_secs(created_at_ms: i64, now_ms: i64) -> u64;
```

#### 5.2 `deliver_queued` returns a `Delivery`

```rust
pub(super) async fn deliver_queued(&self, call: &QueuedCall, target: String, max_wait_secs: u64)
    -> Result<Delivery, ProxyError>
{
    self.deliver_or_deposit_inner(&self.request_from(call, target), max_wait_secs).await
}
```

`enqueue_call`'s probe keeps `invoke_inner` (C5-15). It is the only other
caller of `request_from`.

#### 5.3 `drain_one_outbox`: the pending pass

Split the per-item body into `async fn settle_pending(&self, queue, item,
now)` (the function is already long). Changes inside it:

- Before the `claim_count` check, add: if `now - item.created_at >
  MAX_ROW_AGE_MS` → `fail(.., "the receiver never took this call", true)`,
  count the dead letter, continue.
- `deliver_queued(&call, target, max_wait_secs(item.created_at, now))`:

  ```rust
  match outcome {
      Ok(Delivery::Direct(_)) => complete (as today)
      Ok(Delivery::Deposited(ticket)) => {
          let deadline = ticket.deadline_at_ms.min(item.created_at + MAX_ROW_AGE_MS).max(now + 1);
          queue.park(item.id, now, ticket.poll_interval.as_millis() as u64, deadline, &ticket.context)
          // on Err: warn; leave the row (it comes back after the visibility
          // timeout and is tried again; a second deposit is safe: the receiver
          // fence runs the call once)
      }
      Err(e) => disposition_of(&e) as today
  }
  ```

#### 5.4 `drain_parked_outbox`: the parked pass

Call it from `drain_outboxes_once` for each service, after
`drain_one_outbox`.

```rust
async fn drain_parked_outbox(&self, queue: &Queue, service_id: &str, still_deployed: bool) -> usize {
    let now = now_ms();
    let items = spawn_blocking(claim_parked_due(now, CLAIM_LIMIT_PER_TICK));
    if !still_deployed { complete each; return items.len(); }
    let (live, contexts): (Vec<QueueItem>, Vec<Vec<u8>>) = ...; // item.parked is Some for every one
    let statuses = self.poll_deposits_inner(service_id, &contexts).await;
    for (item, status) in live.into_iter().zip(statuses) {
        self.settle_parked(queue, item, status, now).await;
    }
}

async fn settle_parked(&self, queue: &Queue, item: QueueItem, status: DepositStatus, now: i64) {
    let parked = item.parked.as_ref().expect("claim_parked_due returns parked items");  // use `let Some .. else` in real code
    let too_old = now - item.created_at > MAX_ROW_AGE_MS;
    match status {
        DepositStatus::Delivered => complete + "delivered" metric,
        DepositStatus::Rejected { code } => fail(id, now, &format!("the receiver refused the call: {code}"), true) + dead-letter metric,
        _ if too_old => fail(id, now, "the receiver never took this call", true),
        DepositStatus::Retry { reason } => { debug!(reason); unpark(id, now) }
        DepositStatus::Waiting if parked.is_past_deadline(now) => unpark(id, now),
        DepositStatus::Waiting => park(id, now, parked.poll_interval_ms, parked.deadline_at, &parked.context),
    }
}
```

Never call non-terminal `fail` on a parked row (Q11). Each arm is one call
to a small helper so the `match` stays a table.

#### 5.5 Operator view

`QueuedCallInfo` (`crates/rpc/src/proxy.rs:195`) gets `pub deposited:
bool`, set from `item.parked.is_some()` in `ProxyOutbox::queued_calls`.
This is S-6 for the proxy outbox. The type crosses a WIT boundary: the
record `proxy-queued-call` in
`crates/wit_interfaces/wit/control-plane/control-plane.wit` (about line
365) gets `deposited: bool` too. Then follow every user: the control-plane
conversion (`crates/control_plane/src/service/orchestration.rs:246` and
wherever the WIT record is built from `QueuedCallInfo`; grep
`proxy-queued-call` / `ProxyQueuedCall`), the SDK client
(`crates/sdk/src/client/orchestrator.rs:279`), and the `roymctl` command
that prints the outbox (grep `proxy_outbox` in `apps/roymctl`). Update
their tests.

#### 5.6 Tests (`crates/router/src/proxy/tests/outbox.rs`, 384 lines, or a new `proxy/tests/outbox_mailbox.rs`)

- A row to an unreachable receiver with a mailbox is parked after one
  worker pass, and `queued_calls` shows `deposited: true` (S-3, S-6).
- A parked row is past its attempt budget's time and still parked (S-9):
  use `max_attempts = 2` and run several passes.
- Delivered → row gone, no dead letter. Rejected → one dead letter with the
  code. Retry → pending again, the next pass makes a direct attempt.
- Waiting past the deadline → pending again.
- A row older than 30 days → dead letter, in both passes (set
  `created_at` by writing the row with a `now` 31 days ago).
- A row to a receiver without a mailbox behaves as before (the existing
  tests stay green; add one that asserts no deposit was made).

### Step 6 — Conversation: the state value everywhere

#### 6.1 Rust enum and its users

`crates/rpc/src/conversation.rs`:

```rust
pub enum ConversationDeliveryState {
    Pending,
    /// Waiting at the receiver's mailbox. Not a final success: only
    /// `Delivered` is (ADR-0026).
    Deposited,
    Delivered,
    Failed,
}
```

Then make every `match` compile. The full list (grep
`ConversationDeliveryState::` and `DeliveryState::`, excluding generated
`bindings.rs`):

- `crates/conversation/src/store.rs:295-308`: `state_str` →
  `"deposited"`; `state_from_str` → `"deposited" => Deposited`.
- `crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs:49`
  (`map_state`), `crates/sandbox_wasm/src/engine/auth.rs:190`
  (`conversation_state_str` → `"deposited"`).
- `crates/app_host_native/src/convert/conversation.rs:21-23, 155-157`
  (both directions).
- `crates/app_host` (`types.rs`, `lib.rs`, `guest.rs`) re-export the WIT
  type; they change with the WIT (6.2).
- `crates/roym_core/src/conversation.rs:44-70`: `StoredState::Deposited`
  (serialises as `"deposited"`), both `From` impls, and the round-trip
  test at line 248.
- `test-components/dual-build-fixture/src/{app.rs,native.rs,guest.rs,
  app/callbacks.rs}`: add the arm.

#### 6.2 WIT

`crates/wit_interfaces/wit/conversation/conversation.wit:25`:

```wit
/// `deposited`: the receiver was not reachable, and a sealed copy waits at
/// its mailbox. Not a final success.
enum delivery-state { pending, delivered, failed, deposited }
```

Append the case at the end so the three old cases keep their numbers.
Then rebuild the generated bindings and every component that imports the
interface: `mise run build:roym` and `mise run build:test-components`
(`dual_build_parity` loads the pre-built wasm, so a stale build fails it).
Commit the regenerated `bindings.rs` files.

#### 6.3 Hub UI (`crates/roym_web/ui`)

- `src/screens/messages.ts:52`: the union gets `"deposited"`. The state
  word stays the raw value (the comment at line 474 says so); add a
  `title` on the state span for `deposited`: "The other side is offline.
  The message waits in their mailbox." No retry button for `deposited`.
- `src/screens/groups.ts:71` and `src/groups/words.ts:41`: no change. A
  group message never has the message-level state `deposited` (C5-11).
- Tests: `src/screens/messages.test.ts`: a `deposited` message shows the
  word `deposited`, the title, and no retry button.
- Rebuild the UI bundle the way `build:roym` / `test:roym-ui` expect (see
  `mise.toml`).

### Step 7 — Conversation store: `deposited` in SQL and in transactions

#### 7.1 Every `'pending'` literal (checked list)

| Place | Change |
| --- | --- |
| `store/message.rs:75` pending quota | `state IN ('pending', 'deposited')`: a deposited message still holds a payload. |
| `store/message.rs:405-420` delete before delivery | `state == "pending" \|\| state == "deposited"` for the message; recipients `state IN ('pending', 'deposited')`. The copy at the mailbox cannot be taken back; the parked row is completed by the worker because the message is no longer `deposited`. Backlog row (Step 11). |
| `store/message.rs:643` `outbox_messages` | `IN ('pending', 'deposited', 'failed')`. |
| `store/message.rs:672` `recipients_remaining` | `IN ('pending', 'deposited')`. |
| `group.rs:380` group pending quota | no change: a group message stays `pending` (C5-11). |
| `group.rs:406`, `group.rs:435`, `store/message.rs:98` inserts | no change. |
| `store/backup.rs:368` restore check | accept `"deposited"`. |
| `store/backup.rs:388` restore mapping | `m.outgoing && (m.state == "pending" \|\| m.state == "deposited")` → `failed`, "restored from a backup". |

#### 7.2 Writes through the transaction (`D-C1-5`)

New file `crates/conversation/src/store/deposit.rs` (`mod deposit;` in
`store.rs`). `store/message.rs` is 729 lines, so the new code does not go
there.

```rust
impl ConversationStore {
    /// Marks the message (1:1) or one member's copy (group) `deposited` and
    /// parks its outbox row, in one commit. The worker deletes any row
    /// whose message state it does not expect, so the two must never be
    /// seen apart.
    pub(crate) fn park_deposited(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,   // Some(peer) for a group
        now: i64,
        poll_interval_ms: u64,
        deadline_at: i64,
        context: &[u8],
    ) -> anyhow::Result<()> {
        self.queue().transaction(|tx, txq| {
            set_state_in(tx, message_id, member, ConversationDeliveryState::Deposited, None)?;
            txq.park(tx, item_id, now, poll_interval_ms, deadline_at, context)
        })
    }

    /// The other way: back to `pending`, and the row back to direct attempts.
    pub(crate) fn unpark_to_pending(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,
        now: i64,
    ) -> anyhow::Result<()> {
        self.queue().transaction(|tx, txq| {
            set_state_in(tx, message_id, member, ConversationDeliveryState::Pending, None)?;
            txq.unpark(tx, item_id, now)
        })
    }
}

/// Through `tx`. Never call `set_state` or `set_recipient_state` inside a
/// queue transaction: both lock the connection the transaction holds.
fn set_state_in(tx: &Transaction<'_>, message_id: &str, member: Option<&str>, state, last_error) -> rusqlite::Result<()> {
    match member {
        None => tx.execute("UPDATE messages SET state = ?1, last_error = ?2 WHERE id = ?3", ..),
        Some(m) => tx.execute("UPDATE message_recipients SET state = ?1, last_error = ?2 WHERE message_id = ?3 AND member_address = ?4", ..),
    }.map(|_| ())
}
```

Also a read: `pub(crate) fn recipient_state(&self, message_id, member) ->
Result<Option<ConversationDeliveryState>>` (the parked pass must check that
a member's copy is still `deposited`).

Tests in `store/tests/`: `park_deposited` sets the state and parks in one
commit (a failing park leaves the state unchanged: make `park` fail with a
`deadline_at <= now`); `unpark_to_pending` the reverse; the quota counts a
deposited message; `recipients_remaining` counts a deposited member; a
deleted deposited message is `failed`; backup accepts and maps
`deposited`.

### Step 8 — Conversation outbox: deposit and poll

#### 8.1 Transport (`transport.rs`, `transport/group_sync.rs`)

A delivery now has two good endings:

```rust
/// How a delivery attempt that did not fail ended.
#[derive(Debug)]
pub(crate) enum Sent {
    Delivered,
    Deposited(DepositTicket),
}
```

- Split `call_peer` into `fn peer_request(&self, svc, peer, method,
  params, key, timeout) -> Result<ProxyRequest, Disposition>` (the identity
  check and the request) and the existing `call_peer` (unchanged for
  `prekey-bundle`, `group-sync`, `fetch_prekey_bundle`).
- New:

  ```rust
  /// For an outbox row only: the receiver's mailbox may take the message.
  pub(crate) async fn call_peer_deferrable(&self, svc, peer, method, params, key: String,
      timeout: Option<Duration>, max_wait_secs: u64) -> Result<Delivery, Disposition>
  {
      let request = self.peer_request(svc, peer, method, params, Some(key), timeout)?;
      let Some(proxy) = self.current_service_proxy().upgrade() else { return Err(Disposition::Retry) };
      proxy.deliver_or_deposit(request, max_wait_secs).await.map_err(classify)
  }
  ```

- `deliver_one(svc, peer, msg, max_wait_secs) -> Result<Sent,
  Disposition>`: use `call_peer_deferrable`; `Delivery::Deposited(t)` →
  `Ok(Sent::Deposited(t))` (no ack to read, no refusal to record: S-1
  exception); `Delivery::Direct(json)` → the existing ack handling, then
  `Ok(Sent::Delivered)`. The `Terminal` arm that forgets an unanswered
  session stays.
- `push_group_entry(store, svc, peer, entry, deferrable: Option<u64>) ->
  Result<Sent, Disposition>`: `Some(max_wait_secs)` from the outbox row
  uses `call_peer_deferrable`; `None` (the relay push, C5-16) keeps
  `call_peer` and maps its success to `Sent::Delivered`. Update the relay
  call site (`outbox.rs:130`) to pass `None` and keep ignoring the result.
- `deliver_group_one(svc, peer, msg, max_wait_secs) -> Result<Sent,
  Disposition>` passes `Some(max_wait_secs)`.

The two files are 658 and 406 lines; the change is small. If
`transport.rs` passes 800, move `call_peer`, `peer_request` and
`call_peer_deferrable` to `transport/peer_call.rs`.

#### 8.2 Pending pass (`outbox.rs::drain_item`)

- `max_wait_secs = (max_age_ms - (now - msg.received_at_ms)) / 1000`,
  computed next to the existing age check (line 196).
- `delivery_result` is now `Result<Sent, Disposition>`:

  ```rust
  match delivery_result {
      Ok(Sent::Delivered) | Err(Disposition::Delivered) => settle_delivered(..) (as today),
      Ok(Sent::Deposited(ticket)) => self.settle_deposited(svc, store, &item, &msg, is_group, &parsed.peer_address, ticket, now).await,
      Err(..) => as today,
  }
  ```

- `settle_deposited`:

  ```rust
  let age_deadline = msg.received_at_ms + max_age_ms;
  let deadline = ticket.deadline_at_ms.min(age_deadline).max(now + 1);
  let member = is_group.then_some(peer_address);
  match store.park_deposited(item.id, &msg.id, member, now, ticket.poll_interval.as_millis() as u64, deadline, &ticket.context) {
      Ok(()) => if !is_group { self.notify_state(svc, msg.id.clone(), Deposited).await },
      Err(e) => warn!(..),  // the row returns after the visibility timeout; a second deposit is safe
  }
  metrics::counter!("substrate.conversation.outbox.deposited").increment(1);
  ```

  Note: `drain_item` already skips a row whose message is not `Pending`
  (line 190). That stays right: a parked row is never returned by
  `claim_due`.

#### 8.3 Parked pass: new file `crates/conversation/src/outbox/parked.rs`

Add `mod parked;` in `outbox.rs` (the `outbox/` folder already holds
`tests.rs`). Call `self.drain_parked_once().await` in `run_worker` right
after `self.drain_once().await`.

```rust
impl ConversationService {
    pub(super) async fn drain_parked_once(&self) {
        for svc in self.candidate_service_ids() {
            let still_deployed = /* same expression as drain_once */;
            if let Ok(store) = self.store_for(&svc).await {
                self.drain_parked_one(&svc, &store, still_deployed).await;
            }
        }
    }

    async fn drain_parked_one(&self, svc: &str, store: &ConversationStore, still_deployed: bool) {
        let now = now_ms();
        let Ok(items) = store.queue().claim_parked_due(now, CLAIM_LIMIT_PER_TICK) else { return };
        // 1. Settle the rows that need no poll; keep the rest.
        let mut live = Vec::new();
        for item in items {
            if let Some(row) = self.check_parked(svc, store, still_deployed, item, now).await {
                live.push(row);
            }
        }
        if live.is_empty() { return; }
        // 2. One poll for all of them.
        let Some(proxy) = self.current_service_proxy().upgrade() else { return };
        let contexts = live.iter().map(|r| r.parked.context.clone()).collect();
        let statuses = proxy.poll_deposits(svc, contexts).await;
        // 3. Settle each.
        for (row, status) in live.into_iter().zip(statuses) {
            self.settle_parked(svc, store, row, status, now).await;
        }
    }
}

/// A claimed parked row that still waits for a receipt.
struct ParkedRow { item: QueueItem, parked: Parked, outbox: OutboxItem, msg: StoredMessage }
```

`check_parked` returns `None` after it settles the row itself:

- not deployed → `complete`;
- payload unreadable → `fail(.., true)`;
- message missing, restored, or no longer `deposited` (1:1: `msg.state`;
  group: `recipient_state(msg.id, peer)`) → delete the outbound envelope
  only when it is a 1:1 message, `complete`;
- older than `max_pending_age_secs` → `settle_failed(.., "recipient never
  became reachable", ..)` (the age limit on the parked path, `D-C1-7`).

`settle_parked`:

```rust
match status {
    DepositStatus::Delivered => self.settle_delivered(svc, store, &row.item, &row.msg, is_group, peer).await,
    DepositStatus::Rejected { code } => self.settle_failed(svc, store, &row.item,
        &format!("the receiver refused the message: {code}"), is_group, peer).await,
    DepositStatus::Retry { .. } => self.back_to_direct(svc, store, &row, now).await,
    DepositStatus::Waiting if row.parked.is_past_deadline(now) => self.back_to_direct(svc, store, &row, now).await,
    DepositStatus::Waiting => { let _ = store.queue().park(row.item.id, now, row.parked.poll_interval_ms, row.parked.deadline_at, &row.parked.context); }
}

async fn back_to_direct(&self, svc, store, row: &ParkedRow, now: i64) {
    let member = row.outbox.group.is_some().then_some(row.outbox.peer_address.as_str());
    if store.unpark_to_pending(row.item.id, &row.msg.id, member, now).is_ok() && member.is_none() {
        self.notify_state(svc, row.msg.id.clone(), ConversationDeliveryState::Pending).await;
    }
}
```

`settle_delivered` and `settle_failed` already work for a parked row: they
call `set_state` / `set_recipient_state` and `queue().complete` /
`fail(.., true)` outside any transaction, and `fail` on a parked row
dead-letters it. Check that `settle_delivered` for a group now sees a
deposited member as settled: it sets the member to `Delivered` first, then
calls `recipients_remaining` (which now also counts `deposited`).

#### 8.4 Conversation tests (`crates/conversation/src/outbox/tests.rs` or a new `outbox/tests/deposit.rs`)

Use a `ServiceProxy` test double that returns `Delivery::Deposited` and
then a scripted list of `DepositStatus` values (look at the double the
existing tests already use for `invoke`; grep `impl ServiceProxy for` under
`crates/conversation/src`).

- 1:1: a deposit sets `deposited`, notifies the app once, and parks the
  row; the next pending pass does not touch it.
- `Delivered` → `delivered`, notified, envelope removed, row gone.
- signed `Rejected` → `failed` with the code in `last_error`.
- `Retry` → `pending` again and notified; the next pending pass delivers
  directly and **reuses the stored envelope** (the ratchet does not move).
- `Waiting` past the deadline → `pending`.
- A parked row past `max_pending_age_secs` → `failed`.
- A deleted deposited message: its parked row is completed, nothing is
  polled for it.
- Group: one member deposited, one delivered: the message stays `pending`;
  when the deposited member's receipt is `Delivered`, the message becomes
  `delivered`. One member rejected → the message ends `failed`.
- The relay push never deposits (the double records the method used).

### Step 9 — Integration tests (`crates/substrate/tests/`)

Use `common::SubstrateNode` and `common::alloc_ports`; never a port
literal. The helpers in `proxy_outbox_e2e.rs` (`fast_queue_role`,
`configure_queue_node`, `deploy_guest`, `guest_enqueue`) and in
`conversation_cross_node_e2e.rs` (`Peer`) are file-local. Move the ones you
need to `tests/common/outbox.rs` and `tests/common/conversation_peer.rs`
(each a `mod` in `common/mod.rs`), and switch the old files to them in the
same change. Add `tests/common/mailbox_receiver.rs`: a test double that
plays the receiver at a mailbox with the receiver's instance identity
(register, claim, open with `MailboxSecretKey::derive_from_master`, then ack
with `sign_delivered` or `sign_rejection`); reuse the shapes from
`mailbox_service_e2e.rs` (`instance_client`, `record`).

Since C4b and C4c, every `SyneroymClient` that presents an identity signs
its preamble, and a client that dials a node other than the target's host
must know the host key: from a registry record, or from
`SyneroymClient::with_host` (`D-C4c-7`). The receiver double therefore
talks to the mailbox node with a client built like `instance_client` in
`mailbox_service_e2e.rs` (already updated for C4b/C4c), with a registry URL
or `with_host(<mailbox node did:key>)`. `signed_preamble_e2e.rs` shows both.

Each test needs a registry, a mailbox node (`[roles.mailbox]`), a sender
node, and a receiver deployed with a mailbox record (as in
`mailbox_record_e2e.rs::record_with_mailbox`), then stopped. Keep the
mailbox on its own node in every test except the same-node one, so the
deposit and `status` calls cross the wire: that is the path where the
mailbox node's router checks the sender's signed proof (SP-2).

New file `crates/substrate/tests/mailbox_sender_e2e.rs`:

| Test | Criteria |
| --- | --- |
| `a_queued_call_to_a_stopped_receiver_waits_at_its_mailbox_until_a_signed_delivery` | S-2, S-3, S-5, S-6 (proxy outbox `deposited: true`), S-9 |
| `a_signed_not_eligible_rejection_dead_letters_the_queued_call` | S-5 |
| `an_expired_deposit_goes_back_to_direct_and_lands_once_when_the_receiver_returns` | S-5, R-7 (the receiver runs the guest method once; count with the guest's own state) |
| `a_receiver_without_a_mailbox_is_never_deposited` | S-8 |
| `a_stopped_mailbox_keeps_the_call_on_direct_attempts` | S-4 |
| `a_sender_on_the_mailbox_host_deposits_through_the_local_path` | forward note from C4 (same-node `invoke_local` proof, unsigned and never passed on) |
| `a_receiver_whose_old_host_answers_not_hosted_here_gets_a_deposit` | Q15, Q16: undeploy the receiver service from its node and keep the node running; the stale record (or the sender's stored peer record, K-6) still names that node, which answers `-32096`; the call is deposited after one direct attempt. |

New file `crates/substrate/tests/mailbox_conversation_e2e.rs`:

| Test | Criteria |
| --- | --- |
| `a_message_to_a_stopped_peer_shows_deposited_then_delivered` | S-6, S-7 (1:1). First exchange one message live, so a session exists (first contact stays live, a non-goal). |
| `a_group_push_to_one_stopped_member_is_deposited_and_the_others_are_delivered` | S-7 (group) |

Run them with nextest, one file at a time, output to a file (see
AGENTS.md **Context Budget**). `cert_renewal_e2e` loops under the
sandbox; e2e and `cargo test --workspace` need the sandbox off.

### Step 10 — Docs

- `docs/planning/changes/service-mailbox/change.md`:
  - `## Tasks`: tick "`ProxyRouter` deposit and status poll. New state in
    both outboxes. New `ConversationDeliveryState` value, the WIT enum in
    `conversation.wit`, and the Hub UI." and "Receiver idempotency memory
    for services with a mailbox". Tick "Resolve the open questions" only if
    the two left (`pull-now`, values) are answered; they are not, so leave
    it.
  - Fill the "Test" column of S-1 to S-9 and R-7 with the test names.
  - `## Deviations`, `D-C5-1` onward, at least: Q2 (per-caller cap), Q3
    (stored `max_ttl_secs`), Q4 (eligibility declaration in C6, call-kind on
    the sender), Q5 (30-day proxy-outbox limit; `D-C1-4` corrected), Q6
    (`group-push` exception), Q8 (member-level group state), Q12 (two
    direct attempts), Q13 (lookup bound only with a stored record), Q14
    (the constants), Q15 (pre-dispatch refusals `-32096`, `-32097` and a
    host mismatch lead to a deposit), Q16 (the direct attempt does not
    re-home), Q17 (5 s direct attempt includes the handshake on an
    indirect connection), C5-3 (a call that carries a caller's proof stays
    direct, because a sealed message has no `on_behalf`), C5-4 (the
    unsigned same-node instance proof), C5-8 (`ttl_too_long` reseal), C5-13 (receipt
    revocation check), C5-9 (fence length `30 days + 24 h`), C5-16 (relay
    push stays live), `QueuedCallInfo.deposited`, the WIT case appended last,
    and the new registry fact (C5-2).
  - Forward note for C6 in `task.md` §3 C6: the registry fact
    `mailbox_of` exists; overwrite `max_ttl_secs` from the `register`
    answer with `registry.set_mailbox`; R-7 is done; Q7 (`group-push`
    freshness); R-5 needs the `mailbox_methods` manifest field (Q4).
- `task.md`: status line; C5 entry "Done …" with the decision range.
- `status.md`: the C5 row with evidence (test files and counts, verify
  result).
- `docs/planning/deferred-backlog.md` (§5 Messaging), new rows:
  - a deposit cannot be taken back when the message is deleted before
    delivery;
  - a `group-push` that waits more than 24 hours at a mailbox is refused at
    the receiver (until C6 decides);
  - a sender that has never seen a peer still waits for the full registry
    lookup before it can deposit;
  - a fence row for a service with a mailbox may hold a result of up to
    64 KiB for 31 days, 100,000 rows per caller (storage bound to review);
  - the deposit TTL is clamped to the open mailbox limit until C8.
- Living docs: no statement in `system-architecture.md` or
  `system-requirements-spec.md` is false after C5 alone, because the full
  mailbox close-out is planned after C7 (`task.md` C7). Check the
  messaging/offline section of `system-architecture.md` anyway, and the
  `delivery-state` description in the developer guide if it lists the
  values (grep `delivery-state` and `pending, delivered`).

### Step 11 — Finish

1. Import clean-up pass over every edited file (AGENTS.md).
2. `cargo dupes report` over the changed files (recipe in AGENTS.md); the
   two parked passes are the likely duplicate. If they are, move the shared
   `match` shape into one generic helper only if it stays readable.
3. Deferred-backlog pass (Step 10 list, plus anything new).
4. `mise run verify` in the background, without `--skip`. All gates must
   pass. Lower any ratchet the gates report as improved
   (`xtask/oversized-test-files.txt`, `MAX_TOO_MANY_LINES_SUPPRESSIONS`,
   `MAX_EXACT_DUPLICATE_PERCENT`).
5. Do not commit until the owner asks.

## 5. Call sites that change (one list)

| Symbol | Change | Call sites |
| --- | --- | --- |
| `QueueItem` | `+ created_at` | built only in `async_queue/src/queue.rs::read_item` |
| `DedupStore::begin` / `finish` | `+ window: DedupWindow` | `router/src/call_dedup.rs` (`begin`, `DedupClaim::settle`), `async_queue/src/dedup/tests.rs` |
| `EndpointStorage` | `+ load_all_mailboxes`, `save_mailbox`, `remove_mailbox` | impls: `core/src/storage.rs` (`MockStorage`), `data_db/src/registry_store.rs`, `control_plane/.../tests/helpers.rs:322`, `router/tests/service_ownership.rs:152` |
| `EndpointRegistry` | `+ set_mailbox`, `mailbox_of`, `remove_mailbox` | writers: `control_plane/.../deploy/commit.rs`, `lifecycle.rs::remove_deploy_registry_rows`; reader: `router/src/call_dedup.rs` |
| `validate_mailbox` | returns `Option<(MailboxSecretKey, String)>` | `deploy/admission.rs:113`, `tests/mailbox.rs` |
| `ValidatedDeploy` | `+ mailbox` | `deploy/admission.rs:120`, `deploy/commit.rs` |
| `ServiceProxy` | `+ deliver_or_deposit`, `poll_deposits` (default bodies) | impl in `router/src/proxy/router.rs`; callers `conversation/src/transport.rs`, `conversation/src/outbox/parked.rs` |
| `ProxyRouter::invoke_remote_at` | its `JsonRpcRequest` literal moves to `outbound_request`; new siblings `invoke_remote_once` and `invoke_remote_resolved` reuse `outbound_identity` and `invoke_rehoming`; no preamble is built or signed in the proxy | internal only |
| `ProxyError` (has `HostMismatch` since C4c) | no change; every new exhaustive `match` (`worth_a_deposit`) names all variants | `router/src/proxy/mailbox.rs` |
| `ProxyRouter::deliver_queued` | `+ max_wait_secs`, returns `Delivery` | `proxy/outbox_forwarding.rs` |
| `record_of` | bounded lookup with a stored record | `net_iroh.rs::resolve_peer` (no signature change) |
| `QueuedCallInfo` and WIT `proxy-queued-call` | `+ deposited` | `router/src/proxy_outbox.rs::queued_calls`, `control-plane.wit`, control-plane conversion, `sdk/src/client/orchestrator.rs`, `roymctl` printing |
| `ConversationDeliveryState` | `+ Deposited` | the list in Step 6.1 |
| WIT `delivery-state` | `+ deposited` (last) | regenerated `bindings.rs` in `crates/roym_*`, `crates/app_host`, test components |
| `deliver_one`, `deliver_group_one`, `push_group_entry` | return `Result<Sent, Disposition>`; `+ max_wait_secs` / `deferrable` | `conversation/src/outbox.rs::drain_item`, relay call `outbox.rs:130` |

## 6. What C5 does not do

- The receiver: register, pull, open, deliver, ack, `mailbox.pull-now`,
  R-1 to R-6 (C6). C5 tests play the receiver with a test double.
- The `mailbox_methods` manifest field and the R-5 check (C6, Q4).
- `--mailbox auto` and the registry list (C7). Dedicated mailboxes (C8).
- First contact in Conversation with an offline receiver (non-goal).
- The Conversation refusal reason on the mailbox path (non-goal; the
  sender sees `delivered` with no refusal note).

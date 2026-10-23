# M06D slice C6 — Receiver path: implementation plan

*Reader: the developer (or AI) who implements slice C6. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready, 2026-10-23. The owner accepted the recommendations for
Q1 to Q9 (section 2) on 2026-10-23. A review on the same day changed Q3
(an error answer is now an unsigned rejection, not `delivered`, so no
ADR-0026 amendment is needed) and added C6-15 to C6-18. The owner accepted
the revised Q3 on 2026-10-23. You can start Step 1 once C5 has merged (see
Base).**

**Base.** C6 builds on C5 (branch `feat/m06d-c5-sender-path`, not merged
into `main` when this plan was written; local `main` is at C4c, `3d5ad525`).
Start the C6 branch, `feat/m06d-c6-receiver-path`, from `main` after the C5
PR has merged. All C5 work is committed (the last C5 commit is
`ac1fe2d7`). File paths and line numbers below were checked against
`feat/m06d-c5-sender-path` at `50916074` and spot-checked again at
`ac1fe2d7`; the two later commits touch only `async_queue` exports and a
`call_dedup` test.

## 0. Read first, and the rules

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (background job, about 20 minutes),
  the import clean-up pass, the deferred backlog, no planning references in
  code (no "M06D", "C6", "R-5", "D-C5-3", "C6-4" in code, comments or test
  names; ADR ids are fine), simple English in comments, 800-line files,
  100-line functions, short match arms, the module layout rule, and no
  hard-coded ports in substrate tests.
  **This plan names its decisions `C6-1` to `C6-18`. Those ids are for this
  plan only. When a step says "for the reason in C6-n", write the reason
  itself in the comment.**
- `task.md` in this folder: §3 C4 (forward note), C5 (all notes), C6 (all
  notes).
- `docs/planning/changes/service-mailbox/change.md`: R-1 to R-7, Design
  "Sealed message", "Receiver flow", Open questions, `## Deviations`
  (`D-C2-4`, `D-C2-5`, `D-C2-7`, `D-C2-11`, `D-C3-3`, `D-C3-5`, `D-C3-18`,
  `D-C4-1`, `D-C4-8` to `D-C4-12`, `D-C5-2`, `D-C5-3`, `D-C5-14`,
  `D-C5-17`, `D-C5-22`).
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) Decisions
  5 and 7, amendments A1 and A2.
- [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
  Decision 2 (`CallerContext`) and Amendments 2 and 3 (`on_behalf`).
- [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §4 (the
  receiver fence).
- The deferred-backlog rows that name this slice: "A `group-push` that waits
  more than 24 hours at a mailbox is refused by the receiver", "The long
  receiver fence is bounded by a total, not by who calls", "A message that
  arrives both directly and from the mailbox runs once only if both paths
  use the same fence caller".
- The code this slice builds on (read first):
  - `crates/mailbox_format/src/message.rs` (`open`, `OpenPolicy`,
    `OpenedMessage::{sign_delivered, sign_rejection}`, `RefusedMessage`),
    `error.rs` (`OpenError::reject_code`), `receipt.rs`
    (`Receipt::unsigned_rejection`, `RejectCode::is_sender_fault`).
  - `crates/core/src/mailbox.rs` (wire types, `codes`, `HostedMailbox`),
    `crates/core/src/local_registry.rs:335-361` (`set_mailbox`,
    `mailbox_of`, `remove_mailbox`), `crates/core/src/endpoint_publisher.rs`
    (the stored record file `hosted_apps/<service_id>.json`).
  - `crates/mailbox/src/service.rs`, `store/queue.rs` (`register`,
    `admit_instance`), `store/claim.rs` (`claim`, `ack`), `record.rs`
    (`CheckedRecord::check`), `caller.rs` (`instance_of`).
  - `crates/router/src/proxy/mailbox.rs` (`sender_identity`,
    `instance_caller`, `mailbox_request`, `is_native_mailbox`),
    `proxy/mailbox/poll.rs` (`AnchorCache`, `receiver_key_revoked`),
    `proxy/router.rs:366-392` (`outbound_identity`, `instance_identity`),
    `:523` (`dispatch_call`), `route_handler/dispatch.rs:124-176`
    (`dispatch_json_rpc_once`), `:319` (`plan_pipeline`),
    `route_handler/io.rs:167-267` (`build_caller`), `call_dedup.rs`
    (`begin`, `settle`, `window_for`), `route_handler.rs:381-492` (`init`,
    `proxy`), `connection_router.rs:68-146`.
  - `crates/control_plane/src/service/orchestration/mailbox_key.rs`,
    `types.rs:262-344` (`ValidatedDeploy`, `validate_mailbox`),
    `deploy/commit.rs:1-110`, `service/dispatch.rs` (security interface),
    `service/dispatch/home_relay.rs` and `src/home_relay.rs` (the pattern
    for a substrate hook), `service.rs:285-320` (the `OnceLock` setters).
  - `crates/substrate/src/runtime/services.rs` (worker spawn, `select!`,
    `shutdown`).
  - `crates/control_plane/src/synsvc_native/conversation.rs:229-275`
    (`deliver`, `group-push` arrive on the native `conversation` interface).

### Terms used in this plan

- **Receiver:** the substrate that hosts a service whose signed record names
  a mailbox.
- **Puller:** the new receiver worker, `MailboxPuller`. One for each node.
- **Pass:** one run of the puller over every hosted service with a mailbox.
- **Stored deploy record:** the `SignedEndpointInfo` the deployer signed,
  kept in `<app_local_data_dir>/hosted_apps/<service_id>.json`
  (`ControlPlaneService::persist_or_clear_registry_cert_file`).
- **Hosted-mailbox fact:** the `HostedMailbox` row the control plane writes
  at deploy (`EndpointRegistry::set_mailbox`, table `service_mailboxes`).
- **Receiver fence:** `CallDedupGuard` (ADR-0023 §4).
- **Eligible method:** a method the receiver accepts from its mailbox
  (R-5).

## 1. What C6 builds, in one paragraph

Each substrate runs one puller. For each hosted service that has a
hosted-mailbox fact, a stored deploy record that names a mailbox, a mailbox
secret key in its vault, and a valid instance certificate, the puller
registers at the mailbox (when needed), claims messages, opens each one with
the service's mailbox key, checks the sender (certificate at send time, send
time, revocation now), checks that the method is eligible, and hands the
call to the service through the same fenced entry the wire uses, with the
original sender as the caller. It then acknowledges each message: a signed
`delivered` for a success answer, a signed `rejected` for a sender-caused
failure, an unsigned `rejected` for anything else (an error answer of the
service included), or no ack when nothing ran. It runs at start, every 60
seconds, after a certificate renewal, a KEK injection or a deploy with a
mailbox, and on the new node-owner call `security.mailbox-pull-now`. When a
service is undeployed or moves to another mailbox (or to none), the control
plane has the puller remove its old queue (`unregister`) before it removes
the service's certificate.
Services declare eligible methods in the app manifest
(`mailbox_methods = ["<interface>#<method>"]`) or with
`roymctl svc deploy --mailbox-method`. Conversation `deliver` and
`group-push` are always eligible. The receiver writes the queue's real
`max_ttl_secs` back into the hosted-mailbox fact, so the receiver fence
uses it.

## 2. Points for the owner

### 2.1 Questions (each has a recommendation)

**Answered 2026-10-23: the owner accepted every recommendation below.**
The "Recommendation" column is now the decision.

| # | Question | Recommendation | Steps |
| --- | --- | --- | --- |
| Q1 | **`mailbox.pull-now` collides with the mailbox role.** The control-plane methods are `<interface>.<method>` (`security.home-relay-set`). Interface `mailbox` on the node DID is already the native mailbox service of the `[roles.mailbox]` role (`runtime/mailbox.rs:57`). A `mailbox.pull-now` would reach the mailbox role, not the control plane. Who may call it is also open (change doc Open questions: "does the shell call it?"). | Name it **`security.mailbox-pull-now`**, node-owner only (`substrate/admin`, like every `security.*` call). It runs one pass, waits for it, and answers a `PullSummary` (counts). The phone shell is the node owner's own client. Keep the open question in the change doc for the "who wakes the substrate" part. | 5, 8, 11 |
| Q2 | **R-5 eligibility for Conversation.** The sender deposits Conversation `deliver` and `group-push` by kind of call (`D-C5-3`). They arrive on the native `conversation` interface, which no app manifest declares. If eligibility came only from `mailbox_methods`, every Conversation deposit would be rejected `not_eligible` and fail at the sender. | `conversation#deliver` and `conversation#group-push` are **always eligible** (a constant in `crates/core/src/mailbox.rs`). `mailbox_methods` lists the guest methods. An entry may not name a native-capability, node-level, `mailbox` or `supervisor` interface. | 2, 3, 5, 8 |
| Q3 | **What an error answer is acked as.** R-6 says "a temporary local failure is not acknowledged" but does not say what an error answer is (bad params, permission denied, a Conversation refusal or decrypt failure, the `group-push` 24-hour check). Some error frames are made by the host, not the service: the WASM arm turns any engine error into `-32603` (`dispatch.rs:262-272`), and native Conversation answers `internal("this node runs no conversation service")` (`conversation.rs:233`). A signed final `rejected` would let a mailbox make a sender drop a message just by delaying it: ADR-0026 A1 forbids that. | **Revised after review and accepted by the owner (2026-10-23).** A success answer (also a replayed one) is acked signed `delivered`. **Any error answer** (fresh or replayed by the fence) is acked with an **unsigned** `rejected(call_failed)`, a new unsigned reject code. The sender then goes back to direct attempts (S-5, no sender change), and its next direct attempt gets the same answer from the receiver fence, because the deferred delivery recorded it under the same `(sender master, key)`. Both outboxes classify a `Callee` error as `Terminal` (`proxy_outbox.rs:135`, `conversation/src/transport.rs:704`), so the sender ends exactly as if the call had been direct. This follows A1 rule 2 as written, so **no ADR amendment**. "Nothing ran" stays unacked (fence refused, call already running, dispatch failure, local timeout). The first proposal (ack errors as `delivered`) was dropped: it told the sender "delivered" for host-made failures and for a Conversation decrypt failure. | 1, 2, 7, 8 |
| Q4 | **Narrow the long receiver fence to eligible methods?** The backlog row "The long receiver fence is bounded by a total" names C6 and suggests "only methods that can be mailbox-eligible". | **Not in C6.** `conversation#deliver` is always eligible (Q2), and any stranger can send it with any key, so narrowing by method does not bound the attack on the services that matter most. The better bound is "only callers the service admitted", which needs the fence after admission. Re-target the row to `TBD` with this reason. `CallDedupGuard::begin` keeps its signature. | 1 |
| Q5 | **The fence-caller row.** The C5 forward note asks for two points "with a test for each": (a) a direct attempt and a mailbox delivery must present the same fence caller; (b) the fence does not move when a service moves, so a direct run on one node and a mailbox delivery on another both run. | Do (a): the mailbox delivery uses the sealed sender master as `caller_did`, the same DID a direct call with an instance certificate presents; test it (direct, then mailbox, runs once). For (b), write **no** test: it would pin wrong behaviour. Keep (b) and the "direct attempt with the node key" case in the backlog row, re-targeted `TBD`, with a `## Deviations` entry. | 1, 12 |
| Q6 | **Proxy-outbox calls to methods that did not opt in now fail fast.** A guest `enqueue` to a receiver with a mailbox is deposited after one 5 s attempt (C5). With R-5, the receiver answers a signed `not_eligible`, and the sender dead-letters the row (S-5). Before C6 such a row retried directly for about 10 hours. | Accept. It is ADR-0026 Decision 5 ("a method must declare that it accepts deferred delivery"). Write one sentence in the developer guide. | 13 |
| Q7 | **The caller a mailbox delivery presents.** `build_caller` (`io.rs:167`) also adds `substrate/admin` for the admin root, `supervisor/resolve` for the node's own DID, and UCAN capabilities. A sealed message has no UCAN, and its proof has no stream signature. | The delivery caller is `CallerContext::verified(master, Delegated, [])` plus a `CallerProof` with the instance key and certificate, `signed: None`, `on_behalf: None`. **No capabilities.** Eligible methods never include node-level interfaces, so nothing that needs them is reachable. Because the proof is unsigned, a receiving service cannot pass it on in `on_behalf` (`CallerProof::to_pass_on`). Write both as deviations. | 7, 8 |
| Q8 | **Granularity for Roym services.** Every Roym service sends all its methods through one WIT function, so an entry like `api#invoke` makes every Roym method eligible (`D-C5-3` already says so). | No action in C6. Roym uses Conversation (always eligible). Record one line in `## Deviations`. | 1 |
| Q9 | **C5 end-to-end tests use a test double as the receiver.** `mailbox_sender_e2e.rs` and `mailbox_conversation_e2e.rs` deploy the receiver on a real node, then let `common/mailbox_receiver.rs` (`MailboxReceiver`) register and claim with its own instance certificate. After C6 the real puller on that node registers too, as soon as the deploy wakes it. Its certificate is newer, so the double gets `stale_instance` (`-32046`) and those tests fail. | Move the tests that end in a delivery to the real receiver (that is what C6 is for). Keep the double only where the receiver node never runs a puller for that service. If no test uses it any more, delete `common/mailbox_receiver.rs` (`mailbox_service_e2e.rs` has its own `Member` double and does not use it). Details in Step 12.4. | 12 |

### 2.2 Stale or unclear points in `task.md` (no decision needed, but read them)

1. **"Parallel with C5"** (C6 heading) is stale: C5 is done, and the order
   is serial.
2. **"the pull waits for [the KEK] after a restart"**: there is no "KEK
   loaded" event. `ControlPlaneService::mailbox_key` fails while the vault
   is locked. The puller treats that failure as "not now" and tries again on
   the next pass (C6-4).
3. **"The pull always follows the stored deploy record"**: nothing in the
   code reads that record back except `EndpointPublisher::build_record`,
   which is private and drops an expired record. C6 needs the record even
   past `not_after` (to keep claiming). C6 adds
   `ControlPlaneService::deploy_record` (raw read, no `verify`).
4. **Forward note from C4, `-32045` "means the instance key is revoked"**:
   the mailbox also answers `-32045` for a certificate that does not verify
   now. The puller only presents a certificate that is not expired
   (`instance_identity` filters), so in practice it is "revoked, or the two
   clocks disagree". The puller handles both the same way: stop for this
   service until its certificate changes.
5. **Forward note from C4, `-32047` "for an existing queue"**: the mailbox
   checks the record (`CheckedRecord::check`) before it looks at the queue,
   so a first `register` with an expired record also answers `-32047`; then
   `claim` answers `-32040`. The puller handles this with the same rule (go
   on to `claim`; `-32040` means there is no queue).
6. **"`dispatch_json_rpc_once` with the sender's identity"**: that function
   turns everything into a response frame. A fence refusal (`-32010`,
   `-32603`) and a callee's own error with the same code look the same, so
   the puller could not tell "nothing ran" (no ack, R-6) from "ran"
   (Q3). C6 splits out the fenced part (`dispatch_fenced`) so the new entry
   point sees the difference. The wire behaviour does not change. A
   host-made `-32603` (engine error, no conversation service) still looks
   like a service answer once the fence has recorded it; Q3 handles it
   like any error answer (unsigned rejection, then the same answer on the
   direct path), so the sender never sees `delivered` for it.
7. **R-1 "a warning in the log and a metric every hour"**: which metric is
   not said. C6 increments a counter each time it writes the hourly warning
   (C6-10).
8. **The fence-caller and long-fence backlog rows** name C6 but `task.md` C6
   does not list them. See Q4 and Q5.
9. **Nothing removes a queue.** No slice calls `unregister`. After an
   undeploy, or a redeploy to another mailbox or to none, the old queue
   would live for up to 30 days, and the old mailbox would keep
   republishing the old record every hour (K-4): the service would stay
   findable and senders would keep depositing to a queue nobody pulls.
   C6 builds a best-effort `unregister` (C6-15).
10. **The change doc's integration list** ("expired certificate",
   "duplicate delivery") is left to "after C5, C6 and C7" in `task.md` C7,
   but C7 does not name the items. C6 covers these two below the
   end-to-end level (router unit and integration tests, Step 12), because
   an end-to-end test cannot wait for a certificate of at least one hour
   to expire, and cannot make a direct answer get lost. Step 1 writes this
   into `task.md` C7.

## 3. Decisions this plan makes (check them before you start)

- **C6-1 Where the code lives.** A new router module
  `crates/router/src/mailbox_pull.rs` (+ `mailbox_pull/`). It needs the
  proxy (instance identity, mailbox calls, anchors), the endpoint registry
  and the route handler (fenced dispatch), which are all in the router. It
  reaches the vault and the stored record through a small trait,
  `ReceiverVault`, that `ControlPlaneService` implements. It is not behind
  the `mailbox` Cargo feature: that feature is the mailbox *role*; any node
  may host a service with a mailbox. The local delivery goes through a
  trait, `DeferredDispatch`, that `RouteHandler` implements, so unit tests
  can use a fake.
- **C6-2 One pass at a time.** A `tokio::sync::Mutex` around the puller
  state. The loop runs a pass at start, then waits for 60 s, a wake, or
  cancellation. `security.mailbox-pull-now` takes the same lock and runs a
  pass. Services are pulled one after another, messages one after another.
- **C6-3 When to register.** For a service: the first pass of this process,
  a stored deploy record different from the one last registered (compare
  `pkarr_packet_hex`), a different mailbox, a `claim` that answered
  `unknown_destination` (`-32040`), or one hour since the last successful
  `register`. `claim` already keeps the queue alive (it updates
  `last_active_at`), so `register` is not needed on every pass.
- **C6-4 Service-level outcomes.**

  | Situation | What the puller does |
  | --- | --- |
  | No valid instance certificate (`ProxyRouter::instance_identity` is `None`) | Skip. Note `no_certificate` (C6-10). The supervisor renews it; `svc deploy` services wait for a new deploy (R-1). |
  | Certificate is the one that got `-32045` or `-32046` before | Skip. Note `instance_refused` or `stale_instance`. Clear the block when the certificate changes (`temporary_did` or `issued_at_secs` differs). |
  | `mailbox_key` is `Ok(None)` | Skip. Note `no_key`. |
  | `mailbox_key` is `Err` (vault locked, I/O) | Skip. Note `key_unreadable`. Next pass tries again. |
  | No stored deploy record, or it does not parse, or it names no mailbox | Skip. Note `no_record`. |
  | Stored record names another mailbox than the hosted-mailbox fact | Use the stored record (`task.md`). Note `record_and_fact_differ`. |
  | `register` ok | Keep `limits`; write `limits.max_ttl_secs` back (C6-8). |
  | `register` `-32047` (`bad_record`) | Note `record_expired` (text: "the owner must deploy again to renew the record"). Go on to `claim` with the last known `max_ttl_secs`. |
  | `register` or `claim` `-32045` | Block on this certificate (`instance_refused`). Stop for this pass. |
  | `register` or `claim` `-32046` | Block on this certificate (`stale_instance`: "a newer instance holds the queue"). Stop for this pass. |
  | `register` `-32044` (`mailbox_full`) | Note `mailbox_full`. Stop for this pass. |
  | `claim` `-32040` | Forget the registration. Register once more in this pass, then claim again. A second `-32040` stops the pass for this service. This register, like every other, first checks that the hosted-mailbox fact did not change during the pass (C6-18): `-32040` is also what a claim gets just after the control plane removed the queue. |
  | Any other error (transport, timeout, `-32043`, `-32603`, not the native mailbox) | Stop for this pass, log at debug. |
  | The registered mailbox differs from the stored record's mailbox (a redeploy to another mailbox) | Forget the registration and register at the new one. The control plane already removed the old queue (C6-15). |
  | A service is no longer in `hosted_mailboxes` (undeploy, or a redeploy with no mailbox) | Forget its state. The control plane already removed the queue (C6-15). |

- **C6-5 Message-level outcomes** (in this order).

  | Check | Ack |
  | --- | --- |
  | `open` fails with `TooLarge`, `Undecryptable`, `Malformed`, `BadSignature` | Unsigned `rejected(e.reject_code())`. |
  | `open` fails with `Refused(r)` (`BadCertificate`, `TooOld`, `FromFuture`) | `r.receipt(instance, cert, now)` (signed only for `FromFuture`). On `FromFuture` log a warning: the decision depends on this node's clock. |
  | Sender master's anchor cannot be read | No ack (temporary). |
  | Sender instance key (`sender_instance_did`) is in the master's `revoked_keys` | Signed `rejected(revoked)` (`D-C2-5`). |
  | Empty idempotency key | Signed `rejected(not_eligible)`: the fence cannot run a call without a key. |
  | Interface not served by this service (`EndpointRegistry::lookup` is `None`) | Signed `rejected(not_eligible)`. |
  | `(canonical interface, method)` not eligible (`HostedMailbox::accepts`) | Signed `rejected(not_eligible)` (R-5). |
  | `dispatch_deferred` → `NotServed` (a TCP or container service) | Signed `rejected(not_eligible)`. |
  | `dispatch_deferred` → `NotRun` (fence refused, already running, dispatch error) or the 30 s timeout | No ack (R-6). The mailbox closes it after 5 claims (M-14). |
  | `dispatch_deferred` → `Succeeded` (a success answer, fresh or replayed) | Signed `delivered`. |
  | `dispatch_deferred` → `Failed { code }` (an error answer, fresh or replayed, host-made `-32603` included) | Unsigned `rejected(call_failed)` (Q3). Log the code at debug. |
  | Signing fails | No ack; warn. |
  | The claim's visibility deadline is near (C6-12) | Stop handling; the rest of the batch is not acked and comes back after the visibility time. |

  The ack goes to `(claimed.sender, deposit_id)` from the claim answer. If
  `claimed.sender` differs from `opened.sender_master_did()`, log at debug
  and still deliver as the sealed sender: the sealed proof is the
  authority (R-3).
- **C6-6 The delivery caller** (Q7).
  ```rust
  let cert = &opened.sender_certificate;
  let key = substrate::resolve_did_key(&cert.temporary_did)?; // VerifyingKey
  let mut caller = CallerContext::verified(&cert.master_did, AuthLevel::Delegated, Vec::new());
  caller.session.verified_at_secs = now;
  caller.proof = Some(CallerProof {
      pubkey_hex: hex::encode(key.to_bytes()),
      delegation_json: cert.to_json().ok(),
      signed: None,
      on_behalf: None,
  });
  ```
  `resolve_did_key` cannot fail after `open` succeeded (the signature was
  checked against that DID); still map an error to "no ack".
- **C6-7 The fenced entry** (§2.2 item 6). `dispatch_json_rpc_once` keeps
  its signature and its frames. Its fenced part moves to a private
  `dispatch_fenced`, which returns:
  ```rust
  pub(crate) enum Fenced {
      Refused(ProxyError),
      Replayed(FirstOutcome),
      Ran(anyhow::Result<Vec<u8>>),
  }
  ```
  The new `RouteHandler::dispatch_deferred` maps it: `Refused` → `NotRun`;
  `Replayed(Success | SuccessNotRetained)` → `Succeeded`;
  `Replayed(CalleeError { code, .. })` → `Failed { code }`;
  `Ran(Ok(frame))` → `frame_as_outcome(&Ok(frame))`: `Ok` → `Succeeded`,
  `Err(Callee { code, .. })` → `Failed { code }`, an unreadable frame →
  `Failed { code: -32603 }` (the service made it); `Ran(Err(e))` →
  `NotRun`.
- **C6-8 Writing the learned time-to-live back.** After a successful
  `register`, if `limits.max_ttl_secs` differs from the fact, call
  `registry.set_mailbox(service_id, HostedMailbox { max_ttl_secs, ..fact })`
  **only if** `registry.mailbox_of(service_id)` still names the mailbox the
  puller registered with. A deploy that changed the mailbox meanwhile must
  not be overwritten. The small race left is accepted (one pass later the
  next `register` corrects it).
- **C6-9 `mailbox_methods`: type and path.**
  - `MailboxMethod` (text `<interface>#<method>`) lives in
    `crates/app_orchestration/src/models/service.rs`, beside
    `MailboxChoice`. Not in `crates/core`: `syneroym-roym-web` (a
    `wasm32-wasip2` crate) depends on `syneroym-app-orchestration`, so that
    crate must not gain a `syneroym-core` dependency.
  - The reserved-interface rule needs `NATIVE_CAPABILITY_INTERFACES` from
    `crates/core`. It reuses the existing node-level helper: move
    `call_dedup::is_node_level_interface` (router) into `crates/core` as
    `local_registry::is_node_level_interface` (same names, same short-hash
    match; `call_dedup` calls it), and add
    `mailbox::is_reserved_mailbox_interface` = node-level **or** a native
    capability name, each matched by name or short hash. The substrate
    (authoritative), roymctl (pre-flight) and the supervisor's `submit`
    (`app_supervisor` already depends on `core`) call it.
  - An entry may name its interface by the short hash. `accepts` matches an
    entry interface equal to the canonical name **or** to
    `short_hash(canonical name)`, so such an entry works instead of never
    matching.
  - On the wire: a new field `methods: list<string>` in the WIT record
    `mailbox-credentials` (it is present exactly when a mailbox is).
  - On the substrate: `HostedMailbox.methods: Vec<String>` (checked text),
    persisted in a new column of `service_mailboxes`. No migration
    (product not released).
  - At most 64 entries, no duplicates, each part 1 to 128 bytes, no
    whitespace, exactly one `#`. Only with a mailbox.
- **C6-10 Notes once an hour.** A small keyed limiter
  `HashMap<(String, Note), Instant>` in the puller: the first time and then
  once an hour per `(service, note)` it writes `warn!` and increments
  `substrate.mailbox_receiver.blocked{reason}`; other times `debug!`. Not
  the substrate's `WarnLimiter` (one slot, 5 minutes, `pub(super)` in
  `runtime/home_relay.rs`).
- **C6-11 The served-record check.** At most once an hour per service, the
  puller looks up its own service record (`RegistryClient::lookup(id,
  false)`, 5 s timeout). If the served record names another mailbox, or
  none, note `served_record_differs` naming both mailbox DIDs. Pulling
  continues from the stored record. A lookup error is ignored (debug).
- **C6-12 Bounds.** Claim batch `MAX_CLAIM_BATCH` (32). At most 8 claim
  rounds per service per pass (`MAX_CLAIM_ROUNDS`), then the next service.
  A round that returns fewer than 32 messages ends the service. Each local
  delivery is bounded by `DEFAULT_PROXY_CALL_TIMEOUT` (30 s). Each mailbox
  call is bounded by the existing `MAILBOX_CALL_TIMEOUT` (10 s).
  **A round stays inside its visibility time** (M-6, 5 minutes by
  default). The deadline is measured **on the receiver's own clock only**:
  `visible_again_at_secs` in the claim answer is the mailbox's clock, and
  two nodes may differ by up to 24 hours (the allowance used everywhere
  else), so the puller never compares it with its own time. Instead:
  - At each `register`, the puller also calls `info` once and keeps
    `visibility_secs` in the `Registration` (300 when `info` fails or the
    service has not registered in this process).
  - It takes `let claimed_at = Instant::now();` just **before** it sends
    `claim`, so the local window can only be shorter than the mailbox's.
  - Round budget: `visibility_secs - ACK_MARGIN_SECS` (30 s), at least 1 s.
  - Before each message **except the first**, it stops when
    `claimed_at.elapsed() + DEFAULT_PROXY_CALL_TIMEOUT >= budget`. The
    first message of a round is always handled, so a short visibility time
    or a slow service still makes progress.
  - It sends the acks it has after every 8 handled messages (`ACK_FLUSH`)
    and when it stops.
  The messages it did not start come back after the visibility time. So a
  slow service never lets a delivered message become visible again
  unacked, and never burns a claim toward the M-14 limit for a message it
  handled. Rate: a round makes at most 1 claim and 4 acks, so a pass makes
  at most 40 receiver calls per service (plus `register` and `info` once an
  hour; `info` is not rate-limited), under the mailbox default of 120
  per minute per master (`D-C4-21`); a `pull-now` during a timer pass can
  double that, still under 120. The puller reads every `AckOutcome`:
  `Stored` is counted; `NotFound` (the mailbox closed or swept the message
  meanwhile) is logged at debug; `Refused` (the mailbox would not take the
  receipt) is logged as a warning, because it means a bug.
- **C6-13 Wakes.** `ControlPlaneService` wakes the puller (no wait) after a
  successful `renew-cert`, after a successful `security.inject-kek` (the
  vault opens, so the keys become readable), and after a deploy whose
  validated mailbox is `Some`. R-1 then does not wait up to a minute.
- **C6-14 Constants, not config** (like `D-C5-9`): pass interval 60 s,
  register refresh 1 h, note interval 1 h, served-record check 1 h, lookup
  timeout 5 s, 8 claim rounds, ack flush every 8 messages, ack margin 30 s.
- **C6-15 Removing a queue (best effort), driven by the control plane.**
  The control plane removes the queue at the moment it changes the
  hosted-mailbox fact, while the service's owner and instance certificate
  are still installed. This matters for a **remote** mailbox: a remote
  call takes its identity from the registry at call time
  (`outbound_identity` → `instance_identity`, `proxy/router.rs:366-392`),
  and undeploy removes the owner, the certificate and the fact together
  (`lifecycle.rs:443-457`). A puller that noticed the undeploy on its next
  pass would reach a remote mailbox with the node key only, and the
  mailbox would refuse it (`not_an_instance`). So:
  - `MailboxPullControl` gets
    `async fn release(&self, service_id: &str, mailbox: &str)`: one
    `unregister` at `mailbox`, made as the service's **current** instance
    (`ProxyRouter::service_instance`, read from the registry now). It does
    not take the pass lock (a pass may run for minutes); it takes only the
    service's register gate (C6-18), which a pass holds for one `register`
    call at most. No instance (no valid certificate) → nothing is sent, debug log.
    `stale_instance` (`-32046`) is the expected answer after a relocation
    (a newer instance on another node holds the queue) and is ignored; any
    other error is logged at debug.
  - **Undeploy** (`remove_deploy_registry_rows`): read
    `registry.mailbox_of(service_id)`, then **remove the fact**
    (`forget_hosted_mailbox`), then call `release`, and only then
    `remove_owner` and `remove_instance_cert`. The fact goes first so a
    running pass cannot register the queue again (C6-18); the certificate
    goes last so the remote mailbox still admits the `unregister`.
  - **Redeploy that changes or drops the mailbox** (`commit_hosted_mailbox`):
    read the old fact; if the new mailbox differs or is `None`, **write the
    new fact first** (`record_hosted_mailbox` or `forget_hosted_mailbox`),
    then call `release(service_id, old.mailbox)`. The new instance
    certificate is already installed at that point (`commit.rs` installs it
    first). It has the same master and is newer, so the mailbox admits it
    (newest-certificate rule).
  - The control plane bounds each `release` with a 30 s timeout. It covers
    the wait for a `register` in flight (C6-18; one mailbox call, up to
    about 20 s when a remote call looks the mailbox up again) and the
    `unregister` itself. It only warns on failure: an undeploy or deploy
    never fails because of it.
  - The puller itself sends no `unregister`. At the start of a pass it
    only forgets the state of services that left `hosted_mailboxes`; when
    the stored record names another mailbox than its registration, it
    forgets the registration and registers at the new mailbox.
  When the queue is removed, the mailbox marks its messages `expired`
  (`store/queue.rs::unregister`), the senders go back to direct attempts
  and keep the payload (S-5), and the mailbox stops republishing the
  record. Limits (one backlog row): a mailbox that cannot be reached at
  that moment keeps the queue until its 30-day lifetime; a service with no
  valid certificate at that moment (expired, or deployed without one)
  cannot remove it. An undeploy on the old node during a relocation can
  remove the queue before the new node registers; its messages then go
  back to their senders, who deposit again.
- **C6-16 Shutdown.** `RuntimeServices::shutdown` cancels the token **and
  calls `JoinHandle::abort`** on the puller task. A pass can be long, and
  in-process restarts (the end-to-end tests) must not leave an old puller
  pulling with the same certificate and data directory. Aborting is safe:
  an unacked message comes back after the visibility time, and a delivery
  cut off inside the fence leaves its claim in flight, as a crash does.
- **C6-17 `security.mailbox-pull-now` has no time limit.** It waits for a
  pass that is running and then runs a full pass. A pass is bounded by its
  parts (C6-12) but not as a whole: with many services it can take
  minutes. The client's request timeout ends the wait for the caller; the
  pass still finishes. Written in the developer guide and as a backlog row.
- **C6-18 A register never brings back a removed queue.** `release` does
  not take the pass lock, so without more care a pass that is running for
  the same service can register at the mailbox after `release` removed the
  queue. The pass already holds its `Inputs`, with the stored record. It
  registers again when it was about to register anyway (the first pass,
  a new record, the hourly register), and when its next `claim` answers
  `-32040` (C6-4). The queue then comes back. The puller never sends
  `unregister` itself, so the mailbox keeps it and republishes the old
  record every hour until its `not_after`. The same happens to the old
  mailbox on a redeploy to another mailbox. (For a remote mailbox the late
  `register` mostly goes out after the certificate is removed, under the
  node key, and is refused; a same-node mailbox reads the instance proof
  from the `CallerContext` and admits it.) Two rules close it:
  - **Order.** The control plane changes the hosted-mailbox fact *before*
    it calls `release` (C6-15).
  - **Register gate.** The puller keeps one `tokio::sync::Mutex<()>` for
    each service (`register_gates`, outside the pass lock). A pass holds it
    around "check, then `register`"; `release` holds it around its
    `unregister`. The check, under the gate: `registry.mailbox_of(service_id)`
    still names the mailbox of the fact this pass started with
    (`pass.fact.mailbox`). It compares with the fact, not with the stored
    record, so the C6-4 rule "use the stored record when the two differ"
    still holds; it only asks whether the fact *changed* during the pass.
    When it did, the pass does not register, forgets the registration, and
    stops for this service; the next pass reads the new fact.

  With both rules the two calls cannot cross. If the register runs first,
  the `unregister` that follows removes the queue. If `release` runs
  first, the fact has already changed, so the check fails and no register
  is sent. The gate is held only for the `register` call itself, not for
  the `info` call after it or for claims, so `release` waits at most one
  mailbox call. A `claim` sent to the removed queue just answers `-32040`.
  Gates of services that left `hosted_mailboxes` are dropped at the start
  of a pass when no one holds them.

## 4. Step-by-step changes

### Step 1 — Write the decisions down first (docs only)

The owner accepted Q1 to Q9 as recommended (Q3 as revised after review:
no ADR amendment):

- `docs/planning/changes/service-mailbox/change.md` `## Deviations`: add
  `D-C6-1` … for C6-1 to C6-18 and the answers to Q1 to Q9 that change the
  design (pull-now name and gate, always-eligible Conversation methods,
  reserved interfaces and short-hash entries, a success acked `delivered`
  and an error answer acked with the unsigned `call_failed`, no
  capabilities on the delivery caller, unsigned proof cannot be passed on,
  fence-caller scope, no long-fence narrowing, `api#invoke` granularity,
  best-effort `unregister` driven by the control plane, the fact changed
  before `release` and the register gate, the C5 e2e migration). Update
  the Open
  questions: the eligibility question is built; the pull-now question
  stays open only for "who wakes the substrate".
- Change doc **Design → "Receiver flow"** (`change.md:224-226`): rewrite
  it to match: `security.mailbox-pull-now` (not `mailbox.pull-now`), the
  new fenced entry `dispatch_deferred` (not `dispatch_json_rpc_once`), the
  ack rules, and removing the queue on undeploy. The **"Sender flow"**
  diagram needs no change: `call_failed` is an unsigned rejection, which it
  already sends back to direct attempts.
- No ADR-0026 amendment: the unsigned `call_failed` follows A1 rule 2 as
  written ("for every other failure the receiver's rejection is
  unsigned").
- `task.md`:
  - C6: mark it in progress; fix "Parallel with C5" (serial).
  - C7: add a note: "Expired certificate" and "duplicate delivery" from the
    change doc's integration list are covered by C6 at the router level
    (`mailbox_pull/tests/blocked.rs`, `router/tests/call_dedup_dispatch.rs`);
    C7 runs the rest of the list (offline sender, receiver asleep longer
    than 2 hours, relocation, over quota, group push, expiry then direct).
  - C8: add a forward note from C6: the puller builds
    `RegisterParams { record }` in `mailbox_pull/service.rs::register`; C8
    adds `passport` there. `ReceiverVault` has no way to read the owner's
    child token; C8 adds a method (the token arrives in
    `mailbox-credentials`, next to `methods`). C8's cap on the passport
    fact `max_ttl_secs` also bounds the receiver fence window, because the
    puller writes the queue's `max_ttl_secs` into the hosted-mailbox fact
    (C6-8) and `window_for` reads it.
- `docs/planning/deferred-backlog.md`:
  - "A `group-push` that waits more than 24 hours…": move to "Recently
    resolved" (the receiver acks the refusal with an unsigned
    `call_failed`; the sender's direct retry gets the refusal from the
    fence and fails the member, as for a direct refusal; group sync
    repairs the entry).
  - New: "A queue is removed only when its mailbox can be reached at
    undeploy or redeploy" (C6-15 limits: an unreachable mailbox, or a
    service with no valid certificate at that moment, leaves the queue
    until its 30-day lifetime; `TBD`;
    `crates/control_plane/src/service/orchestration/mailbox_key.rs`).
  - New: "The receiver fence records a host-made failure as the call's
    answer" (a WASM engine error or a missing conversation service is
    recorded as `-32603`, so every later delivery of that key, direct or
    from the mailbox, gets that error; this was already true for direct
    calls; `TBD`; `crates/router/src/call_dedup.rs` `settle`).
  - New: "`security.mailbox-pull-now` has no time limit" (C6-17; `TBD`).
  - New: "An error answer to a sleeping receiver's mailbox delivery is
    deposited again until the sender reaches the receiver directly" (the
    unsigned `call_failed` sends the sender back to direct; if the receiver
    is asleep again, the sender deposits again; bounded by the 30-day
    sender age limit; `TBD`).
  - "The long receiver fence is bounded by a total…": target `TBD`, add the
    Q4 reason.
  - "A message that arrives both directly and from the mailbox…": keep the
    node-key and relocation cases, target `TBD`; say the mailbox delivery
    now presents the sealed master.

### Step 2 — `crates/mailbox_format` and `crates/core`: the shared mailbox facts

**2.0 `crates/mailbox_format`.**
- `receipt.rs`: a new `RejectCode::CallFailed` (`"call_failed"`), in the
  "never signed" group, added to `ALL` (its length becomes 11) and
  `as_str`. `is_sender_fault` stays false for it. Doc: "The receiving
  service answered with an error. The sender goes back to direct
  attempts, where the receiver fence gives it the same answer."
  `sign_rejection` already refuses it (`NotSignable`). Tests in
  `receipt/tests.rs`: round trip, not signable, and the sender's
  `verify_signed` treats an unsigned `call_failed` as `Unsigned`.
- A `test-support` Cargo feature that makes `seal_unchecked` public
  (`#[doc(hidden)]`), so the router tests can make a message with a
  certificate that does not verify (R-4 "bad certificate" case). Enable it
  only in the router's `[dev-dependencies]`.

**2.1 `crates/core`.**

`crates/core/src/local_registry.rs`: move `is_node_level_interface` here
from `crates/router/src/call_dedup.rs:195` (same body, same doc). It
names `"supervisor"` and `MAILBOX_INTERFACE` directly (core cannot see
`syneroym_control_plane::{SUPERVISOR_RESERVED_SERVICE_ID,
MAILBOX_RESERVED_SERVICE_ID}`); add `pub const SUPERVISOR_INTERFACE: &str =
"supervisor";` in core and make the control-plane constant
`SUPERVISOR_RESERVED_SERVICE_ID` refer to it, as
`MAILBOX_RESERVED_SERVICE_ID` already refers to `MAILBOX_INTERFACE`.
Every caller moves to `local_registry::is_node_level_interface`:
- `crates/router/src/call_dedup.rs:195` (the free function) is removed,
  and the method wrapper `CallDedupGuard::is_node_level_interface`
  (`:291`) is removed too; its one caller, `begin` (`:449`), calls the
  core function.
- `crates/router/src/proxy/router.rs:548` (`enqueue_call`) and
  `crates/router/src/proxy/saga_dispatch.rs:142` call
  `call_dedup::is_node_level_interface` today; change both to the core
  path.
- `crates/router/src/call_dedup/tests.rs:453-454` call the free function
  directly; move those two asserts to `core/src/local_registry/tests.rs`.
Check with `grep -rn is_node_level_interface crates` that no other caller
is left.

`crates/core/src/mailbox.rs` (170 lines now):

```rust
/// Conversation calls a sender may always deposit. Every service with a
/// mailbox accepts them; they arrive on the native `conversation` interface,
/// which no manifest declares.
pub const ALWAYS_ELIGIBLE: [(&str, &str); 2] =
    [("conversation", "deliver"), ("conversation", "group-push")];

/// The longest `mailbox_methods` list a service may declare.
pub const MAX_MAILBOX_METHODS: usize = 64;

/// Whether `interface` belongs to the node or the platform, so no service
/// may list it in `mailbox_methods`. Matched by name and by short hash,
/// like `is_node_level_interface`.
#[must_use]
pub fn is_reserved_mailbox_interface(interface: &str) -> bool {
    local_registry::is_node_level_interface(interface)
        || NATIVE_CAPABILITY_INTERFACES
            .iter()
            .any(|name| *name == interface || util::short_hash(name) == interface)
}

pub struct HostedMailbox {
    pub mailbox: String,
    pub max_ttl_secs: u64,
    /// The guest methods this service accepts from its mailbox, as
    /// `<interface>#<method>`, besides `ALWAYS_ELIGIBLE`. Checked at deploy.
    #[serde(default)]
    pub methods: Vec<String>,
}

impl HostedMailbox {
    /// Whether a call to `interface` (canonical name) and `method` may be
    /// delivered from the mailbox. An entry may name the interface by its
    /// short hash.
    #[must_use]
    pub fn accepts(&self, interface: &str, method: &str) -> bool {
        if ALWAYS_ELIGIBLE.contains(&(interface, method)) {
            return true;
        }
        let hash = util::short_hash(interface);
        self.methods.iter().filter_map(|m| m.split_once('#')).any(|(i, m)| {
            m == method && (i == interface || i == hash)
        })
    }
}

/// What one pull pass did (`security.mailbox-pull-now`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullSummary {
    /// Services with a mailbox that the pass looked at.
    pub services: u32,
    /// Services skipped (no certificate, no key, no record, blocked).
    pub blocked: u32,
    pub delivered: u32,
    pub rejected: u32,
    /// Messages left for a later claim (a temporary local failure).
    pub not_acked: u32,
}
```

Use `crate::{local_registry::{self, NATIVE_CAPABILITY_INTERFACES}, util}`.
Both modules are in one crate, so the mutual use is fine.

`crates/core/src/local_registry.rs`, beside `mailbox_of` (`:352`):

```rust
/// Every hosted service with a mailbox, sorted by service id.
#[must_use]
pub fn hosted_mailboxes(&self) -> Vec<(String, HostedMailbox)>
```

`crates/data_db/src/registry_store.rs`:
- `CREATE TABLE … service_mailboxes` (`:91`): add
  `methods TEXT NOT NULL DEFAULT '[]'` (a JSON array of strings).
- `load_all_mailboxes` (`:345`): select it, `serde_json::from_str` into
  `Vec<String>`; a parse error is an error (`anyhow`), like a bad row.
- `save_mailbox` (`:365`): write it (`serde_json::to_string`), and update
  it in `ON CONFLICT … DO UPDATE`.

Literal sites of `HostedMailbox { … }` to update (add `methods`):
`core/src/local_registry/tests.rs:323`,
`control_plane/src/service/orchestration/mailbox_key.rs:58` (Step 5),
`control_plane/.../tests/mailbox.rs:363`,
`data_db/src/registry_store.rs:354`,
`data_db/src/registry_store/tests.rs:475`,
`router/src/call_dedup/tests.rs:457`.

Tests (`crates/core/src/mailbox/tests.rs`): `accepts` for the two
Conversation pairs, a listed pair, an unlisted method on a listed interface,
an entry that names the interface by its short hash (accepted);
`is_reserved_mailbox_interface` for each reserved name and its short hash;
the moved `is_node_level_interface` gets the two asserts from
`call_dedup/tests.rs:453-454` plus name and short-hash tests for the other
node-level names in `local_registry/tests.rs` (the `call_dedup` tests that
reach it only through `begin` stay where they are); a
`HostedMailbox` without `methods` in JSON reads as empty. In
`registry_store/tests.rs`: methods round-trip and an upsert replaces them.

### Step 3 — `crates/app_orchestration`: the manifest field

`crates/app_orchestration/src/models/service.rs`, beside `MailboxChoice`
(`:303-340`):

```rust
/// A guest method that accepts delivery from the service's mailbox
/// (ADR-0026 Decision 5). Text form `<interface>#<method>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct MailboxMethod { interface: String, method: String }

impl MailboxMethod {
    pub fn interface(&self) -> &str;
    pub fn method(&self) -> &str;
}
impl FromStr for MailboxMethod { type Err = anyhow::Error; … }
impl fmt::Display for MailboxMethod { … "{interface}#{method}" }
impl From<MailboxMethod> for String;
impl TryFrom<String> for MailboxMethod;
```

`from_str` rules: exactly one `#`; both parts 1 to 128 bytes; no ASCII
whitespace or control characters. The error text names the entry. No
reserved-interface check here (C6-9).

`ServiceSpec` (after `instance_cert_expires_hours`, `:414`):

```rust
/// Guest methods that accept delivery from the mailbox (ADR-0026). Needs
/// `mailbox`. Conversation `deliver` and `group-push` are always accepted
/// and are not listed.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub mailbox_methods: Vec<MailboxMethod>,
```

`PlannedService` (`models/plan.rs:80`): the same field, "Cloned from
`ServiceSpec.mailbox_methods`; the supervisor holds no manifest, so the
plan is where it reads it."

`compiler.rs`:
- `planned_members` (`:188`): `mailbox_methods: spec.mailbox_methods.clone(),`.
- `validate_plan_member_settings` (`:135`): for each service, if
  `mailbox_methods` is not empty: refuse when `mailbox` is `None`
  ("'{name}' lists mailbox_methods but has no mailbox"); refuse more than
  `MAX_MAILBOX_METHODS` (copy the value 64 as a local constant here with a
  comment that the substrate enforces the same limit; app_orchestration
  cannot import core); refuse duplicates.

Every other literal `ServiceSpec { … }` / `PlannedService { … }`: add
`mailbox_methods: Vec::new(),` next to `instance_cert_expires_hours`. The
full list (from `grep -rn 'instance_cert_expires_hours[:,]'`):
`substrate/tests/{gateway_hostname_e2e.rs:105, supervisor_interface_e2e.rs:66,95,
multi_substrate_placement_e2e.rs:122, scheduled_task_e2e.rs:113,
supervisor_loop_e2e.rs:82,111, topology_document_e2e.rs:99,156,
reference_scenario_e2e.rs:122,151, binding_push_e2e.rs:111,140,
durable_outbox_e2e.rs:124,153, common/fixtures.rs:142}`,
`app_orchestration/src/{journal.rs:431, substrate_inventory.rs:255,
catalog.rs:76, reconcile.rs:192, compiler/tests/visibility.rs:42,
models/tests.rs:210,355,455,579,916}`,
`app_supervisor/src/{keys.rs:661, topology.rs:246,
service/tests/helpers.rs:435}`,
`sdk/src/{deploy/tests.rs:94, mapper/tests.rs:54,349,369}`,
`apps/roymctl/src/commands/{member_identity.rs:316, app/deploy.rs:243,
app/tests.rs:105}`. Compile errors list any site this misses. Three more
places read the C3 fields in assertions and need no change to compile, but
are where the new assertions go: `compiler/tests/member_settings.rs:39,49`
and `models/tests/member_settings.rs:25`.

Tests: `models/tests/member_settings.rs` (parse a manifest with
`mailbox_methods`, refuse `"a#b#c"`, `"#m"`, `"i#"`, `"i #m"`),
`compiler/tests/member_settings.rs` (copied to every member; refused
without a mailbox; refused above 64; refused on a duplicate).

### Step 4 — WIT, SDK and roymctl

`crates/wit_interfaces/wit/control-plane/control-plane.wit` (`:178-184`):

```wit
record mailbox-credentials {
    /// The member's mailbox secret key: z-base-32 of its 32-byte seed.
    secret-key: string,
    /// Guest methods (`<interface>#<method>`) that accept delivery from
    /// the mailbox. Conversation `deliver` and `group-push` are always
    /// accepted and are not listed.
    methods: list<string>,
}
```

Only host crates see this record (`grep MailboxCredentials`: sdk,
control_plane, one substrate test), so no guest component needs a rebuild
for it. Check with `grep -rn mailbox-credentials crates/*/src/bindings.rs
test-components` before you skip the rebuild.

`MailboxCredentials { … }` literal sites (add `methods`):
`sdk/src/mapper.rs:394`, `sdk/src/types.rs:91`,
`control_plane/.../tests/mailbox.rs:79,93,173`,
`substrate/tests/mailbox_record_e2e.rs:200`.

`crates/sdk/src/mapper.rs` `map_planned_service` (`:390`):

```rust
let mailbox = ctx.members.mailbox_keys.get(&svc.service_id).map(|key| MailboxCredentials {
    secret_key: key.to_z32().to_string(),
    methods: svc.mailbox_methods.iter().map(ToString::to_string).collect(),
});
```

`crates/sdk/src/types.rs`: `Publication::Public` gains
`mailbox_methods: Vec<MailboxMethod>`; `split` copies it into
`MailboxCredentials.methods` (only when `mailbox_key` is `Some`). Update
the three literals in `sdk/src/tests.rs:75,91,101` and the four in the
substrate tests: `service_visibility_e2e.rs:294`, `mailbox_record_e2e.rs:120`,
`record_signing_e2e.rs:44`, `common/mailbox_receiver.rs:159` (if that file
still exists after Step 12.4). The `Debug` of `PublicationParts` stays as
it is (it already hides the whole record).

`crates/sdk` also gets the shared plan check (used by roymctl and the
supervisor, Step 5.5):

```rust
/// The services of `plan` that list a node or platform interface in
/// `mailbox_methods`, as error texts. The substrate refuses them at deploy;
/// this refuses them before anything is placed.
pub fn refuse_reserved_mailbox_methods(plan: &DeploymentPlan) -> Result<(), Vec<String>>
```

`crates/sdk/src/client/orchestrator.rs` (beside `home_relay_set`, `:412`):

```rust
/// Runs one mailbox pull pass on the node now and answers what it did.
/// Node owner only.
pub async fn mailbox_pull_now(&self) -> Result<PullSummary> {
    let res = self.request("security", "mailbox-pull-now", serde_json::json!([])).await?;
    Ok(serde_json::from_value(res.result)?)
}
```

`apps/roymctl/src/commands/svc.rs` (deploy args, beside `mailbox`,
`:154-161`):

```rust
/// A guest method that accepts delivery from the mailbox, as
/// `<interface>#<method>`. Repeat for more. Needs `--mailbox`.
#[arg(long = "mailbox-method", value_name = "INTERFACE#METHOD")]
mailbox_methods: Vec<MailboxMethod>,
```

`apps/roymctl/src/commands/svc/deploy.rs`:
- `check_mailbox_args` (`:89`) gains `methods: &[MailboxMethod]`: refuse
  methods without `--mailbox`; refuse more than `MAX_MAILBOX_METHODS`,
  duplicates, and any `is_reserved_mailbox_interface(m.interface())`.
- `build_publication` (`:190`): `Publication::Public { record,
  mailbox_key, mailbox_methods }`.
- `apps/roymctl/src/commands/app/deploy.rs`: where the compiled plans are
  checked with `validate_plan_member_settings`, also call
  `syneroym_sdk::refuse_reserved_mailbox_methods` for every compiled plan,
  including spawned apps, like `D-C3-17`.

Tests: `cli_args.rs` (the flag parses and repeats; refused without
`--mailbox`), `svc/tests.rs` (the methods reach `Publication::Public`;
reserved interface refused), `app/tests/settings.rs` (reserved interface in
a manifest refused), `sdk` `mapper/tests.rs` and `deploy/tests/mailbox.rs`
(methods reach `MailboxCredentials`), `sdk/src/tests.rs` (`split`).

### Step 5 — `crates/control_plane`

**5.1 Deploy validation** (`service/orchestration/types.rs`):

```rust
pub(crate) struct ValidatedMailbox {
    pub(super) key: MailboxSecretKey,
    /// The node DID of the mailbox the verified record names.
    pub(super) mailbox: String,
    /// Checked `<interface>#<method>` texts.
    pub(super) methods: Vec<String>,
}

pub(crate) struct ValidatedDeploy {
    pub(super) instance_cert: Option<DelegationCertificate>,
    pub(super) visibility: AppVisibility,
    pub(super) mailbox: Option<ValidatedMailbox>, // replaces mailbox_key + mailbox
}

pub(crate) fn validate_mailbox(…) -> Result<Option<ValidatedMailbox>, String>
```

In the `(Some(credentials), Some(mailbox))` arm, after the key checks:
parse each `credentials.methods` entry with `MailboxMethod::from_str`;
refuse more than `MAX_MAILBOX_METHODS`, duplicates, and
`is_reserved_mailbox_interface`; keep `m.to_string()`. The error text may
name the method, never the key.

Call sites: `deploy/admission.rs:113` (the `validate_mailbox` call) and
`:124` (build `ValidatedDeploy { …, mailbox }`), `deploy/commit.rs:81-110` (use `validated.mailbox.as_ref()`
for the key store and the fact), `tests/mailbox.rs:103,113,123,137`.

**5.2 The fact** (`mailbox_key.rs`):
`record_hosted_mailbox(service_id, mailbox, methods: Vec<String>)` keeps
`max_ttl_secs` for the same mailbox as today and always writes the new
`methods`. `commit_hosted_mailbox` (`commit.rs:6`) passes them.

**5.3 The stored deploy record** (`mailbox_key.rs`, new):

```rust
/// The record the deployer signed for `service_id`, as stored at deploy,
/// even past its `not_after`. `None` when the service has none. The
/// mailbox receiver registers with it; it is not verified here.
pub async fn deploy_record(&self, service_id: &str) -> anyhow::Result<Option<SignedEndpointInfo>> {
    let path = self.hosted_apps_dir.join(format!("{service_id}.json"));
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
```

**5.4 The pull hook** — new file `crates/control_plane/src/mailbox_pull.rs`
(the pattern of `src/home_relay.rs`):

```rust
/// The node's mailbox receiver, as the control plane sees it.
#[async_trait::async_trait]
pub trait MailboxPullControl: Send + Sync + fmt::Debug {
    /// Runs one pass now, after any pass that is running, and answers what
    /// it did.
    async fn pull_now(&self) -> PullSummary;
    /// Asks for a pass soon. Does not wait.
    fn wake(&self);
    /// Removes the service's queue at `mailbox` (one `unregister`, made as
    /// the service's current instance). Best effort; never fails.
    async fn release(&self, service_id: &str, mailbox: &str);
}
```

`mailbox_key.rs` gets the caller of `release` (C6-15):

```rust
/// How long removing an old queue may hold up an undeploy or deploy: the
/// wait for a `register` in flight, then the `unregister`. A remote mailbox
/// is looked up first.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);

/// Removes `service_id`'s queue at `mailbox`, while its instance
/// certificate is still installed: a remote mailbox only admits the call
/// under that instance. Warns and goes on when it cannot.
pub(super) async fn release_mailbox_queue(&self, service_id: &str, mailbox: &str) {
    let Some(puller) = self.mailbox_pull.get().and_then(Weak::upgrade) else { return };
    if time::timeout(RELEASE_TIMEOUT, puller.release(service_id, mailbox)).await.is_err() {
        tracing::warn!("Removing the old mailbox queue of {service_id} timed out; it ends with its lifetime");
    }
}
```

Callers:
- `lifecycle.rs::remove_deploy_registry_rows` (`:442`): the function
  starts with
  ```rust
  let old = self.registry.mailbox_of(service_id);
  self.forget_hosted_mailbox(service_id).await; // moved up from the end
  if let Some(old) = old {
      self.release_mailbox_queue(service_id, &old.mailbox).await;
  }
  ```
  before `remove_owner` and `remove_instance_cert`. Write both reasons in
  a comment: the fact goes first so a running pass does not register the
  queue again; the certificate goes after, because the remote mailbox
  admits only the instance. `clear_mailbox_key` stays where it is.
- `commit.rs::commit_hosted_mailbox` (`:6`): read
  `let old = self.registry.mailbox_of(service_id);` first, then run
  `record_hosted_mailbox` / `forget_hosted_mailbox` as today; after that
  write succeeds, when `old.mailbox` differs from the new mailbox, or the
  new one is `None`, call `release_mailbox_queue(service_id,
  &old.mailbox)`. If the write fails, the deploy rolls back through
  undeploy, which releases the queue the same way. The new instance
  certificate is installed earlier in `commit_deploy_registry_writes`, so
  the call goes out as the new instance of the same master.

`service.rs`: field `pub(crate) mailbox_pull: OnceLock<Weak<dyn
MailboxPullControl>>` (initialised in the constructor beside
`home_relay`), setter:

```rust
/// Wires in the mailbox receiver. `Weak`: the receiver holds the route
/// handler, which holds this service. No-op after the first call.
pub fn set_mailbox_pull(&self, puller: Weak<dyn MailboxPullControl>)
```

and a helper `pub(crate) fn wake_mailbox_pull(&self)` (upgrade and call
`wake`; nothing when unset).

`service/dispatch.rs` `dispatch_security_interface`: add
`"mailbox-pull-now" => mailbox_pull::pull_now(self.mailbox_pull.get()).await,`.
New file `service/dispatch/mailbox_pull.rs`: no params (accept `[]` or
`null`); upgrade the `Weak`; when unset or gone answer
`RpcError::MethodNotFound("this substrate runs no mailbox receiver")`;
otherwise `to_response(&puller.pull_now().await)`.

Wakes (C6-13): `dispatch_renew_cert` (`dispatch.rs:325`) after success;
the `"inject-kek"` arm of `dispatch_security_interface` after
`key_store.inject_kek` succeeds; `commit_deploy_registry_writes`
(`commit.rs`) after `commit_hosted_mailbox` succeeds with `Some`.

Tests (`tests/mailbox.rs` and a new `dispatch/mailbox_pull/tests.rs`):
methods are stored and replaced by a redeploy; a reserved interface (by
name and by short hash), a duplicate and 65 entries are refused;
`deploy_record` reads a stored file, `None` without one;
`mailbox-pull-now` answers the fake's summary, needs `substrate/admin`, and
answers `MethodNotFound` when unwired; a renew-cert, an inject-kek and a
deploy with a mailbox each wake the fake; an undeploy calls `release` with
the old mailbox **while the instance certificate and owner are still in
the registry** (the fake checks `registry.instance_cert` and `owner_of`
inside `release`); a redeploy to another mailbox and a redeploy with no
mailbox call `release` with the old one; a redeploy to the same mailbox
does not; a `release` that never returns holds the undeploy for at most
`RELEASE_TIMEOUT` (paused time); when `release` runs, the hosted-mailbox
fact is already gone (undeploy) or already names the new mailbox
(redeploy) (the fake checks `registry.mailbox_of` inside `release`,
C6-18).

**5.5 The supervisor.** `crates/app_supervisor/src/service/verbs/submit.rs`
(`:337`, after `validate_plan_member_settings`): refuse a plan whose
services list a reserved interface in `mailbox_methods`, with
`RpcError::InvalidParams`, by calling
`syneroym_sdk::refuse_reserved_mailbox_methods(plan)` (Step 4;
`app_supervisor` already depends on `syneroym-sdk`). Test in
`app_supervisor`'s submit tests: a reserved interface is refused before
anything is placed.

### Step 6 — `crates/router` proxy helpers (no behaviour change)

`crates/router/src/proxy/mailbox.rs`:
- Rename `DepositSender` → `ServiceInstance` and `sender_identity` →
  `service_instance`; make both `pub(crate)` (fields too). The sender path
  and the receiver now both present this.
- New:

  ```rust
  /// One mailbox call made as `instance`. Refused unless the mailbox, when
  /// it is on this node, is the native one.
  pub(crate) async fn call_mailbox(
      &self,
      instance: &ServiceInstance,
      mailbox: &str,
      method: &str,
      params: Value,
  ) -> Result<Value, ProxyError> {
      if !self.is_native_mailbox(mailbox) {
          return Err(ProxyError::PermissionDenied(format!(
              "'{mailbox}' is not the native mailbox of its node"
          )));
      }
      self.dispatch_call(&Self::mailbox_request(instance, mailbox, method, params)).await
  }
  ```

  Use it in `deposit` and `poll_chunk` too, so there is one way to call a
  mailbox. Keep `DepositFailure::NotNative` by mapping the
  `PermissionDenied` from `call_mailbox`, or drop the variant if the
  message is clear enough; the sender tests decide.
- `proxy/mailbox/poll.rs`: move the anchor read out of
  `receiver_key_revoked` into

  ```rust
  /// The revoked keys of `master`, read once per `cache`; `None` when the
  /// anchor cannot be read.
  pub(crate) async fn revoked_keys<'a>(&self, master: &str, cache: &'a mut AnchorCache)
      -> Option<&'a [String]>
  ```

  and make `AnchorCache` `pub(crate)`. `receiver_key_revoked` calls it.
- `proxy.rs`: `pub(crate) fn registry_client(&self) -> &Arc<RegistryClient>`.

All existing proxy tests must pass unchanged except for the renames.

### Step 7 — `crates/router` route handler: the deferred entry

**7.1** `route_handler/dispatch.rs`: split `dispatch_json_rpc_once`
(`:124-176`).

```rust
/// What the fence decided for one keyed call, and the answer if it ran.
pub(crate) enum Fenced {
    Refused(ProxyError),
    Replayed(FirstOutcome),
    Ran(Result<Vec<u8>>),
}

async fn dispatch_fenced(
    &self,
    guard: &CallDedupGuard,
    pipeline: &RoutePipeline,
    preamble: &RoutePreamble,
    caller: Option<&CallerContext>,
    body: &[u8],
    key: &str,
) -> Fenced {
    // target_service: the same match as today (`:139-143`)
    match guard.begin(target_service, &preamble.interface, caller.map(|c| c.caller_did.as_str()), Some(key)).await {
        GuardOutcome::Refuse(e) => Fenced::Refused(e),
        GuardOutcome::Answer(outcome) => Fenced::Replayed(outcome),
        GuardOutcome::Execute(claim) => {
            let response = self.dispatch_json_rpc_unfenced(pipeline, preamble, caller, body).await;
            if let Some(claim) = claim {
                claim.settle(&frame_as_outcome(&response)).await;
            }
            Fenced::Ran(response)
        }
    }
}
```

`dispatch_json_rpc_once` keeps: no `KeyProbe` → unfenced; no guard → the
same `-32603` frame; then `Refused(e)` → the `substrate.request.errors`
counter and `json_error(probe.id, e.code(), e.to_string())`;
`Replayed(o)` → `replay_frame(probe.id, o)`; `Ran(r)` → `r`.
`dispatch.rs` is 676 lines; put 7.2 in a new file.

**7.2** New file `crates/router/src/route_handler/deferred.rs`
(`pub mod deferred;` in `route_handler.rs`):

```rust
/// What became of a call handed to a service from its mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeferredOutcome {
    /// The service answered with a result, now or before (the fence
    /// replayed it). The result itself is not kept.
    Succeeded,
    /// The call ran and answered an error, now or before. The fence keeps
    /// this answer, so a direct call with the same key gets it too.
    Failed { code: i32 },
    /// Nothing ran, for a reason that may pass.
    NotRun(String),
    /// This service cannot take a JSON-RPC call (a TCP or container one).
    NotServed,
}

impl RouteHandler {
    /// Hands a call that waited at the service's mailbox to the service,
    /// through the same fence and dispatch a call from the wire meets.
    /// `interface` is the canonical name. `caller` is the original sender.
    pub async fn dispatch_deferred(
        &self,
        service_id: &str,
        interface: &str,
        content: &MessageContent,
        caller: &CallerContext,
    ) -> DeferredOutcome {
        let Some((endpoint, interface)) = self.inner.registry.lookup(service_id, interface) else {
            return DeferredOutcome::NotRun("the service does not serve this interface".into());
        };
        let preamble = RoutePreamble::binary_json_rpc(service_id, &interface);
        let pipeline = self.plan_pipeline(&preamble, &endpoint);
        if !matches!(pipeline.service,
            ServiceStage::NativeService { .. } | ServiceStage::WasmComponent { .. }) {
            return DeferredOutcome::NotServed;
        }
        let Some(guard) = &self.inner.dedup_guard else {
            return DeferredOutcome::NotRun("this node keeps no per-service storage".into());
        };
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: content.method.clone(),
            params: content.params.clone(),
            id: Some(Value::from(1)),
            idempotency_key: Some(content.idempotency_key.clone()),
        };
        let Ok(body) = serde_json::to_vec(&request) else {
            return DeferredOutcome::NotRun("the call does not encode".into());
        };
        match self.dispatch_fenced(guard, &pipeline, &preamble, Some(caller), &body,
                                   &content.idempotency_key).await {
            Fenced::Refused(e) => DeferredOutcome::NotRun(e.to_string()),
            Fenced::Replayed(outcome) => outcome_of(outcome),
            Fenced::Ran(Ok(frame)) => answer_of(&frame),
            Fenced::Ran(Err(e)) => DeferredOutcome::NotRun(e.to_string()),
        }
    }
}

/// A stored first answer, as the mailbox receiver sees it.
fn outcome_of(outcome: FirstOutcome) -> DeferredOutcome {
    match outcome {
        FirstOutcome::Success(_) | FirstOutcome::SuccessNotRetained => DeferredOutcome::Succeeded,
        FirstOutcome::CalleeError { code, .. } => DeferredOutcome::Failed { code },
    }
}

/// A fresh response frame. A frame that does not parse was made by the
/// target, so it counts as an error answer.
fn answer_of(frame: &[u8]) -> DeferredOutcome {
    match frame_as_outcome(&Ok(frame.to_vec())) {
        Ok(_) => DeferredOutcome::Succeeded,
        Err(ProxyError::Callee { code, .. }) => DeferredOutcome::Failed { code },
        Err(_) => DeferredOutcome::Failed { code: -32603 },
    }
}
```

`frame_as_outcome` is private in `dispatch.rs`; make it `pub(super)`
(or give it a `&[u8]` sibling so no copy is needed) when you move it.

The WASM arm forwards `caller` into the guest's `HostState.caller`
(`dispatch.rs:119-123`), so R-3 holds for guests too. The proof in that
caller may carry a certificate that has expired since the send time
(R-4 accepts it). Nothing past the router reads `CallerContext.proof`
today (checked with a grep over `sandbox_wasm`, `control_plane`,
`conversation`, `app_host*` and `roym_core`); inside the router only the
mailbox (`caller.rs::instance_of`) and `on_behalf` passing read it, and
neither is on an eligible path. The R-4 tests in Step 12.2 keep it that
way. Do **not** use
`ProxyRouter::invoke_local` here: its WASM arm passes `caller: None`
(`proxy/router.rs:252-259`), so a guest would see itself as the caller.
Write that reason in a comment.

### Step 8 — `crates/router`: the puller

Module layout (all under `crates/router/src/`):

```
mailbox_pull.rs          // MailboxPuller, traits, constants, run, pull_once (~300 lines)
mailbox_pull/service.rs  // one service: inputs, register, claim rounds, acks (~350)
mailbox_pull/message.rs  // one claimed message: open, checks, deliver, receipt (~250)
mailbox_pull/notes.rs    // Note enum and the keyed hourly limiter (~80)
mailbox_pull/tests.rs    // `mod helpers; mod register; mod release; mod messages; mod rounds; mod blocked;`
mailbox_pull/tests/{helpers,register,release,messages,rounds,blocked}.rs
```

Add `pub mod mailbox_pull;` to `lib.rs` and re-export `MailboxPuller`,
`ReceiverVault`, `DeferredDispatch`, `DeferredOutcome`.

**8.1 `mailbox_pull.rs`**

```rust
const PULL_INTERVAL: Duration = Duration::from_secs(60);
const REREGISTER_INTERVAL: Duration = Duration::from_secs(3600);
const SERVED_RECORD_CHECK_INTERVAL: Duration = Duration::from_secs(3600);
const SERVED_RECORD_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CLAIM_ROUNDS: usize = 8;
const ACK_FLUSH: usize = 8;
const ACK_MARGIN_SECS: u64 = 30;

/// What the puller needs from the node's control plane.
#[async_trait::async_trait]
pub trait ReceiverVault: Send + Sync {
    async fn mailbox_key(&self, service_id: &str) -> anyhow::Result<Option<MailboxSecretKey>>;
    async fn deploy_record(&self, service_id: &str) -> anyhow::Result<Option<SignedEndpointInfo>>;
}

/// How a call from the mailbox reaches a service on this node.
#[async_trait::async_trait]
pub trait DeferredDispatch: Send + Sync {
    async fn dispatch_deferred(&self, service_id: &str, interface: &str,
        content: &MessageContent, caller: &CallerContext) -> DeferredOutcome;
}

pub struct MailboxPuller {
    proxy: Arc<ProxyRouter>,
    dispatch: Arc<dyn DeferredDispatch>,
    vault: Arc<dyn ReceiverVault>,
    registry: EndpointRegistry,
    state: tokio::sync::Mutex<PullState>,
    wake: Notify,
    /// One gate for each service, outside the pass lock: a `register` and
    /// a `release` of the same service never cross (C6-18).
    register_gates: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

#[derive(Default)]
struct PullState {
    services: HashMap<String, ServiceState>, // defined in service.rs
    notes: Notes,                            // defined in notes.rs
}

impl MailboxPuller {
    pub fn new(proxy, dispatch, vault, registry) -> Self;

    /// The register gate of `service_id` (C6-18), made on first use.
    fn register_gate(&self, service_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.register_gates.lock().unwrap_or_else(PoisonError::into_inner);
        gates.entry(service_id.to_string()).or_default().clone()
    }

    /// Pulls at start, then every minute, and when woken, until cancelled.
    pub async fn run(self: Arc<Self>, cancel: CancellationToken) {
        loop {
            self.pull_once().await;
            tokio::select! {
                () = cancel.cancelled() => return,
                () = self.wake.notified() => {}
                () = time::sleep(PULL_INTERVAL) => {}
            }
        }
    }

    /// One pass over every hosted service with a mailbox.
    pub async fn pull_once(&self) -> PullSummary {
        let mut state = self.state.lock().await;
        let PullState { services, notes } = &mut *state;
        let hosted = self.registry.hosted_mailboxes();
        // The control plane removed the queue of a service that is gone
        // (`release`); only the state is left to forget.
        services.retain(|id, _| hosted.iter().any(|(h, _)| h == id));
        // A gate nobody holds (only the map has it) of a service that is
        // gone is dropped; a `release` in progress keeps its own. A
        // poisoned lock only skips this clean-up.
        if let Ok(mut gates) = self.register_gates.lock() {
            gates.retain(|id, gate| Arc::strong_count(gate) > 1 || hosted.iter().any(|(h, _)| h == id));
        }
        let mut anchors = AnchorCache::new();
        let mut summary = PullSummary::default();
        for (service_id, fact) in hosted {
            summary.services += 1;
            let entry = services.entry(service_id.clone()).or_default();
            let mut pass = ServicePass { service_id: &service_id, fact: &fact, entry,
                                         notes, anchors: &mut anchors, summary: &mut summary };
            self.pull_service(&mut pass).await;
        }
        summary
    }
}

#[async_trait::async_trait]
impl MailboxPullControl for MailboxPuller {
    async fn pull_now(&self) -> PullSummary { self.pull_once().await }
    fn wake(&self) { self.wake.notify_one(); }

    /// C6-15. No pass lock: the control plane calls this inside an undeploy
    /// or deploy, which must not wait for a pass. It takes the service's
    /// register gate (C6-18), so a `register` in flight lands first and
    /// this `unregister` removes it.
    async fn release(&self, service_id: &str, mailbox: &str) {
        let Some(instance) = self.proxy.service_instance(service_id) else {
            debug!(service_id, "no instance certificate; the old queue stays until its lifetime ends");
            return;
        };
        let gate = self.register_gate(service_id);
        let _held = gate.lock().await;
        match self.proxy.call_mailbox(&instance, mailbox, METHOD_UNREGISTER, json!({})).await {
            Ok(_) => {}
            Err(e) if code_of(&e) == Some(codes::STALE_INSTANCE) => {} // a newer instance holds it
            Err(e) => debug!(service_id, mailbox, error = %e, "could not remove the old queue"),
        }
    }
}

impl fmt::Debug for MailboxPuller { … finish_non_exhaustive }

#[async_trait::async_trait]
impl ReceiverVault for ControlPlaneService { /* delegate to mailbox_key, deploy_record */ }

#[async_trait::async_trait]
impl DeferredDispatch for RouteHandler { /* delegate to RouteHandler::dispatch_deferred */ }
```

The anchor cache lives for one pass, like the sender's poll.

**8.2 `mailbox_pull/service.rs`**

```rust
#[derive(Default)]
pub(super) struct ServiceState {
    registered: Option<Registration>,
    /// The certificate the mailbox refused, and why. The service is skipped
    /// until the installed certificate differs.
    blocked_on: Option<(CertId, Note)>,
    served_checked_at: Option<Instant>,
}

struct Registration {
    mailbox: String,
    record_hex: String,   // pkarr_packet_hex of the record registered
    at: Instant,
    max_ttl_secs: u64,
    /// From the mailbox's `info`, read at register; 300 when it failed.
    /// The round budget is derived from it (C6-12).
    visibility_secs: u64,
}

#[derive(PartialEq, Eq, Clone)]
struct CertId { temporary_did: String, issued_at_secs: u64 }

/// Everything one service needs for one pass.
struct Inputs { instance: ServiceInstance, key: MailboxSecretKey,
                record: SignedEndpointInfo, mailbox: String }
```

Functions (each under 100 lines; the pseudo-code shows the order):

```rust
impl MailboxPuller {
    pub(super) async fn pull_service(&self, pass: &mut ServicePass<'_>) {
        let Some(inputs) = self.inputs(pass).await else { pass.summary.blocked += 1; return };
        self.check_served_record(pass, &inputs.mailbox).await;
        // A different mailbox shows up in `needs_register`; the control
        // plane already removed the old queue (C6-15).
        if pass.entry.needs_register(&inputs) && !self.register(pass, &inputs).await {
            return;
        }
        self.claim_rounds(pass, &inputs).await;
    }

    /// The inputs, or `None` after a note (C6-4 table, first six rows).
    async fn inputs(&self, pass: &mut ServicePass<'_>) -> Option<Inputs> {
        let Some(instance) = self.proxy.service_instance(pass.service_id) else {
            pass.note(Note::NoCertificate, ""); return None;
        };
        let cert_id = CertId::of(&instance.certificate);
        match &pass.entry.blocked_on {
            Some((blocked, why)) if *blocked == cert_id => { pass.note(*why, ""); return None }
            _ => pass.entry.blocked_on = None,
        }
        let key = match self.vault.mailbox_key(pass.service_id).await {
            Ok(Some(key)) => key,
            Ok(None) => { pass.note(Note::NoKey, ""); return None }
            Err(e) => { pass.note(Note::KeyUnreadable, &e.to_string()); return None }
        };
        let record = match self.vault.deploy_record(pass.service_id).await {
            Ok(Some(record)) => record,
            Ok(None) => { pass.note(Note::NoRecord, ""); return None }
            Err(e) => { pass.note(Note::NoRecord, &e.to_string()); return None }
        };
        let Some(mailbox) = record.info.mailbox.as_ref().map(|m| m.service_id.clone()) else {
            pass.note(Note::NoRecord, "the stored record names no mailbox"); return None;
        };
        if mailbox != pass.fact.mailbox {
            pass.note(Note::RecordAndFactDiffer, &format!("record {mailbox}, fact {}", pass.fact.mailbox));
        }
        Some(Inputs { instance, key, record, mailbox })
    }

    /// `true` when claiming may go on (C6-4 register rows). Every register
    /// of a pass goes through here, also the one after a `-32040`.
    async fn register(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) -> bool {
        let params = json!(RegisterParams { record: inputs.record.clone() });
        // C6-18: under the gate, register only while the fact is the one
        // this pass started with. The control plane changes the fact before
        // it calls `release`, so a removed queue is never registered again.
        let answer = {
            let gate = self.register_gate(pass.service_id);
            let _held = gate.lock().await;
            let current = self.registry.mailbox_of(pass.service_id).map(|f| f.mailbox);
            if current.as_deref() != Some(pass.fact.mailbox.as_str()) {
                debug!(service_id = pass.service_id, "the mailbox changed during the pass; not registering");
                pass.entry.registered = None;
                return false;
            }
            self.proxy.call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_REGISTER, params).await
        }; // the gate is released here: `learn` (and its `info` call) runs without it
        match answer {
            Ok(value) => match serde_json::from_value::<RegisterAnswer>(value) {
                Ok(answer) => { self.learn(pass, inputs, answer.limits).await; true }
                Err(e) => { debug!(…); false }
            },
            Err(e) => match code_of(&e) {
                Some(codes::BAD_RECORD) => { pass.note(Note::RecordExpired, …); true }
                Some(codes::NOT_AN_INSTANCE) => { pass.block(&inputs.instance, Note::InstanceRefused); false }
                Some(codes::STALE_INSTANCE) => { pass.block(&inputs.instance, Note::StaleInstance); false }
                Some(codes::MAILBOX_FULL) => { pass.note(Note::MailboxFull, …); false }
                _ => { debug!(…); false }
            },
        }
    }

    /// Calls `info` once (5 s; on failure `visibility_secs = 300`), records
    /// the registration, and writes `max_ttl_secs` back (C6-8).
    async fn learn(&self, pass, inputs, limits: QueueLimits);

    async fn claim_rounds(&self, pass: &mut ServicePass<'_>, inputs: &Inputs) {
        let policy = OpenPolicy { max_ttl_secs: pass.max_ttl_secs(), max_clock_skew_secs: MAX_CLOCK_SKEW_SECS };
        let mut registered_again = false;
        for _ in 0..MAX_CLAIM_ROUNDS {
            let params = json!(ClaimParams { max: Some(MAX_CLAIM_BATCH) });
            // Taken before the call, so the local window is never longer
            // than the mailbox's. The mailbox's own times are never compared
            // with this node's clock.
            let claimed_at = Instant::now();
            let answer = match self.proxy.call_mailbox(&inputs.instance, &inputs.mailbox, METHOD_CLAIM, params).await {
                Ok(v) => match serde_json::from_value::<ClaimAnswer>(v) { Ok(a) => a, Err(_) => return },
                Err(e) => match code_of(&e) {
                    Some(codes::UNKNOWN_DESTINATION) if !registered_again => {
                        pass.entry.registered = None;
                        registered_again = true;
                        if self.register(pass, inputs).await { continue } return;
                    }
                    Some(codes::NOT_AN_INSTANCE) => { pass.block(…, Note::InstanceRefused); return }
                    Some(codes::STALE_INSTANCE) => { pass.block(…, Note::StaleInstance); return }
                    _ => { debug!(…); return }
                },
            };
            let full = answer.messages.len() >= MAX_CLAIM_BATCH as usize;
            let finished = self.handle_round(pass, inputs, &answer, claimed_at, policy).await;
            if !finished || !full { return }
        }
    }

    /// Handles one claimed batch inside its visibility time (C6-12), on
    /// this node's clock only: `budget = visibility_secs - ACK_MARGIN_SECS`
    /// (at least 1 s); before each message except the first, stop when
    /// `claimed_at.elapsed() + DEFAULT_PROXY_CALL_TIMEOUT >= budget`. Send
    /// the acks so far after every `ACK_FLUSH` handled messages and at the
    /// end. `true` when every message was handled and every ack call went
    /// through.
    async fn handle_round(&self, pass, inputs, answer: &ClaimAnswer, claimed_at: Instant, policy) -> bool {
        let budget = pass.round_budget(); // from Registration::visibility_secs, else 300 s
        let mut acks = Vec::new();
        for (i, m) in answer.messages.iter().enumerate() {
            if i > 0 && claimed_at.elapsed() + DEFAULT_PROXY_CALL_TIMEOUT >= budget {
                pass.summary.not_acked += (answer.messages.len() - i) as u32;
                self.send_acks(pass, inputs, acks).await;
                return false;
            }
            if let Some(entry) = self.handle_one(pass, inputs, m, policy).await { acks.push(entry) }
            if acks.len() >= ACK_FLUSH && !self.send_acks(pass, inputs, mem::take(&mut acks)).await {
                return false;
            }
        }
        self.send_acks(pass, inputs, acks).await
    }

    /// One `ack` call (at most `ACK_FLUSH` entries, under `MAX_ACK_BATCH`).
    /// Reads every `AckOutcome` (C6-12). `false` when the call failed: the
    /// messages come back after the visibility time and the fence replays
    /// them. An empty list makes no call.
    async fn send_acks(&self, pass, inputs: &Inputs, acks: Vec<AckEntry>) -> bool;

    /// C6-11.
    async fn check_served_record(&self, pass: &mut ServicePass<'_>, mailbox: &str);
}

impl ServiceState {
    fn needs_register(&self, inputs: &Inputs) -> bool {
        match &self.registered {
            None => true,
            Some(r) => r.mailbox != inputs.mailbox
                || r.record_hex != inputs.record.pkarr_packet_hex
                || r.at.elapsed() >= REREGISTER_INTERVAL,
        }
    }
}
```

`pass.max_ttl_secs()` is the registered value when there is one, else
`pass.fact.max_ttl_secs` (persisted, so it survives a restart and a
`-32047`). `code_of(e)` is `Some(code)` for `ProxyError::Callee { code, .. }`.

**8.3 `mailbox_pull/message.rs`**

```rust
/// The ack for one claimed message, or none.
enum Handled { Ack(Receipt, Count), NoAck(String) }
enum Count { Delivered, Rejected }

async fn handle_one(&self, pass, inputs, m: &ClaimedMessage, policy) -> Option<AckEntry> {
    // let handled = self.handle(pass, inputs, m, policy, now_secs()).await;
    // Ack(receipt, count) -> Some(AckEntry { sender: m.sender.clone(), deposit_id: m.deposit_id, receipt }),
    //                        count it in summary and in the counter
    //                        substrate.mailbox_receiver.messages{result=delivered|rejected};
    // NoAck(why) -> None; summary.not_acked += 1, debug!, counter result=not_acked.
}

async fn handle(&self, pass, inputs, m: &ClaimedMessage, policy: OpenPolicy, now: u64) -> Handled {
    let me = inputs.instance.key.as_ref();
    let cert = &inputs.instance.certificate;
    let opened = match open(&inputs.key, pass.service_id, &m.deposit_id, &m.sealed, policy, now) {
        Ok(opened) => opened,
        Err(OpenError::Refused(refused)) => {
            if refused.code == RejectCode::FromFuture { warn!(…"check this node's clock"…) }
            return receipt_of(refused.receipt(me, cert, now), Count::Rejected);
        }
        Err(e) => return Handled::Ack(Receipt::unsigned_rejection(e.reject_code(), now), Count::Rejected),
    };
    if opened.sender_master_did() != m.sender { debug!(…) }
    match self.proxy.revoked_keys(opened.sender_master_did(), pass.anchors).await {
        None => return Handled::NoAck("the sender's anchor cannot be read".into()),
        Some(revoked) if revoked.iter().any(|k| k == opened.sender_instance_did()) =>
            return reject(&opened, me, cert, RejectCode::Revoked, now),
        Some(_) => {}
    }
    if opened.content.idempotency_key.is_empty() {
        return reject(&opened, me, cert, RejectCode::NotEligible, now);
    }
    let Some((_, interface)) = self.registry.lookup(pass.service_id, &opened.content.interface) else {
        return reject(&opened, me, cert, RejectCode::NotEligible, now);
    };
    if !pass.fact.accepts(&interface, &opened.content.method) {
        return reject(&opened, me, cert, RejectCode::NotEligible, now);
    }
    let Ok(caller) = delivery_caller(&opened, now) else {    // C6-6
        return Handled::NoAck("the sender key does not resolve".into());
    };
    let delivery = self.dispatch.dispatch_deferred(pass.service_id, &interface, &opened.content, &caller);
    match time::timeout(DEFAULT_PROXY_CALL_TIMEOUT, delivery).await {
        Err(_) => Handled::NoAck("the service did not answer in time".into()),
        Ok(DeferredOutcome::Succeeded) => receipt_of(opened.sign_delivered(me, cert, now), Count::Delivered),
        Ok(DeferredOutcome::Failed { code }) => {
            debug!(code, "the service answered an error; the sender gets it on the direct path");
            Handled::Ack(Receipt::unsigned_rejection(RejectCode::CallFailed, now), Count::Rejected)
        }
        Ok(DeferredOutcome::NotServed) => reject(&opened, me, cert, RejectCode::NotEligible, now),
        Ok(DeferredOutcome::NotRun(why)) => Handled::NoAck(why),
    }
}

fn reject(opened, me, cert, code, now) -> Handled { receipt_of(opened.sign_rejection(me, cert, code, now), Count::Rejected) }
fn receipt_of(r: Result<Receipt, ReceiptError>, count: Count) -> Handled { Ok → Ack, Err → NoAck + warn }
```

The `pass.fact` is the hosted-mailbox fact of this pass. The check uses the
canonical interface from `lookup`, so a sender that sealed the short-hash
form still matches.

**8.4 `mailbox_pull/notes.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Note {
    NoCertificate, InstanceRefused, StaleInstance, NoKey, KeyUnreadable,
    NoRecord, RecordAndFactDiffer, RecordExpired, MailboxFull, ServedRecordDiffers,
}
impl Note { fn reason(self) -> &'static str; fn text(self) -> &'static str }

#[derive(Default)]
pub(super) struct Notes { last: HashMap<(String, Note), Instant> }  // tokio Instant, like WarnLimiter
impl Notes {
    /// Warns and counts the first time and then once an hour for each
    /// service and note; logs at debug in between.
    pub(super) fn note(&mut self, service_id: &str, note: Note, detail: &str);
}
```

Counter: `substrate.mailbox_receiver.blocked` with label `reason =
note.reason()`. Never log a key, a certificate or a message body.

**8.5 Logging and metrics names** (write them in the developer guide,
Step 13): `substrate.mailbox_receiver.messages{result=delivered|rejected|not_acked}`,
`substrate.mailbox_receiver.blocked{reason=…}`.

### Step 9 — Wiring in the router

`route_handler.rs`:

```rust
/// The mailbox receiver of this node, or `None` in coordinator mode.
#[must_use]
pub fn mailbox_puller(&self, vault: Arc<dyn ReceiverVault>) -> Option<Arc<MailboxPuller>> {
    let proxy = self.proxy()?;
    Some(Arc::new(MailboxPuller::new(proxy, Arc::new(self.clone()), vault,
                                      self.inner.registry.clone())))
}
```

The puller holds a `RouteHandler` clone; `RouteHandlerInner` never holds
the puller, and the control plane holds only a `Weak`, so there is no
reference cycle (the shutdown hang of an earlier `Arc` cycle came from
exactly such a loop).

`connection_router.rs`:
- New field `mailbox_puller: Option<Arc<MailboxPuller>>` (init to `None`
  at `:75`).
- Before `RouteHandler::init` (`:104`): `let control_plane =
  route_handler_deps.control_plane.clone();`.
- After it:

  ```rust
  router.mailbox_puller = control_plane.and_then(|control_plane| {
      let puller = route_handler.mailbox_puller(control_plane.clone())?;
      control_plane.set_mailbox_pull(Arc::downgrade(&puller) as Weak<dyn MailboxPullControl>);
      Some(puller)
  });
  ```
- `pub fn mailbox_puller(&self) -> Option<Arc<MailboxPuller>>` beside
  `proxy()`.

### Step 10 — Substrate runtime

`crates/substrate/src/runtime/services.rs`:
- Fields `mailbox_pull_join: Option<JoinHandle<()>>` and
  `mailbox_pull_cancel: CancellationToken`, with a doc comment: raced like
  `proxy_outbox_join`; at shutdown aborted, not awaited (C6-16), because a
  pass may wait on a mailbox that is not reachable and must not keep
  pulling after the node stops.
- `init`: `None` and `CancellationToken::new()`.
- `spawn_background_workers`: after the conversation worker,

  ```rust
  self.mailbox_pull_join = connection_router.mailbox_puller().map(|puller| {
      let cancel = self.mailbox_pull_cancel.clone();
      tokio::spawn(puller.run(cancel))
  });
  ```
- `run_until_shutdown`: `let mut mailbox_pull_fut =
  pin::pin!(join_unit_or_pending(self.mailbox_pull_join.as_mut(), "mailbox receiver"));`
  and the arm `res = &mut mailbox_pull_fut => log_component_exit("mailbox receiver", res),`.
- `shutdown`: beside the proxy outbox,

  ```rust
  // Aborted, not detached: an old puller must not keep pulling with this
  // node's certificate after it stops. Unacked messages come back after
  // the mailbox's visibility time.
  self.mailbox_pull_cancel.cancel();
  if let Some(handle) = self.mailbox_pull_join.take() {
      handle.abort();
  }
  ```

No new config. No Cargo feature.

### Step 11 — SDK for tests

Done in Step 4 (`mailbox_pull_now`). No roymctl verb: the phone shell is
not roymctl, and the tests use the SDK.

### Step 12 — Tests

**12.1 Router unit tests** (`crates/router/src/mailbox_pull/tests/`).

Shared fixtures first: move `TestAnchors`, `CountingMailbox` and
`Receiver` from `proxy/tests/mailbox_helpers.rs` to a new
`crates/router/src/test_support/mailbox.rs` (`pub(crate)`, under the
existing `#[cfg(test)] mod test_support;`), and import them back in
`mailbox_helpers.rs`. Do not copy them: the duplication gate counts it.

`tests/helpers.rs` builds one node that is both the mailbox host and the
receiver host: a `ProxyRouter` (the existing proxy test helper style) with
a local `CountingMailbox` on interface `mailbox` under the node DID; a
hosted service (owner set, instance certificate issued by the test master
for `node_identity.derive_service_identity(owner, service_id)`, a
hosted-mailbox fact); a `FakeVault` (key, record, or errors on demand); a
`FakeDispatch` that records `(service_id, interface, method, caller)` and
answers a scripted `DeferredOutcome`; and `deposit(sender, content)` that
seals with `syneroym_mailbox_format::seal` and calls the mailbox `deposit`
as the sender. Tests then call `puller.pull_once()` and read the mailbox
`status` as the sender.

| File | Tests (names are the behaviour) |
| --- | --- |
| `register.rs` | `the_first_pass_registers_and_later_passes_only_claim`; `a_new_stored_record_registers_again`; `a_removed_queue_is_registered_again_in_the_same_pass`; `an_expired_record_keeps_claiming_with_the_known_time_to_live`; `the_learned_time_to_live_is_written_to_the_fact`; `a_fact_changed_by_a_deploy_is_not_overwritten`; `the_receiver_on_the_mailbox_host_presents_its_instance_proof` (the local `invoke_local` path carries `pubkey_hex` and `delegation_json`; `CountingMailbox` checks it); `a_move_to_another_mailbox_registers_there_without_unregistering` (the puller sends no `unregister` itself); `an_undeploy_during_a_pass_leaves_no_queue` (C6-18: a `FakeDispatch` that, on the first delivery, removes the fact and calls `release` the way the control plane does; the next `claim` answers `-32040`, the pass does not register again, and after the pass the mailbox has no queue for the service); `a_release_waits_for_a_register_in_flight` (C6-18: the mailbox holds a `register` until the test lets it go; a `release` started meanwhile returns only after it, and the queue is gone at the end); `a_fact_rewritten_with_the_same_mailbox_still_registers` (the write-back of C6-8, or a redeploy to the same mailbox, does not count as a change) |
| `release.rs` | `release_removes_the_queue_at_a_local_mailbox` (waiting messages read `expired` for their sender); `release_reaches_a_remote_mailbox_as_the_instance` (remote mailbox set up as in `proxy/tests/mailbox.rs::a_remote_mailbox_call_is_signed_by_the_sender_instance_and_carries_no_on_behalf`: the `MockHop` sees a preamble with the service's instance key and certificate, not the node key); `release_without_a_certificate_sends_nothing` (owner and certificate removed first: no call at all, which is why the control plane calls `release` before it removes them); `release_ignores_a_newer_instance` (`-32046`, queue still there); `release_does_not_wait_for_a_running_pass` (a pass blocked in a slow `FakeDispatch`; `release` still returns) |
| `messages.rs` | `a_message_is_delivered_as_its_sender_and_acked_delivered` (caller DID is the sealed master, proof has the sender key and certificate, no capabilities); `a_sender_certificate_that_expired_after_the_send_time_is_still_delivered` (R-4 accept case: a certificate valid for about 2 s, seal, wait until it has expired, pull: `delivered`); `a_sender_certificate_that_does_not_verify_is_rejected_unsigned` (sealed with `seal_unchecked` from the `test-support` feature); `an_error_answer_is_rejected_unsigned_as_call_failed` (and a replayed error too); `a_method_that_did_not_opt_in_is_rejected_signed`; `the_conversation_methods_are_always_accepted`; `a_short_hash_interface_matches_its_listed_name`; `an_entry_that_names_the_hash_matches_the_canonical_interface`; `an_empty_key_is_rejected_signed`; `a_revoked_sender_is_rejected_signed`; `an_unreadable_anchor_leaves_the_message_unacked`; `a_temporary_failure_is_not_acked`; `a_tcp_service_rejects_signed`; `a_message_for_another_key_is_rejected_unsigned`; `a_message_older_than_the_queue_allows_is_rejected_unsigned`; `a_message_from_the_future_is_rejected_signed` (and the sender's `Receipt::verify_signed` accepts each signed receipt and refuses `call_failed` as unsigned) |
| `rounds.rs` | `acks_are_sent_every_eight_messages` (a slow `FakeDispatch`; the mailbox shows acks before the batch ends); `a_round_stops_before_its_visibility_time_ends` (paused time; a fake whose deliveries take long enough to cross the local budget: the delivered messages are acked, the rest are not handled and come back); `the_first_message_of_a_round_is_always_handled` (a visibility time shorter than one delivery timeout); `a_mailbox_clock_far_ahead_or_behind_does_not_change_the_round` (a mailbox whose `visible_again_at_secs` is hours off this node's clock, for example by wrapping `CountingMailbox` to shift it: the round handles the same messages); `every_ack_outcome_is_read` (a message closed at the mailbox meanwhile answers `not_found`, logged, not counted as delivered) |
| `blocked.rs` | `an_expired_certificate_waits_and_then_pulls` (no cert → skip; install a valid one → delivered; this is the change doc's "expired certificate" item); `a_refused_instance_waits_for_a_new_certificate` (-32045); `a_stale_instance_stops_until_its_certificate_changes` (two certificates, the newer registers first); `a_locked_vault_skips_the_service`; `no_stored_record_skips_the_service`; `a_note_is_written_once_an_hour` (paused time); `pull_now_runs_after_a_running_pass` |

**12.2 Router integration tests** (`crates/router/tests/call_dedup_dispatch.rs`,
278 lines; add about 150): a real `RouteHandler::init` with a
`CountingService` that records the caller.

- `a_call_from_the_mailbox_after_the_same_direct_call_runs_once`:
  `dispatch_json_rpc_once` with the master as caller and key `k1`, then
  `dispatch_deferred` with a caller for the same master and `k1` →
  `Succeeded`, service invoked once (the change doc's "duplicate delivery"
  item, at the router level).
- `a_direct_retry_after_a_failed_mailbox_call_gets_the_same_error`:
  `dispatch_deferred` → `Failed { code }`, then `dispatch_json_rpc_once`
  with the same master and key answers the same error frame, service
  invoked once. This is what makes the unsigned `call_failed` safe (Q3).
- `a_call_from_the_mailbox_reaches_the_service_as_its_sender`: caller DID
  and proof as C6-6, and the call runs while the proof's certificate has
  expired (R-4: nothing on the path checks it again).
- `a_refused_fence_is_not_run` (anonymous or node-level → `NotRun`).
- `a_running_call_is_not_run_again` (first claim held → `NotRun`).

**12.3 Other crates.** Listed in Steps 2 to 5.

**12.4 Substrate end-to-end.** New file
`crates/substrate/tests/mailbox_receiver_e2e.rs` (keep under 800 lines).
Reuse `common/conversation_peer.rs`, `common/outbox.rs` and
`SubstrateNode`; dynamic ports only. Do **not** build on
`common/mailbox_receiver.rs`: it is the test double the real receiver
replaces, and the migration below deletes it. The few plain helpers from
it that the new tests need (a client that presents an instance and its
certificate, one mailbox call with the error split out) move to a new
`common/mailbox_client.rs` first. The receiver node triggers the pull with `mailbox_pull_now` from its
owner client; the sender learns the receipt on its own 60 s poll, so each
test that waits for the sender takes about a minute (as in C5).

- `a_message_to_a_stopped_peer_is_delivered_by_the_peer_when_it_starts`
  (Conversation 1:1, both nodes real: `deposited`, restart the receiver,
  pull now, the receiver's store has the message from the sender's master,
  the sender shows `delivered`).
- `a_group_push_from_the_mailbox_reaches_the_member`.
- `an_opted_in_queued_call_runs_once_and_completes_the_row` (greeter with
  `mailbox_methods`, WASM instantiation count 1, outbox empty, no dead
  letter).
- `a_queued_call_to_a_method_that_did_not_opt_in_is_dead_lettered`.
- `pull_now_is_for_the_node_owner_only`.
- `an_undeployed_service_removes_its_queue_at_a_remote_mailbox` (receiver
  and mailbox on different nodes; after the undeploy, a deposit for the
  receiver answers `unknown_destination` (`-32040`) at once, with no wait
  for a pass).

**Migrate the C5 tests (Q9).** In `mailbox_sender_e2e.rs` and
`mailbox_conversation_e2e.rs`, every test where the receiver service is
deployed on a node with the mailbox key and the double then registers
would now meet `stale_instance`. Run them first to see which fail. For
each: if the test ends with a delivery, drop the double and use the real
receiver (restart the node, pull now). If the test needs a receipt the real
receiver also makes (`not_eligible`), use the real receiver with a method
that is not listed. `mailbox_service_e2e.rs` does not use `MailboxReceiver`
(it has its own `Member` double). After the migration no test uses
`MailboxReceiver`: delete `common/mailbox_receiver.rs` and its `mod` line
in `common/mod.rs`. Plain helpers still in use already moved to
`common/mailbox_client.rs` (above). Do not leave the file behind its
`#![allow(dead_code)]`. Lower the oversized-test baselines if a file
shrinks (Mandatory Ratchet Update).

Also run `mailbox_record_e2e.rs`: it deploys services with a mailbox, so
the real puller now registers them at their mailbox (or fails to reach it
every minute). Check that its assertions still hold (for example, a
mailbox that republishes a record does not change what the test reads),
and that no test leaves a puller warning in a loop that slows it.

### Step 13 — Docs

- `docs/system-architecture.md` §Service Mailbox (`:2066-2130`): remove
  the "Envisioned" note (`:2070`); add a **Receiver side** paragraph (what
  the puller does, when it runs, eligibility, the ack rules, the delivery
  caller, `max_ttl_secs` written back, blocked states).
- `docs/system-requirements-spec.md:1084`: remove the "Envisioned" note.
- `docs/planning/traceability-matrix.md:56` (`[PLT-ASY]` / `[TOP-ROB]`):
  the text still says "deposit by senders and pull by receivers are
  Envisioned". Say that both are built, name the C6 tests as evidence, and
  keep the status `Partial` until C7 and C8 (open list, dedicated
  mailboxes).
- `docs/planning/changes/service-mailbox/change.md` Design → "Receiver
  flow": already rewritten in Step 1; check it still matches the code.
- `docs/developer-guide.md`: `mailbox_methods` in the manifest section
  (near `:440`), `--mailbox-method` (near `:412`), `security.mailbox-pull-now`,
  and that it waits for a whole pass with no time limit (C6-17), the two
  new metrics (near `:1759`, under a "mailbox receiver" heading, not the
  role), the Q6 sentence, and that undeploying a service, or moving it to
  another mailbox, removes its old queue on a best-effort basis.
- `docs/TERMINOLOGY.md`: "mailbox-eligible method".
- Change doc: R-1 to R-6 test column; R-7 "Only partly covered" sentence →
  covered by `call_dedup_dispatch.rs`; Tasks: tick "Substrate pull worker
  and injection with sender identity"; `living-docs-touched` keeps its
  list.
- `task.md` C6: mark Done with a pointer to `D-C6-*`; fix "Parallel with
  C5". `status.md`: the C6 row with evidence.

### Step 14 — Finish

- Import clean-up pass over every edited file (types by `use`, functions
  by parent module, no inline multi-`::` paths).
- `cargo dupes report` over the changed files (AGENTS.md recipe).
- Deferred backlog pass (Step 1 rows, plus anything new: the pull interval
  is a constant; pull-now waits for a whole pass).
- `mise run build:test-components` is not needed for the WIT change in
  Step 4 (host-only record); run it if the grep in Step 4 finds a guest
  binding.
- `mise run verify` in the background; all gates must pass. Then stop: do
  not commit until the owner says so.

## 5. Call sites that change (one list)

| Symbol | Change | Call sites |
| --- | --- | --- |
| `RejectCode` | `+ CallFailed` (`ALL` has 11 entries) | `mailbox_format/src/receipt.rs`, its tests; puller |
| `seal_unchecked` | public behind the `test-support` feature | router `[dev-dependencies]` |
| `is_node_level_interface` | moved from `router/src/call_dedup.rs:195` to `core/src/local_registry.rs`; method wrapper `call_dedup.rs:291` removed | `call_dedup.rs:449` (`begin`), `proxy/router.rs:548`, `proxy/saga_dispatch.rs:142`, `call_dedup/tests.rs:453-454` (moved to core tests), new `is_reserved_mailbox_interface` |
| `MailboxPullControl` | `+ release` | `mailbox_key.rs::release_mailbox_queue`, called from `lifecycle.rs::remove_deploy_registry_rows` and `commit.rs::commit_hosted_mailbox` |
| `remove_deploy_registry_rows` | reads the fact, then `forget_hosted_mailbox` moves from its end to before `release`, before `remove_owner` (C6-18) | `lifecycle.rs:442` |
| `commit_hosted_mailbox` | writes the new fact, then `release`s the old mailbox (C6-18) | `commit.rs:6` |
| `MailboxPuller` | `+ register_gates`, `+ register_gate`; `register` checks the fact under the gate (C6-18) | `mailbox_pull.rs`, `mailbox_pull/service.rs` |
| `SUPERVISOR_RESERVED_SERVICE_ID` | now refers to core `SUPERVISOR_INTERFACE` | `control_plane/src/service.rs:69` |
| `HostedMailbox` | `+ methods`, `+ accepts` | literals in Step 2; readers `call_dedup.rs::window_for` (unchanged), puller |
| `EndpointRegistry` | `+ hosted_mailboxes` | puller |
| `EndpointStorage` impls | save/load `methods` | `data_db/src/registry_store.rs`, `core/src/storage.rs` (`MockStorage`, no change: it clones) |
| `ServiceSpec`, `PlannedService` | `+ mailbox_methods` | literal list in Step 3; `compiler.rs::planned_members` |
| WIT `mailbox-credentials` | `+ methods` | `sdk/src/mapper.rs`, `sdk/src/types.rs`, control-plane tests, `mailbox_record_e2e.rs` |
| `Publication::Public` | `+ mailbox_methods` | `roymctl svc/deploy.rs:190`, `sdk/src/tests.rs:75,91,101`, `roymctl svc/tests.rs:515,538`, `substrate/tests/service_visibility_e2e.rs:294`, `mailbox_record_e2e.rs:120`, `record_signing_e2e.rs:44`, `common/mailbox_receiver.rs:159` |
| `refuse_reserved_mailbox_methods` (new, `crates/sdk`) | — | `roymctl app/deploy.rs`, `app_supervisor/src/service/verbs/submit.rs:337` |
| `check_mailbox_args` | `+ methods` | `roymctl svc/deploy.rs`, its tests |
| `validate_mailbox` | returns `Option<ValidatedMailbox>` | `deploy/admission.rs:113`, `tests/mailbox.rs` |
| `ValidatedDeploy` | `mailbox_key` + `mailbox` → `mailbox: Option<ValidatedMailbox>` | built at `deploy/admission.rs:124`; read in `deploy/commit.rs:81,101` |
| `record_hosted_mailbox` | `+ methods` | `deploy/commit.rs::commit_hosted_mailbox` |
| `ControlPlaneService` | `+ deploy_record`, `+ mailbox_pull` `OnceLock`, `+ set_mailbox_pull`, `+ wake_mailbox_pull` | `connection_router.rs`; wakes in `dispatch.rs` (`renew-cert`, `inject-kek`) and `commit.rs` |
| `frame_as_outcome` | `pub(super)` | `route_handler/deferred.rs` |
| `DepositSender` / `sender_identity` | renamed `ServiceInstance` / `service_instance`, `pub(crate)` | `proxy/mailbox.rs`, `proxy/mailbox/poll.rs`, proxy tests |
| `ProxyRouter` | `+ call_mailbox`, `+ revoked_keys`, `+ registry_client` | `deposit`, `poll_chunk`, puller |
| `dispatch_json_rpc_once` | body split into `dispatch_fenced` (no signature change) | internal |
| `RouteHandler` | `+ dispatch_deferred`, `+ mailbox_puller` | puller, `connection_router.rs` |
| `ConnectionRouter` | `+ mailbox_puller` field and getter | `substrate/src/runtime/services.rs` |
| `RuntimeServices` | `+ mailbox_pull_join` (aborted at shutdown), `+ mailbox_pull_cancel` | `services.rs` only |
| `SyneroymClient` | `+ mailbox_pull_now` | e2e tests |

## 6. What C6 does not do

- `--mailbox auto` and the registry list of open mailboxes (C7). Dedicated
  mailboxes and passports at `register` (C8).
- Narrowing the long receiver fence (Q4), and the node-key and relocation
  cases of the fence-caller row (Q5).
- Returning the service's answer (or a Conversation refusal) through the
  mailbox (non-goal, ADR-0026). An error answer reaches the sender only on
  its next direct attempt, from the receiver fence (Q3).
- Removing a queue whose mailbox cannot be reached at undeploy or
  redeploy, or for a service with no valid certificate at that moment
  (C6-15, backlog row).
- A time limit on `security.mailbox-pull-now` (C6-17, backlog row).
- Changes for C8 (`passport` in `register`, the child token in
  `ReceiverVault`, the `max_ttl_secs` cap): only a forward note in
  `task.md` (Step 1).
- Push wake-up of a sleeping phone (non-goal). `security.mailbox-pull-now`
  is the hook a shell calls.
- The rest of the change doc's integration list (offline sender, receiver
  asleep longer than 2 hours, relocation, over quota): C7 runs it after C5,
  C6 and C7. "Expired certificate" and "duplicate delivery" are covered by
  C6 at the router level (Step 12), not end to end; Step 1 writes this
  into `task.md` C7.
- A roymctl verb for pull-now.

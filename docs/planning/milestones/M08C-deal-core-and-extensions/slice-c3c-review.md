# M08C slice C3c — review

*Reader: the developer who fixes the C3c plan or code, and the project
owner. This file holds every review finding for slice C3c (parts C3c1, C3c2
and C3c3). Plan review findings are P1, P2, …; code review findings are R1, R2,
…. Ids are never renumbered.*

## Plan review round 1 — Gemini 3.8 Flash, 2026-11-23

Checked [slice-c3c-implementation-plan.md](slice-c3c-implementation-plan.md)
on branch `feat/m08c-c3c-agree-as-listed` at `99ff3dd0` (on top of `main` at
`8fafab15`). This is the main checkout, not a worktree (`git rev-parse --git-dir`
is `.git`). `git pull` was run outside the sandbox. The plan was written by
Claude Opus 5.5; this review is performed by Gemini 3.8 Flash.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan (all 1,830 lines);
task.md and status.md of M08C; the three change docs of the prompt
(roym-extensions: criteria, "First extensions", Deviations; roym-agreements:
AG-44, AG-45, AG-49 to AG-51, "Agree as listed", "Enquiry", "Deal decline",
"Progress snapshot", "Reason codes", "Deal rules", open questions 13 and 14;
roym-value-exchange: VX-54 and principle 13); earlier review files
(slice-c1-review.md, slice-c2-review.md, slice-c3a-review.md, slice-c3b-review.md).
task.md's C3c section names the same three change docs as the prompt, so no
change doc is missing from the list. There is no earlier review round for C3c,
so no previous items needed re-checking.

**Summary: 9 findings: 0 blocker, 3 major, 6 minor.**

### What was checked and found correct

- **The driver and wake execution model (D-C3c-1, D-C3c-2).** The wake runs as
  node-local work. In both the native build (`factory.rs:224`, `host.rs:712`)
  and WASM sandbox (`capabilities_services.rs:103`), a local dispatch sets
  `InvocationOrigin::Local`, which evaluates to `CallerOrigin::Internal`.
  `admit::require_internal` admits `CallerOrigin::Internal` with no session.
  `events.wit` provides `read` with cursor/reload semantics, and
  `conversation.message` is announced on each visible message
  (`crates/conversation/src/lib.rs:184`, `host_impl.rs:353`).
- **Sibling dependencies and DAG (Q5, section 5).** `catalog` depends only on
  `profile` (`roym.toml:67`). Adding `catalog` to `transaction.depends_on`
  (`roym.toml:82`) produces `transaction -> [conversation, scheduling, catalog]`
  and introduces no DAG cycle. `roym_deps` lint in `xtask` checks Cargo
  dependencies between sibling crates (which remain clean; sibling calls use the
  universal proxy).
- **The seven substrate sync count assertion sites (D-C3c-4, point 10).**
  Checked each in the codebase: `common/roym_flow.rs:147` (`filed: 1`),
  `roym_booking_e2e.rs:239`, `:240` (`filed: 1`), and
  `roym_transaction_e2e.rs:164` (`filed: 1`), `:230` (`filed: 1`),
  `:254` (`countersigned: 1`), `:567` (`countersigned: 1`). Asserting the
  resulting state rather than the sync count prevents race flakiness once a
  background driver is active.
- **File line counts in `transaction` (section 7.8).** Verified against `main`
  at `8fafab15`: `sync.rs` 759, `backup.rs` 785, `decide.rs` 724, `app.rs` 710,
  `backup/sections.rs` 703, `quote_ops.rs` 695. Placing new code in new sibling
  files (`drive.rs`, `as_listed_ops.rs`, `quote_ops/issue.rs`,
  `backup/as_listed.rs`) is necessary to prevent tripping the 800-line cap
  (see P4 for `sync.rs`).
- **Method contract count (section 6.1 Step 4, section 6.2 Step 4).**
  `crates/roym_web/tests/method_contract/contract.rs:83` asserts `table.len() == 44`.
  The plan correctly tracks the increment to 45 in C3c1 (`transaction.drive`)
  and 47 in C3c2 (`quote.from-listing`, `deal.as-listed-status`).
- **Change doc lines and criteria mapping (section 3).** Criteria mapping is
  accurate: AG-44, AG-45, AG-49, AG-50, AG-51 to `roym-agreements`; VX-54 to
  `roym-value-exchange`; EX-18 to `roym-extensions`.
- **Pure mapping in `terms_from_listing` (D-C3c-6).** The mapping table accurately
  constructs `c1` (owner fulfilment) and `c2` (counterparty currency) with
  `DealRole` positions, maps `PaymentTiming` to `c1.after` or `c2.after`,
  formats currency amounts cleanly, and ensures all localized texts (`scope`,
  `c1.description`, `c2.description`, `cancellation_terms`, `refund_terms`,
  `dispute_path`) share the listing's base language and exact translation
  languages set (AG-39).
- **Idempotency and fence guarantees (D-C3c-12, D-C3c-17).** The fence
  `answer:<enquiry_record_id>` is created before signing on the owner's node,
  binding `quote_id` and guaranteeing at most one quote per tap. The fence
  `autoaccept:<enquiry_record_id>` on the counterparty's node binds
  `quote_record_id` and guarantees at most one acceptance per tap.
- **"How this is shown to an end user" (section 1).** Present immediately after
  the paragraph summary. Contains simple user-facing bullets for C3c1, C3c2, and
  C3c3, followed by developer verification commands for each part.
- **Next free ADR number (Q3, section 6.1 Step 1).** Checked `docs/decisions/`:
  the highest numbered ADR is `0030-signed-message-origins.md`.
  `0031-node-local-wake.md` is the next free ADR number.

### View on the owner questions

The owner has not answered Q1 to Q8 yet. My view on each:

- **Q1 (Split C3c).** Agree with **(a)** (three PRs: C3c1 driver, C3c2 owner's
  side, C3c3 counterparty's side). C3c1 is platform infrastructure with its own
  reviewer angle. C3c2 builds the owner side, terms mapping, listing fields, and
  manual acceptance path. C3c3 adds automatic acceptance only after the owner
  side is proven and working.
- **Q2 (The driver).** Agree with **(a)** (node-local wake declared in the
  manifest, run by the substrate). It functions for both WASM and native builds,
  requires no supervisor, introduces no forbidden `conversation -> transaction`
  cycle, and enables background deal progression for tests and production.
- **Q3 (ADR for the wake).** Agree with **(a)** (yes, write ADR-0031 in C3c1
  step 1). The wake adds a new host-to-guest execution mechanism. Documenting
  its constraints and boundaries in a permanent ADR conforms to AGENTS.md.
- **Q4 (Slot offerings: picking a slot before tap).** Agree with **(a)** (build
  the slot path in records and backend services, but Hub shows "Ask" alone on
  slot offerings for now). Querying slots across nodes requires a wire-reachable
  scheduling interface, which was intentionally deferred out of M08C (C5
  deferred to M08D).
- **Q5 (`transaction` depends on `catalog`).** Agree with **(a)** (add `catalog`
  back to `transaction.depends_on`). Verified: `catalog` depends only on
  `profile`, creating no DAG cycle. It allows a clean sibling `listing.get` call.
- **Q6 (Instant cap race).** Agree with **(a)** (accept passing by at most one).
  The cap is an abuse guard against unbounded open commitments, not an inventory
  or financial ledger count. Heavy coordination overhead is unwarranted.
- **Q7 (Demo).** Agree with **(a)** (C3c1 `- none`, C3c2 `- none`, C3c3 extend
  story 50 with a deal agreed as listed). Story 50 already has Bob publishing a
  listing and Alice searching for it. Extending it in C3c3 to demonstrate
  instant agreement as listed provides comprehensive user-visible evidence.
- **Q8 (Removed slot declined with hook code).** Agree with **(a)** (owner node
  declines enquiry with the hook's error code). It promptly informs the
  counterparty in their own language and frees any expectations immediately.

### Findings

- [ ] **P1 — major.** *Where:* D-C3c-2 (The wake worker), `crates/substrate/src/runtime/wake.rs`.
  *Problem:* In D-C3c-2, `WakeWorker` spawns a call to `proxy.invoke(ProxyRequest { ... })`.
  In `crates/router/src/proxy/router.rs` (`dispatch_call`), if
  `self.registry.lookup(&req.target_service, &req.interface)` returns `None`,
  `ProxyRouter` treats the call as remote:
  `None => self.invoke_remote(req).await`.
  As documented in `control_plane/src/service/orchestration/lifecycle.rs:625`:
  "A miss in the local endpoint registry reads as 'the target lives somewhere else'
  and resolves it through the community registry instead -- so without this, a
  schedule naming a service this node does not host, or an interface the deployed
  component does not export, turns into an outbound call."
  A wake is strictly node-local. If `WakeWorker` dispatches via `proxy.invoke`
  without verifying that the service has a local endpoint registered, a wake
  for a service that is stopping, redeploying, or missing can attempt an outbound
  network call via the community registry.
  *Failure case:* When an app service is redeploying or stopping, `wake_targets`
  still contains the target while the endpoint registry entry is momentarily
  absent. `proxy.invoke` attempts remote resolution, emitting unexpected outbound
  network requests or authentication errors.
  *Fix:* In `WakeWorker`, check that the service has a local endpoint via
  `self.registry.lookup(&service_id, &spec.interface).is_some()` before dispatching,
  or invoke through a local-only proxy method, matching the invariant in `lifecycle.rs:625`.
  *Status:* Open

- [ ] **P2 — major.** *Where:* Section 5 (table under D-C3c-1), Step 3 of C3c1 (section 6.1).
  *Problem:* The plan specifies adding `wake_json` to `set_deploy_facts` on
  `crates/core/src/local_registry.rs:470`. However, `set_deploy_facts` delegates
  to `RegistryStorage::save_deploy_facts`.
  The plan omits:
  1. `crates/core/src/storage.rs`: Updating the `RegistryStorage` trait definition
     of `save_deploy_facts`.
  2. `crates/data_db/src/registry_store.rs`: Updating `save_deploy_facts` SQL query
     (`INSERT INTO service_deploy_facts ...`) and adding the idempotent migration:
     `ALTER TABLE service_deploy_facts ADD COLUMN wake_json TEXT`.
  3. Mock storage implementations in `crates/control_plane/src/service/orchestration/tests/failing_storage.rs`
     and `crates/router/tests/service_ownership.rs`, as well as `data_db` tests.
  *Failure case:* Adding a parameter to `set_deploy_facts` breaks the build of
  `crates/core`, `crates/data_db`, and multiple test binaries across `control_plane`
  and `router`. At runtime, missing the SQLite column migration would cause SQL
  syntax errors during deploy fact recording.
  *Fix:* Update the D-C3c-1 table and C3c1 Step 3 to include `crates/core/src/storage.rs`,
  `crates/data_db/src/registry_store.rs` (with the schema migration), and the mock
  implementations in `failing_storage.rs` and `service_ownership.rs`.
  *Status:* Open

- [ ] **P3 — major.** *Where:* Section 6.2 (C3c2 Step 1 vs Step 3), D-C3c-10.
  *Problem:* In D-C3c-10, `AgreementProgressPayload` gains `pub formed_as: FormedAs`
  as a required field with no default.
  Section 6.2 Step 1 modifies `roym_core` types and states:
  "Each step builds, and its tests pass, before the next starts. Step 1 — Core types (roym_core, pure): ... Every caller of the changed types compiles: fix the roym_core tests and the fixtures."
  However, `AgreementProgressPayload` is directly constructed as struct literals
  across over a dozen test files in `crates/roym_transaction/` (`tests/cancel.rs`,
  `tests/decide.rs`, `tests/restore.rs`, `tests/settle.rs`, `tests/time.rs`,
  `tests/commitment_failure.rs`, `sync/tests.rs`), `presentation/tests/`, and
  `progress_ops.rs`.
  If Step 1 updates `AgreementProgressPayload` in `roym_core` without touching
  `roym_transaction`, `cargo check -p syneroym-roym-transaction` and
  `cargo check --workspace` will fail to compile. Step 2 (`catalog` and
  `directory`) also depends on `roym_core`, leaving the workspace broken until
  Step 3.
  *Failure case:* C3c2 Step 1 cannot build cleanly workspace-wide, breaking the
  step-by-step build rule.
  *Fix:* Explicitly note in C3c2 Step 1 that struct literal instantiations of
  `AgreementProgressPayload` across `roym_transaction` tests must be updated
  (e.g., passing `formed_as: FormedAs::Quote`) so the entire workspace compiles
  at Step 1, before the runtime snapshot logic is wired in Step 3.
  *Status:* Open

- [ ] **P4 — minor.** *Where:* Section 7.8 (Size and time risks), C3c1 Step 4, C3c2 Step 4, C3c3 Step 1.
  *Problem:* `crates/roym_transaction/src/app/sync.rs` is at **759 lines** (41
  lines below the hard 800-line cap).
  Section 7.8 estimates "`sync.rs` (759) gets two calls (+6)".
  However, `sync.rs` houses both `file_enquiry_card` (lines 475-523) and
  `file_quote_card` (lines 525-620). Across C3c1 (`sync_direct` split and error
  handling), C3c2 (wiring `answer_as_listed` in `file_enquiry_card`), and C3c3
  (wiring `auto_accept` in `file_quote_card`), the added logic and error branches
  are at high risk of breaching the 800-line limit.
  *Failure case:* `sync.rs` exceeds 800 lines during C3c2 or C3c3 implementation,
  failing `cargo xtask check-file-lengths`.
  *Fix:* Proactively extract `file_enquiry_card` and `file_quote_card` into
  sibling files under `crates/roym_transaction/src/app/sync/` (e.g., `sync/cards.rs`
  or `sync/enquiry_card.rs` and `sync/quote_card.rs`), matching siblings like
  `sync/confirm.rs` and `sync/receipts.rs`. This frees ~140 lines in `sync.rs`.
  *Status:* Open

- [ ] **P5 — minor.** *Where:* D-C3c-2, line 733.
  *Problem:* The plan states:
  "`record` maps a service id to its app instance, `event_bus.rs:151`. Add a small
  read-only accessor there if none exists; do not copy the map."
  In `crates/core/src/event_bus.rs:122`, the accessor already exists:
  ```rust
  pub fn app_instance_of(&self, service_id: &str) -> Option<String> {
      self.registry.app_context_of(service_id).map(|(app_instance, _)| app_instance)
  }
  ```
  *Failure case:* Minor confusion during implementation looking for where to add
  the accessor.
  *Fix:* Note that `app_instance_of` already exists on `EventBus`.
  *Status:* Open

- [ ] **P6 — minor.** *Where:* D-C3c-2 (The wake worker).
  *Problem:* In D-C3c-2, the worker loop runs:
  ```text
  state.running = true; state.last_run_at = now
  spawn: proxy.invoke(...)
         then state.running = false
  ```
  If the spawned task panics or is cancelled before setting `state.running = false`,
  `state.running` will remain `true` indefinitely, causing all future wakes for
  that target to be skipped (`if running: skip`).
  *Failure case:* An unhandled panic in `proxy.invoke` permanently halts wakes for
  that service on that node.
  *Fix:* Guard `state.running` with an RAII guard or `scopeguard` / `defer` pattern
  so `running` is reset to `false` even on panic or task termination.
  *Status:* Open

- [ ] **P7 — minor.** *Where:* D-C3c-1 (`transaction.drive`), lines 787-800.
  *Problem:* `drive_at` states:
  "take at most MAX_DRIVE_CONVERSATIONS from DRIVE_PENDING, oldest `since` first".
  In the Roym data layer, listing or filtering rows sorted by a field requires an
  index. If `DRIVE_PENDING` is queried sorted by `since`, it must be registered
  in `ensure_collections` with an index: `idx("since", IndexType::Numeric)`.
  *Failure case:* Querying `DRIVE_PENDING` sorted by `since` fails at runtime with
  an unindexed query error.
  *Fix:* Explicitly state in D-C3c-1 and C3c1 Step 4 that `DRIVE_PENDING` is
  registered in `ensure_collections` with a numeric index on `since`.
  *Status:* Open

- [ ] **P8 — minor.** *Where:* Section 2.2 point 8, and D-C3c-7.
  *Problem:* The plan cites `messages.ts:783` as "the quote form's prefill from the enquiry".
  `messages.ts:783` is `openQuoteForm`, which constructs a quote form without
  prefilling from enquiry description. The enquiry form that currently enforces
  required description is `openRequestForm` at `messages.ts:428-475`.
  *Failure case:* Developer checking line 783 will find quote form code rather than
  the enquiry description code.
  *Fix:* Update the citation from `messages.ts:783` to `messages.ts:428-475`.
  *Status:* Open

- [ ] **P9 — minor.** *Where:* D-C3c-6 (`terms_from_listing`).
  *Problem:* `ListingPayload::validate` checks that `payment.payee.len() <= MAX_PAYEE_LEN`,
  but does not forbid an empty string (`payee: ""`). However, `DealTerms::validate`
  strictly requires non-empty `payee` (`if payee.trim().is_empty() { return Err(DealError::PayeeEmpty); }`).
  If a listing with an empty payee is evaluated by `terms_from_listing`, it would
  fail `DealTerms::validate` and return `NotComplete::Invalid("payee-empty")`
  rather than an orderly refusal code like `PriceNotFixed`.
  *Failure case:* A listing with an empty payee string returns an unexpected
  `NotComplete::Invalid` mapping error rather than a predictable refusal code.
  *Fix:* In `terms_from_listing` (and `as_listed_ready`), verify that
  `payment.payee.trim()` is not empty before mapping, returning
  `NotComplete::PriceNotFixed` if it is empty.
  *Status:* Open

## Plan review round 2 — Claude Sonnet 5.5, 2026-11-23

Checked [slice-c3c-implementation-plan.md](slice-c3c-implementation-plan.md)
on branch `feat/m08c-c3c-agree-as-listed` at `420a2ae1` (`main` is at
`8fafab15`). This is the main checkout, not a worktree (`git rev-parse
--git-dir` is `.git`). `git pull`: already up to date. The plan was written
by Claude Opus 5.5. Round 1 was by Gemini 3.8 Flash. The prompt had two
`ROUND` and two `AUTHOR` values; I used round 2 (round 1 already exists)
and the plan author named in `status.md`.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan (all 1,830 lines);
task.md and status.md of M08C; the three change docs of the prompt
(roym-agreements: "Agree as listed" `:220` to `:322`, the listing-fields
table, "Instant has limits"; roym-value-exchange: VX-54; roym-extensions:
EX-18); the round 1 section above. task.md's C3c section names the same
three change docs as the prompt, so no change doc is missing from the list.

**Summary of round 2: 8 new findings: 0 blocker, 3 major, 5 minor.**
Together with round 1, 17 items are open (0 blocker, 6 major, 11 minor).

### Round 1 items, checked again against the code

Round 1 has no item marked Fixed or Rejected, so there was nothing to
confirm as fixed. I re-checked the validity of each open item:

- **P1: valid.** `ProxyRouter::dispatch_call`
  ([router.rs:524](../../../../crates/router/src/proxy/router.rs#L524))
  sends a registry miss to `invoke_remote`. `run_scheduled_impl` guards
  against this on purpose
  ([lifecycle.rs:625](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L625)).
  The wake worker must do the same check.
- **P2: valid, and incomplete.** The trait (`storage.rs:91`), the in-memory
  store (`storage.rs:263`), the SQL (`registry_store.rs:480`) and the two
  mocks are as P1 says. The table has an `ALTER TABLE ... ADD COLUMN`
  precedent for its later columns
  ([registry_store.rs:150](../../../../crates/data_db/src/registry_store.rs#L150)),
  with a test (`registry_store/tests.rs:53`). Follow that precedent for
  `wake_json`; a node database that already exists would fail every deploy
  without it. P10 below adds the caller that P2 misses.
- **P3: valid, and wider than written.** `progress::open` is called in
  production code (`decide.rs:620`, in the `handle_not_formed` path, and the
  `finalize_agreed_deal` path), not only in tests. The struct literals are in
  `roym_core` (`progress.rs:198`, the `presentation` tests, `progress/tests.rs`,
  `record/tests/wire_words.rs`) and in `roym_transaction` tests
  (`sync/tests.rs`, `tests/decide.rs`, `tests/restore.rs`). The same problem holds for the other two Step 1 type
  changes: `EnquiryPayload` has 9 literals and `DealDeclinePayload` 10 across
  the workspace, and `description` becomes `Option` (every reader changes).
  The fix in P3 should name all three types, not only `formed_as`.
- **P4: valid.** `sync.rs` is 759 lines now. The `sync_direct` split
  (a typed error and a wrapper) is about 25 lines on its own, so 759 + 25 +
  2 calls is within a few lines of 800 before any error branch.
- **P5: valid.** `EventBus::app_instance_of` exists
  ([event_bus.rs:124](../../../../crates/core/src/event_bus.rs#L124)); the
  plan should use it. (Round 1 wrote `:122`.)
- **P6: valid.** A panic in the spawned task leaves `running = true`. Also
  keep the `JoinHandle` and treat a finished-with-error handle as "not
  running".
- **P7: partly wrong.** The data layer's `query` has no order option. Rows
  come back in `id` order, and `$sort` exists only in `aggregate`
  ([data-layer.wit:144](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L144)).
  So "oldest `since` first" is a sort in memory, as `agreement_ops.rs:515`
  and `quote_ops.rs:586` do today, and no index on `since` is needed for it.
  What the plan does miss is smaller: the three new collections (`DRIVE_STATE`,
  `DRIVE_PENDING`, `as_listed`) must be created in `ensure_collections`
  ([app.rs:335](../../../../crates/roym_transaction/src/app.rs#L335)), and
  the plan should say that `drive` sorts the pending rows in memory. Keep
  the item open with this smaller fix.
- **P8: valid.** `openQuoteForm` (`messages.ts:783`) reads only
  `reqCard.record_id` (`:915`); it never reads the enquiry's description.
  The required-description check is in `openRequestForm`
  (`messages.ts:461`). D-C3c-7's `git grep` will find the real readers.
- **P9: valid.** `listing.rs:348` checks only the payee length;
  `deal.rs:382` refuses an empty payee with `PayeeEmpty`.

### What was checked and found correct (new in this round)

- **Event bus semantics match `drive`.** `EventBus::read` with `after =
  None` returns the head with no events; a run id change, a cursor past the
  head or a cursor older than the window gives `Reload`
  ([event_bus.rs:189](../../../../crates/core/src/event_bus.rs#L189)). The
  window is 10,000 events or 24 hours, and a read is capped at 500
  (`event_bus.rs:15`). The plan's `DRIVE_EVENT_READ = 500` and its reload
  rescan match this.
- **Both worlds can read events.** `AppEvents` is implemented for the WASM
  guest host (`guest.rs:462`) and the native host (`host.rs:723`), and
  `roym_transaction/wit/world.wit:19` already imports `syneroym:events`.
- **The signing path needs no session.** `quote.set`, `accept_at` and the
  countersign use `signing::person_principal`
  ([signing.rs:192](../../../../crates/roym_core/src/signing.rs#L192));
  `require_internal` admits `CallerOrigin::Internal`
  ([admit.rs:20](../../../../crates/roym_core/src/admit.rs#L20)).
- **`listing.get` has what the owner's answer needs.** It returns the
  envelope, record id and status, and `None` for an unknown id
  ([listing_ops.rs:504](../../../../crates/roym_catalog/src/app/listing_ops.rs#L504)).
- **The card cap bounds one conversation.** `MAX_CARDS_PER_CONVERSATION` is
  2,000 (`transaction.rs:85`), and `SYNC_WINDOW` is 500
  (`transaction.rs:82`). Both matter for P12 and P13.
- **The method-contract count is 44** now
  ([contract.rs:83](../../../../crates/roym_web/tests/method_contract/contract.rs#L83)).
- **Line citations.** I checked 55 `file:line` links in the plan. 53 are
  right; two are off (P17).
- **ADR number.** `0030-signed-message-origins.md` is the newest ADR, so
  `0031` is free.
- **Criteria and change docs.** AG-44, AG-45, AG-49 to AG-51 belong to
  roym-agreements, VX-54 to roym-value-exchange and EX-18 to roym-extensions.
  Section 3 names each correctly.
- **"How this is shown to an end user"** is right after the summary
  paragraph, with the plain bullets and the developer commands for each part.
  Both builds: the plan covers the WASM and native builds for the wake, and
  parity. P11 is the one place the native build is missed.

### View on the owner questions

The owner has not answered Q1 to Q8. My view:

- **Q1 (split).** Agree with **(a)**, three PRs. Round 1 gave the reasons. I
  add one: the C3c1 platform work is larger than "about 450 lines" once
  P10, P14 and P15 are counted, which is one more reason to review it on its
  own.
- **Q2 (the driver).** Agree with **(a)**, the node-local wake. It is the
  only option that runs in both builds and in the two-node tests. P1, P6, P10
  and P15 are what makes it safe; none of them changes the choice.
- **Q3 (ADR).** Agree with **(a)**. The ADR should also say how a wake
  avoids the outbound path (P1) and that a wake ignores `on_events` for a
  service that is in no app (P15).
- **Q4 (slot offerings).** Agree with **(a)**.
- **Q5 (`transaction` depends on `catalog`).** Agree with **(a)**, but only
  with P11 fixed: the substrate's native bindings of `transaction` must get
  the same edge, or the linked build cannot call `listing.get`.
- **Q6 (instant cap).** **Do not accept (a) as written.** Its premise is
  wrong: the cap can be passed by many, not by one (P12). With the P12 fix
  (count quotes that are on their way), the remaining race is a real
  "at most one" and (a) is then fine. Without the fix, the choice is
  between (b) and the P12 fix; I prefer the P12 fix, because it is about 40
  lines and (b)'s fence needs a release pass.
- **Q7 (demo).** Agree with **(a)**.
- **Q8 (removed slot).** Agree with **(a)**, once P16 is settled: the
  decline needs the prepare hook to run on the owner's node, and the plan's
  `issue_quote` as written would skip it.

### Findings

- [ ] **P10 — major.** *Where:* D-C3c-1, the table row for
  `local_registry.rs:470`; C3c1 Step 3.
  *Problem:* Round 1's P2 lists the places that store the new column. It
  misses a caller that re-writes the deploy facts. After a binding push,
  `clear_deploy_hash_after_binding_push`
  ([lifecycle.rs:193](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L193))
  reads the facts as the 4-tuple `DeployFacts`
  ([storage.rs:18](../../../../crates/core/src/storage.rs#L18)), throws the
  hash away with `_`, and calls `set_deploy_facts` again with the other
  fields. The plan adds a fifth value (`wake_json`). The compiler stops this
  call only if the tuple type changes; a fix that writes `..` and passes
  `None` compiles and silently removes the wake of the service. The tuple is
  also matched by position in `admission.rs:198` (`(_, _, hash, _)`),
  `status.rs:34` (`f.3`) and `status.rs:335`; the patterns with `..`
  (`lifecycle.rs:541`, `orchestration.rs:315`, `tests/deploy.rs:1032,1067`)
  still compile, which is why only the re-stamp is dangerous. The in-memory
  cache (`LocalRegistry.service_deploy_facts`, `local_registry.rs:132`) is
  what `wake_targets()` would read, so it needs the field too. About ten test
  calls pass `None, None, None` by position (`tests/probes.rs`,
  `tests/deploy.rs` and `tests/proxy_queue.rs`).
  *Failure case:* A Roym app is deployed with the wake. The supervisor or
  the operator pushes a binding. The wake of `transaction` is gone from the
  registry; no card is driven until the next full deploy, and no test fails.
  *Fix:* In C3c1 Step 3, first replace the `DeployFacts` tuple with a named
  struct (`service_type`, `health_check_json`, `manifest_hash`,
  `visibility`, `wake_json`) and make `set_deploy_facts` take it. Then the
  re-stamp copies every field except the hash. Add the test
  `a_binding_push_keeps_the_wake`. Name `lifecycle.rs:193` in the step, next
  to the `git grep -n set_deploy_facts` the plan already asks for.
  *Status:* Open

- [ ] **P11 — major.** *Where:* Q5, section 4 ("`transaction`'s `depends_on`
  becomes ..."), C3c2 Step 4.
  *Problem:* The plan adds `catalog` to `depends_on` in `roym.toml`. That
  edge reaches the WASM build, because `apply_plan` binds the dependencies
  from the manifest. The linked (native) build does not read `roym.toml` for
  bindings. `wire_roym_topology` saves one binding per edge by hand: the
  `transaction` block saves `conversation` and `scheduling` only
  ([roym.rs:565](../../../../crates/substrate/src/runtime/roym.rs#L565)).
  Nothing in the plan changes it. The two tests that look like guards do not
  catch this: `every_hook_service_is_a_dependency_of_transaction`
  ([services/tests.rs:227](../../../../crates/roym_core/src/services/tests.rs#L227))
  checks only the hook services of extensions (`scheduling`), and
  `manifest_depends_on_equals_siblings` (`router.rs:209`) only checks that
  `conversation` is listed. The parity harness binds every sibling to every
  service (`populate_inventory`, `helpers.rs:841`), so parity passes too, and
  the two-node tests use the WASM build.
  *Failure case:* C3c2 merges green. On a node that runs the linked build
  (`roym` feature), `call_sibling(CATALOG, "listing.get")` from `transaction`
  has no binding and fails. Every as-listed enquiry then waits for the owner
  as if the offering did not exist, and no test shows it.
  *Fix:* In C3c2 Step 4, add the `catalog` binding in the `transaction` block
  of `wire_roym_topology`. Replace the hook-only test with one that checks,
  for every service, that each name in its manifest `depends_on` has a
  `save_binding` for that service in `roym.rs`. Add one native test in
  `crates/substrate` that calls `listing.get` from `transaction` in the
  linked build.
  *Status:* Open

- [ ] **P12 — major.** *Where:* Q6; D-C3c-12 ("Open" for the limits, and the
  `Waiting(CapReached)` and `Waiting(CounterpartyHoldsOne)` checks); section
  3 row AG-45.
  *Problem:* The two instant limits are checked when the owner's node
  *answers*, but "open" counts only agreements that are already *formed*
  (`is_open_instant`; the agreements doc says the same, `:313`). A quote
  that was answered and not yet accepted is not counted. Forming an agreement
  takes at least a round trip: the quote goes to the counterparty, its node
  signs the acceptance, the owner's node (the writer) decides. In that time
  any number of other taps are answered. Q6 says the cap "is passed by at
  most one". That is true only for two syncs that race. It is not true for
  taps that follow each other. With C3c3 the counterparty accepts each quote
  by itself, so every quote becomes an agreement.
  *Failure case:* An instant offering has `instant_cap` 20 and is
  `not_limited`. One counterparty taps "Agree as listed" 30 times in a few
  seconds (each tap is a new enquiry; `MAX_CARDS_PER_CONVERSATION` allows
  2,000 cards). Each answer sees 0 open agreements, so all 30 get a signed
  quote with no person present, and all 30 are accepted. The owner holds 30
  agreements with one counterparty on an offering with cap 20 and "one per
  counterparty". The same happens with 30 counterparties.
  *Fix:* Count what is on its way. In `answer_as_listed`'s driver branch,
  compute `in_flight_instant(listing)`: `answer:` rows with `formed_as =
  AsListedInstant` and state `answered`, whose quote has no `decision:` fence
  yet and is not expired. The two limits then use `open agreements +
  in_flight quotes` (and the same for the counterparty). Say in Q6 that the
  race left is the one between two syncs. A quote that is never accepted
  holds a place until it expires (up to 7 days); the owner's Hub shows it in
  `deal.as-listed-status`, and the owner can confirm or let it expire. Name
  that trade-off for the owner. Add tests
  `many_taps_before_any_acceptance_make_only_cap_quotes` and
  `one_counterparty_tapping_twice_gets_one_instant_quote`. Record a
  Deviation for the agreements doc's definition of "open".
  *Status:* Open

- [ ] **P13 — minor.** *Where:* `drive_at` pseudo-code (D-C3c-1,
  `drive`), "The bound".
  *Problem:* `drive` deletes a conversation from `DRIVE_PENDING` when
  `sync_direct` returns `Ok`. Two cases lose work:
  1. A sync reads at most `SYNC_WINDOW` (500) messages
     ([sync.rs:161](../../../../crates/roym_transaction/src/app/sync.rs#L161)),
     and `SyncOutput` has no "there is more" flag. A conversation with more
     than 500 new messages (a node that was offline, then got a mailbox
     delivery or a restore) is synced once, deleted from pending, and the
     rest wait for the next message event or a Hub open.
  2. `transaction.drive` stays routed, so a Hub call can run beside the
     wake. Call B puts conversation C back (a new event) after call A synced
     it and before A deletes it. A's delete then removes B's mark.
  *Failure case:* (1) A quote is message 501. It is never filed on a node
  that nobody opens. (2) A first enquiry in a conversation with no
  agreement is not driven; no timed work covers it.
  *Fix:* (1) Add `more: bool` to `SyncOutput` (true when `scanned ==
  SYNC_WINDOW`) and keep the row pending when it is set. (2) Delete a
  pending row only if its `since` is unchanged (compare-and-delete), or keep
  `transaction.drive` out of the Hub's reach. Add a test for each.
  *Status:* Open

- [ ] **P14 — minor.** *Where:* D-C3c-1, "Timed work" and "The bound"; D-C3c-2.
  *Problem:* The bound is "8 conversations per call", but the cost of one
  call is not set by that number. Each `sync` also runs
  `recover_deciding_rows` for every agreement of the node and
  `tick_conversation_at` ([sync.rs:133](../../../../crates/roym_transaction/src/app/sync.rs#L133)),
  so 8 syncs repeat the node-wide recovery 8 times. And the timed work is
  spread round-robin, 8 conversations per call, but the worker calls again
  only after an event or after `every_secs` (30 s). The worker never reads
  `DriveOutput.pending`.
  *Failure case:* A node with 400 open agreements needs 50 calls to visit
  each once: about 25 minutes at one call per 30 seconds. A quote expiry, a
  release retry or a consent expiry waits that long, while the plan says
  deals move "within a few seconds". The stop-rule measurement (8
  conversations, one card each) does not show this.
  *Fix:* Let the worker read `pending` from the reply and run again on the
  next 1-second loop while `pending > 0` (still one call at a time). Run
  `recover_deciding_rows` once per `drive`, not once per conversation, or
  say why not. Extend the stop-rule measurement to 8 conversations on a node
  with 200 agreements.
  *Status:* Open

- [ ] **P15 — minor.** *Where:* D-C3c-1 table (`services.rs:236`), D-C3c-2.
  *Problem:* The plan does not say how the worker gets its three handles.
  `RuntimeServices` holds neither the event bus nor the endpoint registry;
  the event bus is made in `router.rs:472` and lives in `SharedNodeHandles`.
  `connection_router.proxy()` is an `Option` (it is absent in coordinator
  mode, `connection_router.rs:135`). `spawn_background_workers` stores
  `JoinHandle`s that `run_until_shutdown` races in a `select!`; the plan says
  the worker "never fails the substrate" but not what a finished handle does
  there. Also, `EventBus::record` refuses a service that is in no app
  (`EventError::NoAppInstance`, `event_bus.rs:151`), so a standalone
  service with `on_events` can never be woken by an event;
  `WakeSpec::validate` cannot know this.
  *Failure case:* The implementer finds no event bus in `RuntimeServices`
  and adds a second one, or the worker panics on a node with no proxy, or a
  standalone deploy with `on_events` waits for the 30-second timer and
  nobody sees why.
  *Fix:* Name the plumbing: pass `shared.event_bus()` and the registry to
  `RuntimeServices` (or build the worker where `shared` is in scope), skip
  the worker when the proxy is `None`, and say how the handle is raced.
  State in D-C3c-2 and in the ADR that `on_events` of a service with no app
  context is ignored and logged once.
  *Status:* Open

- [ ] **P16 — minor.** *Where:* D-C3c-12 (`issue_quote`), D-C3c-6, Q8.
  *Problem:* `terms_from_listing` returns a finished `DealTerms`. `issue_quote`
  is described as "the body of `set_at` from `prepare_terms` on". But
  `prepare_terms` takes a `DealTermsInput`
  ([quote_ops.rs:139](../../../../crates/roym_transaction/src/app/quote_ops.rs#L139);
  the type is in `quote_ops/input.rs:14`, with `into_deal_terms`), and it is
  the function that runs the extension's prepare hook and sets the slot
  window. The plan never says how a `DealTerms` becomes a `DealTermsInput`
  again. If `issue_quote` takes `DealTerms` and starts after `prepare_terms`,
  the prepare hook does not run on the owner's node, so a removed slot is
  not refused there (Q8's decline never happens) and the deal ends later as
  `not-formed` with `slot-taken`.
  *Failure case:* The implementer follows the plan, the Q8 test
  (`a_removed_slot_is_declined_with_the_hooks_code`) fails, and the fix is
  chosen under time pressure.
  *Fix:* Decide it in the plan. Either `terms_from_listing` has an owner-side
  twin that returns a `DealTermsInput` (so `prepare_terms` still runs), or
  `issue_quote` takes `DealTerms` and calls `hooks::prepare` for each
  author commitment with a prepare hook, then checks that the returned window
  equals the window the terms already hold (a difference is a decline with
  `slot-not-in-listing`). Say which.
  *Status:* Open

- [ ] **P17 — minor.** *Where:* D-C3c-8 (`MAX_TERMS_TEXT_LEN`), D-C3c-11
  (`build_verified_row`).
  *Problem:* Two citations are stale. `deal.rs:21` is `MAX_COMMITMENTS`;
  `MAX_TERMS_TEXT_LEN` is at
  [deal.rs:30](../../../../crates/roym_core/src/deal.rs#L30).
  `build_verified_row` is at
  [verify.rs:118](../../../../crates/roym_federated_query/src/verify.rs#L118),
  not `:116`. The other 53 links I checked are right.
  *Failure case:* A developer opens the wrong line.
  *Fix:* Correct both links.
  *Status:* Open

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

- [x] **P1 — major.** *Where:* D-C3c-2 (The wake worker), `crates/substrate/src/runtime/wake.rs`.
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
  *Status:* Fixed (073d0519)

- [x] **P2 — major.** *Where:* Section 5 (table under D-C3c-1), Step 3 of C3c1 (section 6.1).
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
  *Status:* Fixed (073d0519)

- [x] **P3 — major.** *Where:* Section 6.2 (C3c2 Step 1 vs Step 3), D-C3c-10.
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
  *Status:* Fixed (073d0519)

- [x] **P4 — minor.** *Where:* Section 7.8 (Size and time risks), C3c1 Step 4, C3c2 Step 4, C3c3 Step 1.
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
  *Status:* Fixed (073d0519)

- [x] **P5 — minor.** *Where:* D-C3c-2, line 733.
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
  *Status:* Fixed (073d0519)

- [x] **P6 — minor.** *Where:* D-C3c-2 (The wake worker).
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
  *Status:* Fixed (073d0519)

- [x] **P7 — minor.** *Where:* D-C3c-1 (`transaction.drive`), lines 787-800.
  *Problem:* `drive_at` states:
  "take at most MAX_DRIVE_CONVERSATIONS from DRIVE_PENDING, oldest `since` first".
  In the Roym data layer, listing or filtering rows sorted by a field requires an
  index. If `DRIVE_PENDING` is queried sorted by `since`, it must be registered
  in `ensure_collections` with an index: `idx("since", IndexType::Numeric)`.
  *Failure case:* Querying `DRIVE_PENDING` sorted by `since` fails at runtime with
  an unindexed query error.
  *Fix:* Explicitly state in D-C3c-1 and C3c1 Step 4 that `DRIVE_PENDING` is
  registered in `ensure_collections` with a numeric index on `since`.
  *Status:* Fixed (073d0519)

- [x] **P8 — minor.** *Where:* Section 2.2 point 8, and D-C3c-7.
  *Problem:* The plan cites `messages.ts:783` as "the quote form's prefill from the enquiry".
  `messages.ts:783` is `openQuoteForm`, which constructs a quote form without
  prefilling from enquiry description. The enquiry form that currently enforces
  required description is `openRequestForm` at `messages.ts:428-475`.
  *Failure case:* Developer checking line 783 will find quote form code rather than
  the enquiry description code.
  *Fix:* Update the citation from `messages.ts:783` to `messages.ts:428-475`.
  *Status:* Fixed (073d0519)

- [x] **P9 — minor.** *Where:* D-C3c-6 (`terms_from_listing`).
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
  *Status:* Fixed (073d0519)

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

- [x] **P10 — major.** *Where:* D-C3c-1, the table row for
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
  *Status:* Fixed (073d0519)

- [x] **P11 — major.** *Where:* Q5, section 4 ("`transaction`'s `depends_on`
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
  *Status:* Fixed (073d0519)

- [x] **P12 — major.** *Where:* Q6; D-C3c-12 ("Open" for the limits, and the
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
  *Status:* Fixed (073d0519)

- [x] **P13 — minor.** *Where:* `drive_at` pseudo-code (D-C3c-1,
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
  *Status:* Fixed (073d0519)

- [x] **P14 — minor.** *Where:* D-C3c-1, "Timed work" and "The bound"; D-C3c-2.
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
  *Status:* Fixed (073d0519)

- [x] **P15 — minor.** *Where:* D-C3c-1 table (`services.rs:236`), D-C3c-2.
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
  *Status:* Fixed (073d0519)

- [x] **P16 — minor.** *Where:* D-C3c-12 (`issue_quote`), D-C3c-6, Q8.
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
  *Status:* Fixed (073d0519)

- [x] **P17 — minor.** *Where:* D-C3c-8 (`MAX_TERMS_TEXT_LEN`), D-C3c-11
  (`build_verified_row`).
  *Problem:* Two citations are stale. `deal.rs:21` is `MAX_COMMITMENTS`;
  `MAX_TERMS_TEXT_LEN` is at
  [deal.rs:30](../../../../crates/roym_core/src/deal.rs#L30).
  `build_verified_row` is at
  [verify.rs:118](../../../../crates/roym_federated_query/src/verify.rs#L118),
  not `:116`. The other 53 links I checked are right.
  *Failure case:* A developer opens the wrong line.
  *Fix:* Correct both links.
  *Status:* Fixed (073d0519)

## Plan review round 3 — Claude Opus 5.5, 2026-11-23

This round checks the fixes of rounds 1 and 2. They are in `073d0519`
(the plan and the roym-agreements change doc) and closed in `e58aa418`.
Branch `feat/m08c-c3c-agree-as-listed` at `e58aa418`, main checkout (not
a worktree), `git pull` up to date. The plan author is also Claude Opus
5.5, so the reader should weigh this round as a check of the fixes, not an
independent review.

**Summary of round 3: 5 new findings: 0 blocker, 1 major, 4 minor.**
P1 to P17 are fixed except P16, which is only partly fixed (the rest is
P20). 5 items are open.

### Fixes checked against the code

- **P1.** The worker checks `registry.lookup(&service_id, &spec.interface)`
  before it calls. This is the same test as `run-scheduled`
  ([lifecycle.rs:634](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L634)).
  The linked build registers the Roym services in the same endpoint
  registry (`runtime/roym.rs:179`), so a linked wake is not skipped. Fixed.
- **P2, P10.** `DeployFacts` becomes a named struct with `wake_json`, the
  `ALTER TABLE` migration, both mocks, and `clear_deploy_hash_after_binding_push`
  copies the wake. The test `a_binding_push_keeps_the_wake` is listed.
  Fixed. P22 adds the readers of `deploy_facts()` that the step does not name.
- **P3.** Every struct literal of `EnquiryPayload`, `DealDeclinePayload` and
  `AgreementProgressPayload` is in `roym_core` or `roym_transaction`
  (`git grep`), and C3c2 step 1 now updates both crates. Fixed.
- **P4.** `sync/` already exists as a folder (`sync/confirm.rs` and
  others), so `sync/cards.rs` follows the module layout rule. Fixed.
- **P5.** `EventBus::app_instance_of` is at
  [event_bus.rs:124](../../../../crates/core/src/event_bus.rs#L124). Fixed.
- **P6.** An RAII guard resets `running`, with a test. Fixed.
- **P7.** In-memory sort; the three collections go in `ensure_collections`
  ([app.rs:335](../../../../crates/roym_transaction/src/app.rs#L335)). Fixed.
- **P8.** The citation is now `messages.ts:461`, the required-description
  check. Fixed.
- **P9.** Empty `payment.payee` gives `PriceNotFixed`, with a test. Fixed.
- **P11.** The `catalog` binding in `wire_roym_topology`, a test that
  compares every `depends_on` with the native bindings, and a native
  `listing.get` test. Fixed.
- **P12.** `in_flight_instant` is added to both limits, with the two tests
  and the Deviation. The `decision:` fence is written on the owner's node
  too (`decide::run` from `sync.rs:734`), so a formed agreement is not
  counted twice. Fixed. P19 is a smaller gap in the predicate.
- **P13.** `SyncOutput.more` and compare-and-delete. `sync_at` reads after
  a stored `last_seq` (`sync.rs:84`), so a second sync does continue the
  window. Fixed. P18 is a new problem that the re-run adds.
- **P14.** Recovery once per `drive`, the re-run on `pending > 0`, and the
  measurement with 200 agreements. P21 is a gap in the first part; P18
  is a problem with the second.
- **P15.** `shared.event_bus()` exists (`runtime/handles.rs:187`); the
  `Option` proxy, the `JoinHandle` and `on_events` with no app are all
  stated. Fixed.
- **P16.** Partly fixed. See P20.
- **P17.** Both citations are right now. Fixed.

The owner answers to Q1 to Q8 are recorded in the plan. Q6's answer
depends on P12 (fixed) and P19. Q8's answer depends on P16, so on P20.

Fix note (2026-11-23): P18 to P22 are fixed in `6123bf9a`. P20 also
settles the rest of P16. In the same commit the owner changed Q1 to two
PRs: C3c1 (the driver), then C3c2 (agree as listed, both sides).

### Findings

- [x] **P18 — major.** *Where:* D-C3c-1 `drive_at` (`Err(other) -> keep
  it`, "oldest `since` first"), D-C3c-2 (`has_pending`), C3c1 step 3.
  *Problem:* A row in `DRIVE_PENDING` that fails every time is never
  removed, and it keeps its old `since`. Two things follow. First, it is
  always among the oldest, so it is always taken first. Second,
  `DriveOutput.pending` is then always above 0, so the worker calls
  `drive` again on every 1-second tick, not every 30 seconds. The same
  happens when `more` stays true but `last_seq` does not move: `sync_at`
  sets `last_seq` back to before the first declined or deferred message
  ([sync.rs:105-122](../../../../crates/roym_transaction/src/app/sync.rs#L105)),
  so a window that starts with such a message is read again in full each
  time.
  *Failure case:* A conversation is deleted, so `conversation.changes`
  refuses it on every sync. The node now runs `drive` once per second,
  for ever, and each call can take up to 2 seconds (the stop rule), so the
  `transaction` guest is busy almost all the time. With 8 such rows, no
  other conversation is ever synced by the wake, and timed work for those
  conversations never runs.
  *Fix:* On `Err(other)`, put the row again with `since = now` and a
  counter `failures`. Drop the row after a small number of failures (for
  example 5), with a `tracing` warning; a new message event puts it again.
  Re-put on `more` only when `last_seq` grew. The worker re-runs at once
  only when the call took a full batch (`synced ==
  MAX_DRIVE_CONVERSATIONS`); otherwise it waits for `every_secs`. Tests:
  `a_failing_conversation_does_not_starve_the_others` and
  `a_failing_row_does_not_make_the_wake_run_every_second`.
  *Status:* Fixed (6123bf9a)

- [x] **P19 — minor.** *Where:* D-C3c-12, "Open for the limits and
  in-flight quotes".
  *Problem:* `in_flight_instant` counts an `answered` row while its quote
  has no `decision:` fence and has not expired. A quote that the
  counterparty declined has no `decision:` fence (a decline writes
  `decline:<quote>`). So it holds a place for up to 7 days. A helper that
  already answers "has this quote ended?" exists: `is_deal_ended`
  ([app.rs:662](../../../../crates/roym_transaction/src/app.rs#L662)). It
  covers expiry, decline, `not-formed` and a terminal agreement.
  *Failure case:* Y taps, gets an instant quote, declines it, then taps
  again. The second tap waits for the owner with `CounterpartyHoldsOne`,
  and the declined quote also uses one place of the cap for 7 days.
  *Fix:* Count an `answered` row when there is no `decision:` fence and
  `is_deal_ended(quote_id, expiry, now)` is false. Test
  `a_declined_instant_quote_frees_its_place`.
  *Status:* Fixed (6123bf9a)

- [x] **P20 — minor.** *Where:* D-C3c-12 (`issue_quote`, plan lines
  1220-1229), C3c2 step 4, Q8.
  *Problem:* The P16 fix does not settle the types. It also adds a
  behaviour change. (1) The plan still says `issue_quote` is "the body of
  `set_at` from `prepare_terms` on". `prepare_terms` takes a
  `DealTermsInput`, but `terms_from_listing` returns a `DealTerms`. (2) It
  also says `issue_quote` calls `hooks::prepare` for author commitments.
  `hooks::prepare` takes a `&CommitmentInput`
  ([hooks.rs:280](../../../../crates/roym_transaction/src/app/hooks.rs#L280)),
  but `DealTerms.commitments` is `Vec<Commitment>`
  ([deal.rs:184](../../../../crates/roym_core/src/deal.rs#L184)). (3) It
  says the enquiry is declined inside `issue_quote`. But `set_at` uses
  `issue_quote` too.
  *Failure case:* The code does not compile as written. If the builder
  makes it compile the obvious way, `quote.set` runs the prepare hook
  twice. A normal `quote.set` for a removed slot then declines the person's
  enquiry, where today it returns the hook's error to the owner. No test
  covers that change.
  *Fix:* `issue_quote` starts after `prepare_terms`, at
  `validate_and_conceal_terms` (`quote_ops.rs:187`), and takes `DealTerms`.
  `set_at` stays as it is today. Only `answer_as_listed` calls the hook:
  it builds a `CommitmentInput` from each author commitment that has a
  prepare hook (one small `From<&Commitment>`, or a `prepare` that takes
  the extension, details and listing id), calls `hooks::prepare`, and
  compares the window. On a refusal it declines with the hook's code. Keep
  the existing `quote.set` test for a removed slot as the guard.
  *Status:* Fixed (6123bf9a)

- [x] **P21 — minor.** *Where:* D-C3c-1 ("Timed work", `sync_direct`),
  C3c1 step 4.
  *Problem:* The plan runs `recover_deciding_rows` once at the start of
  `drive_at`. But `sync_at` also runs it at the end of every sync
  ([sync.rs:135](../../../../crates/roym_transaction/src/app/sync.rs#L135)),
  and `sync_direct` is `sync_at` with a typed refusal. The plan does not
  say that `sync_direct` leaves recovery out.
  *Failure case:* One `drive` still runs the node-wide recovery 9 times,
  not once. The listed test `drive_runs_recovery_once_per_call` fails, and
  the fix is chosen during the build.
  *Fix:* Say it: the inner function that `sync_direct` uses does not call
  `recover_deciding_rows`, and `sync_at` calls it after the inner function,
  so `transaction.sync` keeps today's behaviour.
  *Status:* Fixed (6123bf9a)

- [x] **P22 — minor.** *Where:* C3c1 step 3 ("read every caller of
  `set_deploy_facts`").
  *Problem:* Changing `DeployFacts` from a tuple to a struct also breaks
  every reader of `deploy_facts()`. They use tuple patterns or `.1`/`.3`:
  `orchestration.rs:315`, `deploy/admission.rs:198`, `lifecycle.rs:541`,
  `status.rs:34`, `:202`, `:335`, and `tests/deploy.rs:1032`, `:1060`,
  `:1067`. The step names only the writers.
  *Failure case:* The compiler finds them, so nothing breaks at run time.
  But the step's size estimate and its file list are short by about nine
  sites in four files.
  *Fix:* Add `git grep -n 'deploy_facts('` to the step and list these
  files.
  *Status:* Fixed (6123bf9a)

## Plan review round 4 — Claude Opus 5.5, 2026-11-23

This round checks the round 3 fixes in `6123bf9a` (closed in `3706044d`).
Branch `feat/m08c-c3c-agree-as-listed`, main checkout (not a worktree),
`git pull` up to date. The same model wrote the plan, so this is a check of
the fixes, not an independent review.

**Summary of round 4: 0 new findings. 0 open. The plan review is closed.**

### Fixes checked against the code

- **P18.** A failing row goes to the end of the queue with `failures + 1`,
  and is dropped after 5 with a warning. `more` is true only when
  `last_seq` grew. `again` is true only after a full batch with rows left.
  The worker reads `again` and `warnings` from the reply. This works
  because `ServiceProxy::invoke` returns the callee's JSON-RPC `result`
  value, not the envelope
  ([proxy.rs:346](../../../../crates/rpc/src/proxy.rs#L346)). The plan
  lists four service tests and three worker tests for it. Fixed.
- **P19.** `in_flight_instant` uses `is_deal_ended`
  ([app.rs:663](../../../../crates/roym_transaction/src/app.rs#L663)), and
  the test `a_declined_instant_quote_frees_its_place` is listed. The
  roym-agreements Deviation was updated too. Fixed.
- **P20 (and the rest of P16).** `issue_quote` starts at
  `validate_and_conceal_terms`
  ([quote_ops.rs:187](../../../../crates/roym_transaction/src/app/quote_ops.rs#L187)),
  and `set_at` keeps `prepare_terms`, so `quote.set` is unchanged. The hook
  runs only in `answer_as_listed` (`check_author_slots`), after the
  early return for an existing `answer:` row and before the fence.
  `hooks::prepare` reads only `extension`, `details` and `window`
  ([hooks.rs:280](../../../../crates/roym_transaction/src/app/hooks.rs#L280)),
  so the `prepare_window` split needs no type conversion. A refusal comes
  back as `invalid_params` with the sibling's `data`, and an unreachable
  sibling as `internal_error`. This matches the plan's `Refused` and
  `Unavailable`. Fixed.
- **P21.** The recovery call moves from the inner function to `sync_at`.
  The tests `drive_runs_recovery_once_per_call` and
  `sync_still_runs_recovery_once` are listed. Fixed.
- **P22.** Step 3 lists 13 reader sites in six files. I checked each line,
  including `data_db/src/registry_store/tests.rs:44`, `:92` and `:99`.
  Fixed.

### The change to two PRs (Q1)

The owner changed Q1 to two PRs in the same commit. I checked that the
plan is consistent after the merge of the old C3c2 and C3c3:

- No `C3c3` is left in the plan, `status.md`, `task.md` or the three
  change docs, except in the Q1 option text that records the old choice.
- Section 3 (criteria), section 5 (what changes), section 6.2 (steps 1 to
  5 are the owner's side, steps 6 to 8 the counterparty's side), section 7
  (tests and the cross-references to section 7.4), section 7.9 (sizes),
  section 7.10 (Hindi), section 8 (backlog), section 9 (living docs) and
  section 10 (exit criteria) all name C3c1 or C3c2 only.
- "How this is shown to an end user" covers both sides under C3c2.
- Q7 (demo) is restated: C3c1 `- none`, C3c2 extends story 50.
- The C3c2 production size is about 1,950 lines. This is the size that the
  old option (b) gave, so the numbers agree.

## Code review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-23

This round reviews **part C3c1 only** (the driver): plan section 6.1 and
D-C3c-1 to D-C3c-4, PR #381, branch `feat/m08c-c3c-agree-as-listed`, at
`e8a57a68`. Main checkout, not a worktree, `git pull` up to date. Diff:
`git diff origin/main...HEAD` (merge base `8fafab15`). The code was written
by Gemini 3.8 Flash. C3c2 is not built yet; its findings will continue the
R numbers.

**CI.** `gh pr checks 381` on `e8a57a68` at review time: "Lints, licenses &
checks" and the CLA check pass; the six test shards and Playwright E2E are
still pending. So CI is not yet proof that the full suite passes.

**Tests I ran** (targeted only): `cargo nextest run -p
syneroym-roym-transaction drive`: 14 passed. `cargo nextest run -p
syneroym-substrate --lib wake`: 8 passed. `cargo nextest list -p
syneroym-roym-transaction`: 226 tests in the crate.

### Checked and found correct

- **(1) A wake never leaves the node.** `WakeWorker::tick_target` calls
  `registry.lookup(service_id, interface)` before every call and skips on a
  miss ([wake.rs:145](../../../../crates/substrate/src/runtime/wake.rs#L145)).
  This is the same test as `run_scheduled_impl`
  ([lifecycle.rs:628](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L628)),
  which is exactly `invoke_inner`'s local-or-remote test. The unit test
  `wake_worker_skips_when_endpoint_not_registered` shows no proxy call on a
  miss. The lookup and the call are a few microseconds apart (the same gap
  `run-scheduled` has); I accept that. But see R5 for what a miss does.
- **(2) One call at a time.** `running` is set before the spawn and is read
  at the top of each tick. `RunningGuard` is built as the first statement of
  the spawned task, so a panic or a timeout inside `proxy.invoke` resets it.
  `invoke_local` applies `spec.timeout_ms` to both the WASM and the native
  path ([router.rs:224](../../../../crates/router/src/proxy/router.rs#L224)),
  so a hung call cannot wedge the worker. Events stay on the bus while a
  call runs (the early return is before `check_events`), so nothing is
  queued and nothing is lost.
- **(3) `again` in the guest.** `drive_at` sets `again` only when `synced ==
  8` and rows are left; failed rows do not count as synced, so rows that
  fail can never make `again` true. A failing row is put again with `since =
  now` and `failures + 1`, and is deleted with a warning at 5. Timed-work
  candidates get `since = now`, so stored pending rows always sort before
  them. `more` is `scanned == SYNC_WINDOW && new_last_seq >
  sync_state.last_seq`, as P18 asks. **But the worker never reads `again`:
  see R1.**
- **(4) `DeployFacts`.** It is a named struct in `storage.rs`; every reader
  in the plan's list of 13 sites (and `orchestration.rs:316`,
  `cert.rs:122`) now uses field names; `data_db` adds the `wake_json`
  column in the same `ALTER TABLE` loop as `manifest_hash` and
  `visibility`. `clear_deploy_hash_after_binding_push` now copies the whole
  struct and clears only `manifest_hash`, and `a_binding_push_keeps_the_wake`
  proves it through `write_bindings`.
- **(5) Recovery.** `sync_direct` no longer calls `recover_deciding_rows`;
  `sync_at` calls it after a successful `sync_direct`, as before (an error or
  a group conversation still returns early, before recovery, as on `main`).
  `drive_at` calls it once, at the start. `transaction.sync`'s error codes
  are unchanged: `fetch_sync_messages` already answered `internal_error` for
  every sibling error, and the group case still answers `invalid_params`
  with `CARDS_NOT_IN_GROUPS_MESSAGE`. **But the two tests that should prove
  this prove nothing: R3.**
- **(6) Both builds.** `transaction.drive` is in the method table, admitted
  by the default `require_internal`, the contract count is 45 with a shape
  file, and parity scenario 252 runs it in both builds. The linked build
  parses the embedded `roym.toml` with `SynAppManifest::from_toml` and
  registers the wake for `did:key:roym-transaction` (one source). CI runs
  nextest with `--all-features`, so the `roym`-gated test runs there. **But
  that test is vacuous: R2.**
- **(7) ADR-0031** matches the code on scope, the local endpoint check, the
  1-second tick, one call at a time, no queue, no retry, the coordinator-mode
  skip and the "logged once" warning for a service with no app context. It
  does not match on the reply contract (R1) and on "works identically in
  both builds" (R2).
- **(8) Stop rule.** Measured in parity
  (`scenario_stop_rule_measurement_200_agreements_8_conversations`, both
  builds, 200 agreements, 8 conversations with one enquiry card each):
  69.8 ms, recorded in status.md and in the roym-agreements Deviations.
  `MAX_DRIVE_CONVERSATIONS` stays 8.
- `WakeSpec` and its validation match D-C3c-1 rule by rule, with one test
  per rule; `ServiceConfig` keeps its deploy hash with no wake. The wire
  change (the `wake-spec` record and `service-config.wake` in
  `control-plane.wit`, the SDK mapper) is recorded in the roym-agreements
  Deviations, and the substrate validates the wake again at deploy.
- Security: a wake can name only an interface of its own service; the call
  is `CallerContext::service_system` of that service, so it gets nothing a
  deployer of that service does not already have.
- The seven substrate sync-count sites (D-C3c-4) no longer read `filed` or
  `countersigned`; each still asserts the card or agreement that follows.
  `roym_driver_e2e.rs` calls `transaction.sync` on neither node.
- Sizes: every changed production file is under 800 lines (`sync.rs` 641
  after the `sync/cards.rs` move). No planning ids in code, `.wit`,
  `.toml` or `.json`. The oversized-test-file baselines went down only, and
  the 9 removed `#[test]`s of `models/tests.rs` moved to
  `models/tests/schedule.rs`.
- The deferred-backlog row "A card is filed only when a client calls
  `transaction.sync`" moved to "Recently resolved", and the new row "Many
  woken services on one node are not pooled" sits beside ADR-0023's row.
- Not C3c1 code, but in the PR: `e2cafe91` adds
  `docs/ideas/did-nid-match-process.md` and `99ff3dd0` bumps `url`. Both are
  the owner's commits; I did not review them.

**Counts: 0 blocker, 2 major, 11 minor. 13 open.** (The status of each finding is in its own *Status* line, and the close-out check is in round 2 below.)

### Findings

- [x] **R1 — major.** *The worker never reads the wake reply: `again` and `warnings` are lost.*
  *Where:* [wake.rs:226](../../../../crates/substrate/src/runtime/wake.rs#L226)
to `:246` (`spawn_wake_call`); [wake/tests.rs:29](../../../../crates/substrate/src/runtime/wake/tests.rs#L29)
(`MockProxy`).

*Problem:* The worker does `val.get("again")` and `val.get("warnings")` on
the value `proxy.invoke` returns. For a Roym service that value is not the
`DriveOutput` object. The guest export is `invoke: func(request: string) ->
result<string, string>`
([world.wit:4](../../../../crates/roym_transaction/wit/world.wit#L4)), and
`invoke_local` turns `Ok(string)` into `Value::String` with the envelope
text inside (`execute_wasm_json` → `wasm_results_to_json`,
[conversions.rs:462](../../../../crates/sandbox_wasm/src/conversions.rs#L462)).
So `val` is `Value::String("{\"result\":{\"synced\":8,…,\"again\":true}}")`,
and `.get(...)` on a string is always `None`. Guests read the same reply
with `read_reply`, which parses the string first
([method.rs:408](../../../../crates/roym_core/src/method.rs#L408)). The plan
review note for P18 (round 4) said `invoke` returns the callee's `result`
value; that is true for the JSON-RPC layer, but the Roym `result` is a
string. The unit test cannot see this: `MockProxy` returns a JSON object
(`json!({"again": true, …})`), which no Roym service returns.

The same cause hides guest errors: a `drive` that answers
`{"error": …}` is `Ok(Value::String(…))` at the proxy, so "wake call failed"
is never logged for it.

*Failure case:* X restarts with 100 direct conversations. The first drive
gets `reload`, rescans 16 per call, syncs 8 per call and returns `again:
true`. The worker ignores it, so the next call waits for a new message or
for `every_secs` (30 s): about 6 minutes to drain instead of about 13
seconds. A conversation that fails 5 times is dropped, and the "gave up on
<conv>" warning, the only report of it, is never logged. ADR-0031 §6 is not
true for any Roym service.

*Fix:* Read the reply through one small function, for example
`fn wake_reply(val: Value) -> Result<Value, String>`: when `val` is a
string, parse it as JSON; when it then holds `error`, return it as an error
(log it as "wake call failed"); when it holds `result`, use that; otherwise
use the value as it is (a non-Roym service that returns an object). Then
read `again` and `warnings` from the result. Change `MockProxy` to return
the envelope string a Roym guest returns, and keep one case with a plain
object. Add a worker test where the reply is a string envelope with `again:
true`, one with `warnings`, and one with `error`.

*Status:* Fixed (20f925c3). `wake_reply` in `wake.rs` now inspects `Value`: if string, it parses JSON and unpacks `result` (or treats `error` as a failure with guest error logging); if object, it reads directly. `again` and `warnings` are read from the result payload. `MockProxy` in `wake/tests.rs` now returns string envelopes by default. Added tests for object `again`, string envelope with `warnings`, and string envelope with `error`. Proof: before unpacking envelopes, `wake_reply_contract_again_sets_has_pending` failed (`left: 2, right: 3`); after unpacking, passed (3).

- [x] **R2 — major.** *The linked-build wake is not proved: both registration tests pass with no wake.*
  *Where:* [runtime/tests.rs:125](../../../../crates/substrate/src/runtime/tests.rs#L125);
[services/tests.rs:260](../../../../crates/roym_core/src/services/tests.rs#L260).

*Problem:* D-C3c-3 asks for a test that "parses the manifest and checks that
every service that declares a wake is registered". There are two tests with
that name, and neither checks it:

- The substrate test loops over `registry.wake_targets()` and checks each
  entry. If `register_roym_wakes` registers nothing, or registers under a
  wrong id, the loop has nothing to check and the test passes.
- The `roym_core` test only checks that the text of `runtime/roym.rs`
  contains `register_roym_wakes(endpoint_registry).await?` and
  `ROYM_MANIFEST_TOML`. Renaming a variable breaks it; registering the wrong
  service does not.

No test calls the wake in the linked build. Every substrate test of the
driver deploys the WASM build.

*Failure case:* someone changes `roym_dispatch_id` or the service name used
in `register_roym_wakes`. The linked (product) build then registers the wake
for an id with no endpoint, the worker skips it every second (R5), and no
deal moves without a Hub. Both tests still pass.

*Fix:* In the substrate test, build the expected set from the manifest
(each service with a `wake`, mapped through `roym_dispatch_id`), and assert
`wake_targets()` equals it, with each parsed `WakeSpec` equal to the
manifest's. Assert the set is not empty. Then drop the `roym_core` text
test, or keep it only as a pointer. If a linked-build boot is affordable in
`runtime/tests.rs`, also run one `WakeWorker::tick` against the linked
`transaction` and assert the reply is a `DriveOutput` (this also covers R1
for the native path).

*Status:* Fixed (3d4fad35). `the_linked_build_registers_every_wake_of_the_manifest` in `crates/substrate/src/runtime/tests.rs` parses `ROYM_MANIFEST_TOML`, maps services declaring wakes via `roym_dispatch_id`, and asserts the expected set is non-empty and matches `wake_targets()` with equal `WakeSpec`s. Replaced brittle substring test in `roym_core/src/services/tests.rs` with a pointer comment. Proof: commenting out `register_roym_wakes` in `runtime/roym.rs` caused the substrate test to fail (`left: {}, right: {"did:key:roym-transaction": WakeSpec { ... }}`).

- [x] **R3 — minor.** *The two recovery tests prove nothing.*
  *Where:* [tests/drive.rs:286](../../../../crates/roym_transaction/src/app/tests/drive.rs#L286)
to `:311`.

*Problem:* `drive_runs_recovery_once_per_call` asserts only `synced == 2`.
`sync_still_runs_recovery_once` asserts only `scanned == 0`. Neither looks
at recovery. Both pass if `drive_at` runs `recover_deciding_rows` once per
conversation (the P14 cost) and if `sync_at` drops the call (the P21
regression). Focus point 5 depends on these two tests.

*Failure case:* a later change moves the recovery call back into
`sync_direct`. A drive of 8 conversations then reads every agreement 9
times. Both tests stay green.

*Fix:* Make recovery observable. For example, seed one `deciding` row old
enough for `recover_deciding_rows` to act on, with a stub that counts the
resend (or count the `AGREEMENTS` queries in `TestHost`). Assert: one
`sync_at` recovers it (so `transaction.sync` keeps today's behaviour), and
one `drive_at` over several conversations runs recovery exactly once.

*Status:* Fixed (2611c71c). Made recovery observable: `drive_runs_recovery_once_per_call` and `sync_still_runs_recovery_once` seed an expired `deciding` row before calling `drive_at` / `sync_at`, asserting `state == NotFormed` and `recovery_queries == 1`. Proof: commenting out `recover_deciding_rows` in `drive.rs` caused `drive_runs_recovery_once_per_call` to fail (`left: 3, right: 2`); commenting out in `sync.rs` caused `sync_still_runs_recovery_once` to fail (`left: Deciding, right: NotFormed`).

- [x] **R4 — minor.** *The worker's event test passes on the interval call, and most planned worker tests are missing.*
  *Where:* [wake/tests.rs:198](../../../../crates/substrate/src/runtime/wake/tests.rs#L198)
to `:245`.

*Problem:* In `wake_worker_triggers_on_event`, `every_secs` is 60 but
`last_run_at` starts at 0, so the first tick is already due and starts a
call. The test runtime has one thread and `tick` never yields, so on the
second tick that call has not run yet: `running` is true and the tick
returns at once, before it reads the event. The one invocation the test
counts is the first-tick interval call. The test passes with the event
code removed.

The plan's list (section 7.2) has these worker tests, which do not exist:
`a_burst_of_events_gives_one_call`, `an_event_of_another_kind_gives_no_call`,
`the_interval_calls_with_no_event`, `a_slow_call_is_never_overlapped`,
`a_reload_of_the_event_window_gives_a_call`,
`an_undeployed_service_is_not_called_again`,
`a_reply_without_again_waits_for_the_interval`,
`warnings_in_the_reply_are_logged`. `a_panic_in_invoke_resets_running_guard`
tests the guard alone, not a proxy that panics inside the worker. Two tests
use `every_secs = 0`, which validation refuses.

*Failure case:* `check_events` stops matching kinds (for example it compares
against the subject). Every worker test still passes; only the slow
two-node test would show it.

*Fix:* Let the first call finish (yield, then check `running` is false),
then record an event and assert a second call. Add the listed tests. Use a
proxy that panics for the guard test, and a valid `every_secs` (use a fake
clock or set `last_run_at` in the state instead of `every_secs = 0`).

*Status:* Fixed (b9c87d66 for the round-1 part; 27e29d31 for the
round-2 remainder). The round-1 fix added the event test, the listed
worker tests and valid `every_secs` values; its proof: removing the
event trigger made `wake_worker_triggers_on_event` fail
(`left: 1, right: 2`). The round-2 remainder: `wake_reply` is tested
directly — a string envelope gives its `result`, a string with
`error` gives `Err`, a plain object gives itself, a non-JSON string
gives `Err` — and the warnings read is the pure `reply_warnings`,
tested the same way (string entries in order, non-strings skipped,
no array is empty). Proof: with `wake_reply` made to return the
reply unparsed, `wake_reply_string_envelope_gives_its_result`,
`wake_reply_string_envelope_with_error_gives_err`,
`wake_reply_string_that_is_not_json_gives_err` and
`wake_reply_contract_again_sets_has_pending` failed; with
`reply_warnings` made to return empty,
`reply_warnings_lists_string_entries_in_order` and
`reply_warnings_skips_entries_that_are_not_strings` failed; after
undo, all 28 wake tests pass.

- [x] **R5 — minor.** *A missing local endpoint logs a warning every second and loses the trigger.*
  *Where:* [wake.rs:134](../../../../crates/substrate/src/runtime/wake.rs#L134)
to `:152`.

*Problem:* On a lookup miss the worker returns before it sets `last_run_at`.
So `due` stays true, and the next tick warns again: one `warn!` per second
for as long as the endpoint is missing. Also, by then the tick has already
taken `has_pending` (`swap(false)`) and moved the bus cursor past the
matching event, so the trigger is lost; the next call waits for a new event
or the interval.

*Failure case:* a WASM service whose component fails to load keeps its
deploy facts but has no endpoint: the log gets 86,400 identical warnings a
day. A node that boots before its endpoints are restored, or the linked id
bug of R2, does the same.

*Fix:* Warn once per target state while the endpoint is missing (as
`warned_no_app_context` does), and reset the flag when a call starts. Keep
the trigger: put `has_pending` back to true on a miss, or read events only
after the lookup passes. Add a test for both.

*Status:* Fixed (b9c87d66). Added `warned_missing_endpoint` flag to `TargetState` to log a warning once while an endpoint is missing and reset once found. `tick_target` checks the local endpoint before reading event bus or clearing `has_pending`, preserving triggers across missing endpoint periods. Added tests `a_missing_local_endpoint_preserves_has_pending_trigger` and `a_missing_local_endpoint_skips_dispatch_and_preserves_event_trigger`.

- [x] **R6 — minor.** *The linked build writes deploy facts with an unknown service type.*
  *Where:* [roym.rs:604](../../../../crates/substrate/src/runtime/roym.rs#L604).

*Problem:* `register_roym_wakes` stores `DeployFacts::new("native")` for
`did:key:roym-transaction`. Before this change a linked Roym service had no
deploy facts. Now it has facts with `service_type = "native"`, which
`parse_service_type` does not know (it knows `"nativehost"`,
[types.rs:224](../../../../crates/control_plane/src/service/orchestration/types.rs#L224)).
The facts are also read by other code: `restart` now answers "has a recorded
service type ('native') this substrate does not recognize; redeploy to
correct it" (a redeploy is impossible for a linked service);
`renew_cert_impl` no longer refuses with "not deployed here"; `status`
passes the type to `instance_phase`. None of this is in the plan or the
Deviations. The plan's own sketch has a separate source ("the registry rows,
and the linked Roym wake").

*Failure case:* an operator runs `roymctl` restart on the linked
`transaction`, gets the "redeploy to correct it" text and cannot follow it.

*Fix:* Give the worker the linked wake through a second `WakeTargets`
source (a fixed list built from the embedded manifest at `init_roym`), and
leave deploy facts to deploys. If deploy facts stay, use `"nativehost"`,
record the side effects in the Deviations, and add a test that `restart`
and `status` of the linked service answer as before.

*Status:* Fixed (3d4fad35). `register_roym_wakes` in `crates/substrate/src/runtime/roym.rs` now records facts with `"nativehost"`. Added unit tests in `crates/control_plane` proving `restart` and `status` handle linked nativehost services without failing. Recorded deviation in `docs/planning/changes/roym-agreements/change.md`.

- [x] **R7 — minor.** *A drive with no stored cursor never rescans, and the test host hides it.*
  *Where:* [drive.rs:111](../../../../crates/roym_transaction/src/app/drive.rs#L111);
[testing/stubs.rs:124](../../../../crates/roym_core/src/testing/stubs.rs#L124)
to `:143`.

*Problem:* The real bus answers a read with no cursor with no events and the
head cursor, not `reload`
([event_bus.rs:205](../../../../crates/core/src/event_bus.rs#L205)). So the
first `drive` with an empty `DRIVE_STATE` starts at the head and does not
rescan. The plan says "after a restore, the first `reload` starts a full
pass" (D-C3c-2, "`drive` is not in the backup"); that is not what happens.
The test host differs from the real bus here: with no cursor it returns
every event from the start, so `drive_syncs_each_conversation_named_by_a_message_event`
and others pass on behaviour no real host has. (Parity scenario 252 works
around it with a first drive "to capture the baseline event cursor".)

*Failure case:* a backup is restored on a new node. `DRIVE_STATE` is empty,
the first drive takes the head cursor, and a restored conversation with an
unfiled card is not synced until a new message arrives (unless it also holds
an unfinished agreement).

*Fix:* In `poll_events`, treat `event_cursor: None` like `reload`: take the
head and set `rescan_offset = Some(0)`. Make `TestHost::read_events(None)`
return no events and the head cursor, as the real bus does, and fix the
tests that relied on the old stub.

*Status:* Fixed (2611c71c). In `crates/roym_transaction/src/app/drive.rs`, `poll_events` treats `event_cursor: None` like `Reload` (takes head cursor and initializes `rescan_offset = Some(0)`). In `crates/roym_core/src/testing/stubs.rs`, `TestHost::read_events(None)` now returns empty events and the head cursor, matching the real event bus. Added test `drive_initial_poll_without_cursor_captures_head_and_starts_rescan`.

- [x] **R8 — minor.** *The restart test does not prove the reload path.*
  *Where:* [roym_driver_e2e.rs:195](../../../../crates/substrate/tests/roym_driver_e2e.rs#L195),
`:254` to `:256`.

*Problem:* `stopped_quote_author_restarts_and_countersigns_via_reload_path`
stops X, lets Y accept, and starts X again. The acceptance reaches X only
after X is up, so X's conversation records a `conversation.message` event,
and the event path alone moves the deal. The test passes with the reload
rescan removed. Its name and its comment ("on restart its wake sees Reload,
rescans conversations, and countersigns") claim more.

*Failure case:* the `reload` branch of `poll_events` breaks; this test stays
green.

*Fix:* Rename the test and fix the comment to say what it shows (a stopped
writer countersigns after restart with no call), and leave the reload proof
to `drive_after_reload_walks_every_conversation_in_pages`. Or make the card
arrive before the drive's first event read after restart, if the harness
can do that.

*Status:* Fixed (e43c5e85). Renamed test in `crates/substrate/tests/roym_driver_e2e.rs` to `stopped_quote_author_restarts_and_countersigns` and updated the comment to accurately state what is verified.

- [x] **R9 — minor.** *Two "why" comments were dropped from the deploy-facts docs.*
  *Where:* [storage.rs:113](../../../../crates/core/src/storage.rs#L113) to
`:116`.

*Problem:* `save_deploy_facts`' doc on `main` said that a redeploy that
drops the check writes `None`, "clearing it by construction", and that
"`manifest_hash` is written only on full deploy success, so a half-failed
deploy is never deduplicated on the next attempt". The new doc keeps only
"(upsert)". These were the reasons, not the wording, and AGENTS.md asks to
keep them.

*Failure case:* a later change writes `manifest_hash` before the deploy
ends; nothing at the trait says why that is wrong.

*Fix:* Put both sentences back on `save_deploy_facts` (they still hold for
the struct form).

*Status:* Fixed (e43c5e85). Restored the two "why" sentences in `crates/core/src/storage.rs` on `save_deploy_facts` doc comment regarding clearing by construction and partial deploy deduplication prevention.

- [x] **R10 — minor.** *The developer guide's wake example is invalid, and `transaction.drive` is not documented.*
  *Where:* [developer-guide.md:1441](../../../../docs/developer-guide.md#L1441)
to `:1448`; section 6.7.

*Problem:* The example has `interface = "api"` (validation refuses an
interface the service does not declare) and `on_events =
["events.conversation.received"]` (no such event kind; the host records
`conversation.message`). It does not match `roym.toml` or ADR-0031. It also
does not give the limits (5 to 86,400 s, at most 8 kinds, `timeout_ms` 1 to
30,000) or the reply contract. Plan section 9 asks for `transaction.drive`
in the `transaction` method docs; the guide does not name its input or
output, nor the new `more` field of `transaction.sync`.

*Failure case:* a developer copies the example; the deploy is refused.

*Fix:* Copy the `roym.toml` block, add the limits and the reply contract
(`again`, `warnings`, and the string-envelope point of R1), and add
`transaction.drive` (`{}` → `{synced, pending, rescanning, again,
warnings?}`) and `more` to section 6.7.

*Status:* Fixed (e43c5e85 for the round-1 part; 02196adc for the
round-2 remainder). The round-1 fix corrected the wake example, the
limits and the reply contract. The round-2 remainder (02196adc):
section 6.7 lists the real `transaction.sync` fields (`scanned`,
`filed`, `refused`, `unknown`, `countersigned`, `deferred`,
`unsent_failures`, `last_seq`, `more`) — there is no `cards_filed` —
and states `more`'s exact rule: `true` only when the sync read a
full window of 500 messages and `last_seq` grew; more unread
messages may remain, and the next sync continues from the stored
`last_seq`.

- [x] **R11 — minor.** *Two backlog notes of plan section 8 are missing.*
  *Where:* [deferred-backlog.md:416](../../deferred-backlog.md) ("A schedule
deployed by `roymctl app deploy` never runs") and `:548` ("Services can
only pull events").

*Problem:* Plan section 8 says each of these rows gets a note: the wake is
not a schedule (point at ADR-0031), and the wake triggers a pull but there
is still no push export. Neither note was added.

*Failure case:* a reader of the schedule row thinks a Roym service deployed
with `app deploy` has no timer at all.

*Fix:* Add one sentence to each row, as the plan says.

*Status:* Fixed (e43c5e85). Added clarifying notes to `docs/planning/deferred-backlog.md` rows at lines 416 and 548 (pointing to ADR-0031 node-local wake and noting wake triggers event pull).

- [x] **R12 — minor.** *status.md test counts cannot be checked.*
  *Where:* [status.md:20](status.md) (the C3c1 row).

*Problem:* "`roym-transaction` 43 passed" names no filter; the crate has 226
tests and its `drive` filter has 14. The same holds for "`sdk` 54" and
"`control-plane` 31". "`dual_build_parity/transaction` 28 passed" names the
wrong file: the new scenarios are in `dual_build_parity/as_listed.rs`. The
row has no CI result.

*Failure case:* a reviewer cannot repeat the numbers, so they prove
nothing.

*Fix:* Give each count with its exact command (crate and filter), or the
full-crate counts. Add the CI run result for the reviewed head when it
finishes.

*Status:* Fixed (in the close-out commit). `status.md` C3c1 row updated with exact test commands, filter paths, pass counts, and reference to CI.

- [x] **R13 — minor.** *Inline qualified paths and a copied parity helper.*
  *Where:* [deploy/manifest.rs:165](../../../../crates/control_plane/src/service/orchestration/deploy/manifest.rs#L165)
to `:186`; [wake.rs:97](../../../../crates/substrate/src/runtime/wake.rs#L97);
[tests/drive.rs:23](../../../../crates/roym_transaction/src/app/tests/drive.rs#L23);
[testing.rs:78](../../../../crates/roym_core/src/testing.rs#L78), `:214`,
[testing/stubs.rs:112](../../../../crates/roym_core/src/testing/stubs.rs#L112),
`:136`; [as_listed.rs:9](../../../../crates/roym_web/tests/dual_build_parity/as_listed.rs#L9),
`:25`, `:33`, `:146`, `:159`.

*Problem:* AGENTS.md's import pass asks for types through `use`, with no
inline paths of several `::`. The diff adds `syneroym_app_orchestration::InterfaceName::new`
(four times), `syneroym_app_orchestration::WakeSpec`,
`tokio::time::MissedTickBehavior::Skip`, `syneroym_roym_core::testing::TestHost`,
`syneroym_app_host::types::events::Event` / `EventsError` / `ReadResult`,
`syneroym_roym_core::record::RecordDraft`, `std::time::Instant::now()`. And
`peer_signed_request_with_identity` in `as_listed.rs` is a copy of the
helper of the same name in `booking.rs` with one more parameter; AGENTS.md
asks to reuse an existing helper.

*Failure case:* none at run time; the rules are not met.

*Fix:* Import the types. Move one `peer_signed_request_with_identity(peer,
conv, seq, issued_at)` into `fixtures.rs` and call it from both files.

*Status:* Fixed (b056b0a7). Deduplicated `peer_signed_request_with_identity` into `crates/roym_web/tests/dual_build_parity/fixtures.rs` and reused in `as_listed.rs` and `booking.rs`. Cleaned up inline qualified paths across `manifest.rs`, `wake.rs`, `tests/drive.rs`, `testing.rs`, `testing/stubs.rs`, `as_listed.rs`, and `booking.rs`.

## Code review round 2 — Claude Opus 5.5 (Claude Code), 2026-11-24

This round checks the fixes of round 1 in `20f925c3`, `3d4fad35`,
`2611c71c`, `b9c87d66`, `b056b0a7` and `e43c5e85` (closed in `b5c8bf43`),
and a CI failure on this branch. Branch `feat/m08c-c3c-agree-as-listed` at
`b5c8bf43`, main checkout, up to date.

**Process note.** The fix session rewrote the round 1 counts line to "0
open … fixed and verified". A round's counts are its record, and checking
the fixes is the reviewer's step. I put the round 1 line back. The fix
session should change only the *Status* lines.

**Tests I ran:** `cargo nextest run -p syneroym-substrate --lib wake`: 21
passed. `cargo nextest run -p syneroym-roym-transaction drive`: 15 passed
(221 tests in the crate, as status.md says). `cargo nextest run -p
syneroym-substrate --features roym --lib the_linked_build_registers`: 1
passed. `cargo nextest run -p syneroym-roym-web --test method_contract
every_transaction_method`: **failed**, the same failure as CI (R14).

### Fixes checked

- **R1 — Fixed.** `wake_reply` parses a string reply as JSON, returns
  `error` as a failure ("wake call failed" is logged) and uses `result`;
  an object reply is read as it is. `again` and `warnings` are read from
  the result. `MockProxy` now returns a string envelope by default, and
  `wake_reply_contract_again_sets_has_pending` proves `again` through the
  string path (it fails without the parse). A plain-text reply from a
  non-Roym service is now logged as a failure on each call; ADR-0031 §6
  asks for a JSON reply, so I accept that.
- **R2 — Fixed.** The substrate test builds the expected map from the
  embedded manifest through `roym_dispatch_id`, asserts it is not empty,
  and asserts `wake_targets()` equals it with equal `WakeSpec`s. It passes
  with `--features roym`. The text-search test in `roym_core` is removed.
- **R3 — Fixed.** Both tests seed an expired `deciding` row, assert it
  ends `not-formed`, and count exactly one recovery query on `LEDGER`. The
  fix note says each test failed with its recovery call removed.
- **R4 — Partly fixed, Open.** The event test now lets the first call
  finish before it records the event, and every test named in round 1
  exists. But `wake_reply_string_envelope_with_warnings` and
  `wake_reply_string_envelope_with_error` assert only that no second call
  happens. A reply with no `again` gives no second call whether `warnings`
  and `error` are handled or not, so both tests pass with either branch
  deleted. *Fix:* test `wake_reply` directly (it is a pure function): a
  string envelope gives its `result`, a string with `error` gives `Err`, an
  object gives itself, a string that is not JSON gives `Err`. Move the
  `warnings` read into a pure function (for example `fn
  reply_warnings(&Value) -> Vec<&str>`) and test it the same way.
- **R5 — Fixed.** The lookup now runs before the bus read and before
  `has_pending` is taken, so a trigger waits on the bus while the endpoint
  is missing. `last_run_at` does not move, so the call runs on the first
  tick after the endpoint returns. The warning is logged once per missing
  period. Two tests cover the event trigger and the `has_pending` trigger.
- **R6 — Fixed.** The linked build records `nativehost`. `restart` answers
  "native-host service and has no restart path" and `status` reports
  `Unknown` with the native-host reason; both are tested and the Deviation
  is recorded. The third side effect, certificate renewal, now passes its
  "is deployed" gate for `did:key:roym-*`, but step (3) of
  `verify_installed_instance_cert` still refuses it, because the
  certificate must be signed by the `master_did` key and no key exists for
  a `did:key:roym-*` id. So no native-dispatch entry can be replaced. I
  accept it.
- **R7 — Fixed.** `poll_events` treats an empty cursor like `reload`: it
  takes the head and starts a rescan. `TestHost::read_events(None)` now
  answers like the real bus (no events, the head cursor), and the tests
  that need events seed a cursor first.
- **R8 — Fixed.** The test is renamed `stopped_quote_author_restarts_and_countersigns`
  and its comment no longer claims the reload path.
- **R9 — Fixed.** Both sentences are back on `save_deploy_facts`.
- **R10 — Partly fixed, Open.** The wake example now matches `roym.toml`,
  and the limits and the reply contract are documented. But the new
  section 6.7 text says `transaction.sync` "returns `{scanned, cards_filed,
  more}`". `SyncOutput` has no `cards_filed`; it has `scanned`, `filed`,
  `refused`, `unknown`, `countersigned`, `deferred`, `unsent_failures`,
  `last_seq` and `more`. It also says `more` is true "when more unread
  messages remain beyond the sync window"; the code sets it only when the
  window was full and `last_seq` grew. *Fix:* list the real fields, and
  give `more`'s exact rule.
- **R11 — Fixed.** Both backlog rows have their ADR-0031 note.
- **R12 — Fixed.** The status row gives each count with its command; the
  counts I repeated match.
- **R13 — Fixed** in the files named in round 1, and the helper is one
  function in `fixtures.rs`. The fix commits add new inline paths
  elsewhere (R15).

**Counts of this round's new findings: 0 blocker, 1 major, 1 minor.
Open after this round: 4 (R4, R10, R14, R15).**

### R14 — CI is red: the method-contract test reads two event kinds in a test as method names (major)

*Where:* [tests/drive.rs:206](../../../../crates/roym_transaction/src/app/tests/drive.rs#L206)
and `:207`, at `b5c8bf43`.

*Problem:* `every_transaction_method_has_typed_input_and_output` scans every
`.rs` file under `crates/roym_transaction/src`, tests included, and refuses
any string literal that routes to `transaction` but is not in the method
table ([check.rs:136](../../../../crates/roym_core/src/method/check.rs#L136)).
`drive_ignores_events_of_transaction_itself` writes the event kinds as
literals: `push_event("transaction.card", …)` and
`push_event("transaction.progress", …)`. The router sends `transaction.*`
names to `transaction`, so the check reads them as methods that are
missing from the table. This has failed since `2a919468`. I missed it in
round 1 because CI was still pending then. I ran the test locally on
`b5c8bf43` and got the same two lines as CI.

*Failure case:* CI test shard fails on every push of this branch
(`contract::every_transaction_method_has_typed_input_and_output`, 3 tries),
so the PR cannot merge.

*Fix:* use the constants that production code already uses:
`transaction::EVENT_CARD` and `transaction::EVENT_PROGRESS` from
`syneroym_roym_core::transaction`
([transaction.rs:38](../../../../crates/roym_core/src/transaction.rs#L38)).
The scanner then sees no literal. Do not add an exception to the scanner.
Then run `cargo nextest run -p syneroym-roym-web --test method_contract`.

*Status:* Fixed (3e636bbd).
`drive_ignores_events_of_transaction_itself` pushes the two event
kinds through `transaction::EVENT_CARD` and
`transaction::EVENT_PROGRESS`, the same constants production code
records the events with, so the scanner sees no string literal and
no scanner exception was added. Proof: `cargo nextest run -p
syneroym-roym-web --test method_contract` failed at HEAD before the
fix with the same two lines as CI (`drive.rs:206` and `:207`: string
literal routes to transaction but is not in the method table);
after the fix the whole suite passes (38 passed), and the drive test
itself still passes.

### R15 — New inline qualified paths, and a pointer comment cut in two (minor)

*Where:* [runtime/tests.rs:126](../../../../crates/substrate/src/runtime/tests.rs#L126)
to `:147`; [wake/tests.rs:293](../../../../crates/substrate/src/runtime/wake/tests.rs#L293)
and `:312`; [services/tests.rs:259](../../../../crates/roym_core/src/services/tests.rs#L259)
to `:261`.

*Problem:* The fix commits add `syneroym_app_orchestration::SynAppManifest::from_toml`,
`std::collections::HashMap<String, syneroym_app_orchestration::WakeSpec>`
(twice) and `tokio::sync::Notify` as inline types. AGENTS.md asks for types
through `use`. The new pointer comment in `services/tests.rs` splits the
path across two lines (`runtime/tests.` / `rs::the_linked_…`), so a search
for the test name or the file path does not find it.

*Failure case:* none at run time; the import rule is not met, and the
comment cannot be searched.

*Fix:* import `HashMap`, `SynAppManifest`, `WakeSpec` and `Notify`. Put the
pointer on one line (a long line in a comment is fine), or drop it.

*Status:* Fixed (638586f5).
`HashMap`, `SynAppManifest` and `WakeSpec` are imported in
`runtime/tests.rs` and `Notify` in `wake/tests.rs`; no inline
qualified type path remains in either. The linked-build pointer in
`services/tests.rs` now keeps the test name and the file path each
intact on their own comment lines, so a search for either finds it.
A literal single line is not possible under this repo's fmt: the
nightly `wrap_comments` re-splits any comment line holding the
backticked `tests.rs::the_linked_…` token as one long word — the
re-split was verified by running fmt on a one-line pointer — so the
pointer uses plain words, which fmt wraps only between words.

## Code review round 3 — Claude Opus 5.5 (Claude Code), 2026-11-24

This round checks the fixes of round 2 in `3e636bbd`, `27e29d31`,
`02196adc` and `638586f5` (closed in `b5b38a86`). Branch
`feat/m08c-c3c-agree-as-listed` at `b5b38a86`, main checkout, up to date.
This time the fix session changed only the *Status* lines.

**Tests I ran:** `cargo nextest run -p syneroym-roym-web --test
method_contract`: 38 passed (it failed on `b5c8bf43`). `cargo nextest run
-p syneroym-substrate --lib wake`: 28 passed. `cargo nextest run -p
syneroym-roym-transaction drive`: 15 passed. `cargo nextest run -p
syneroym-substrate --features roym --lib the_linked_build_registers`: 1
passed. `cargo clippy -q -p syneroym-substrate -p syneroym-roym-transaction
-p syneroym-roym-core --all-targets --all-features -- -D warnings`: clean.

**CI.** `gh pr checks 381` on `b5b38a86` at review time: only the CLA check
has finished (pass); lints, the six test shards and Playwright E2E are
pending. So CI is not yet proof for this head.

### Fixes checked

- **R4 — Fixed.** `wake_reply` is tested directly in four cases (a string
  envelope gives its `result`, a string with `error` gives `Err`, an
  object gives itself, a string that is not JSON gives `Err`). The
  warnings read is the pure `reply_warnings`, and the worker now logs
  exactly what it returns; three tests cover order, entries that are not
  strings, and no array. Each test fails if its function ignores the case.
- **R10 — Fixed.** Section 6.7 lists the nine real `SyncOutput` fields and
  gives `more`'s rule (a full window of 500 messages, which is
  `SYNC_WINDOW`, and `last_seq` grew).
- **R14 — Fixed.** `drive_ignores_events_of_transaction_itself` uses
  `EVENT_CARD` and `EVENT_PROGRESS`; the scanner has no new exception; the
  method-contract suite passes locally.
- **R15 — Fixed.** `HashMap`, `SynAppManifest`, `WakeSpec` and `Notify`
  are imported. The pointer comment keeps the test name and the file path
  each whole, so a search finds both. The fix note explains why it is not
  one line (nightly `wrap_comments`); I accept that.

**Counts of this round's new findings: 0. Open: 0.** All findings R1 to
R15 of C3c1 are fixed and verified. The review of C3c1 is closed, subject
to CI passing on `b5b38a86`.

## Code review round 4 — Claude Opus 5.5 (Claude Code), 2026-11-24

This is the first code review of **part C3c2** (agree as listed on both
nodes): plan section 6.2 and D-C3c-5 to D-C3c-18. PR #382 (draft), branch
`feat/m08c-c3c2-agree-as-listed` at `a63ee51d`, main checkout, not a
worktree, `git pull` up to date. Diff: `git diff origin/main...HEAD`
(`main` at `b260da71`, 113 files). The code was written by Gemini 3.8
Flash. The owner's prompt called this round 2 (the second part of the
slice). This file already has rounds 1 to 3 of C3c1, so I number it 4. I
did not check C3c1's items again. New findings continue at R16.

**CI.** `gh pr checks 382` on `a63ee51d`: **"Lints, licenses & checks"
failed** (R16). fmt, clippy with `-D warnings`, `cargo audit`, `cargo deny`
and `check-roym-deps` passed before it. The six test shards and
Playwright E2E were still pending. So CI is not proof that the suite
passes.

**Tests I ran** (targeted only): `cargo nextest run -p syneroym-roym-core
-p syneroym-roym-transaction`: 735 passed, 0 skipped (status.md says 735).
`cargo xtask check-file-lengths`: fails as in CI. `check-module-layout`,
`check-lint-suppressions` (38 of 38), `check-dep-versions`,
`check-change-docs`, `check-duplication` (6.80%): pass. Two throwaway
tests, removed again, prove R17 and R18 (outputs quoted there). I did not
build the roym WASM, so I did not run parity, and I did not measure the
component sizes again. The disk of this machine filled up at the end of
the review (116 MB free), so I stopped building.

### Checked and found correct

- **The current version and the consent (owner's side).** `start`
  ([answer.rs:218](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L218))
  reads the offering with `listing.get` from `catalog` each time, declines
  with `offering-withdrawn` when it is missing or not active, declines with
  `consent-expired` when `now > valid_until_secs`, and builds the terms
  from the current version (the consent's `record_id` is replaced by the
  current one). The `catalog` envelope is verified again. Tests:
  `a_withdrawn_offering_…`, `a_missing_offering_…`,
  `an_expired_consent_…`.
- **One tap, at most one quote, in the normal path.** `claim_answer` is a
  `create` of `answer:<enquiry>` with the quote id fixed in it, before any
  signature. A loser signs nothing. An `answering` row whose quote is held
  is marked `answered` and the card is sent again. But see R18: the
  driver never comes back to an `answering` row.
- **The slot check only in the answer.** `check_author_slots` runs only
  from `evaluate`; `hooks::prepare` is now a thin wrapper over
  `prepare_window`; `set_at` keeps `prepare_terms` and then calls
  `issue_quote`. I compared the moved body with `main`: the same
  validation, error data, signing, pointer and send, in the same order,
  plus `formed_as` on the pointer. So `quote.set` runs the hook once, as
  before. The plan's guard test `quote_set_runs_the_prepare_hook_once`
  does not exist (R22).
- **`in_flight_instant` uses `is_deal_ended`.** So expiry, a decline
  (`declined_at_secs` or `decline:<quote>`), a `not-formed` decision and a
  terminal agreement free a place. But it does not leave out a quote that
  formed an agreement (R17).
- **A listing with no new field signs the same bytes.** The four fields
  are skipped at their defaults (`acceptance`, `not_limited`,
  `instant_cap = 20`, `deal_terms`).
  `a_listing_without_the_new_fields_signs_the_same_bytes` and
  `a_decline_with_no_code_keeps_todays_bytes` compare the JSON of an old
  payload after a round trip. That shows no new key is written, which is
  what matters for the signed bytes.
- **`terms_from_listing`** follows the agreements doc's table, field by
  field, and the refusal order of D-C3c-6. The two differences
  (`ExtensionUnknown`, and a `details` block for a known non-scheduling
  extension) are in the Deviations. Unit tests for each row, each refusal,
  the order, the largest listing and the amount text exist.
- **The listing rules** (`instant-not-allowed` with its two reasons,
  `instant-cap-range`, `deal-terms-languages`, `quote-valid-range`) and
  `{code, reason}` in `listing.set`'s error data.
- **`formed_as`**: required in the snapshot, given to `open` at the four
  call sites, copied from the writer's quote pointer, and
  `formed-as-changed` refuses a changed value on the counterparty's node.
- **Backup**: `SCHEMA_VERSION` 7 → 8; the `as_listed` section is exported
  and imported; the import checks `answer:` rows (held enquiry; an
  `answered` row names a held quote that answers it, with the same quote
  id) and `autoaccept:` rows (held enquiry, held quote that answers it).
  Tests `the_import_checks_answer_rows`, `the_import_checks_autoaccept_rows`,
  `the_import_checks_as_listed_rows` exist.
- **The native build binding.** `roym.toml` adds `catalog` to
  `transaction`'s `depends_on`; `ROYM_BINDINGS` lists the six edges and
  `every_manifest_dependency_has_a_binding` checks both directions. The
  removed string check in `services/tests.rs` is covered by this test plus
  the remaining hook-service check.
- **Both builds' method table.** `quote.from-listing` and
  `deal.as-listed-status` are in the table with the default
  `require_internal`; the contract count is 47.
- **`same_terms`** compares `DealTerms` with `PartialEq` and replaces
  only the expiry, which must be after `now` and at most 7 days after
  issue. The node checks the quote's issue time against the consent, the
  enquiry's decline, the latest version and the disclosures before it
  takes the fence and signs.
- **`directory`**: `can_agree_as_listed` comes from `as_listed_ready` and
  links no deal code. The implementer measured +37,513 bytes (limit
  +40 KB); I did not measure again.
- **Hindi**: 46 new keys, the same set in `en.ftl` and `hi.ftl`. The owner
  has not reviewed them yet (status.md says so); that is the owner's step,
  not a finding.

**Counts of this round's new findings: 3 blocker, 7 major, 7 minor.
Open: 17 (R16 to R32).** R31 was added after the first write of this
round, when uncommitted changes from the implementation session showed
it. R32 was added when the CI test shards on `731d0835` finished.

### R16 — CI is red: `backup/sections.rs` is over the file limit (blocker)

*Where:* [backup/sections.rs](../../../../crates/roym_transaction/src/app/backup/sections.rs),
865 production lines at `a63ee51d` (703 on `main`).

*Problem:* `cargo xtask check-file-lengths` fails: "865 production lines
(maximum allowed is 800)". The plan (D-C3c-14) puts the `as_listed`
section in a new `backup/as_listed.rs`. The code put it, and the two new
ledger validators, into `sections.rs`.

*Failure case:* CI "Lints, licenses & checks" fails on every push, and the
checks after it (module layout, duplication, change docs) do not run in
CI. The PR cannot merge.

*Fix:* move `validate_as_listed`, `validate_answer_entry` and
`validate_auto_accept_entry` into `backup/as_listed.rs`, and call them from
`sections.rs`. Do not raise any limit.

*Status:* Fixed (3fb867b2). The three validators are in
`backup/as_listed.rs`; `sections.rs` imports and calls them. No limit was
raised. Proof: `cargo xtask check-file-lengths` now passes (694 production
files at or under 800 lines; `sections.rs` 732, `as_listed.rs` 150).

### R17 — The instant cap counts an accepted instant deal twice (major)

*Where:* [answer.rs:325](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L325)
to `:345` (`in_flight_instant`) and `:347` to `:377` (`holds_one`).

*Problem:* D-C3c-12 counts an `answered` row as in flight only while "its
quote has no `decision:` fence yet and has not ended". The code checks
only `is_deal_ended`. A quote that formed an agreement has not ended. So
it is counted once in `in_flight_instant` and once more in `open_instant`.
For the same reason, an agreement with a filed half, which `open_instant`
leaves out, still counts in flight until its quote's expiry, and
`holds_one` stays true for that counterparty.

*Failure case:* proved with a throwaway test: cap 2; tap 1 is answered;
an agreed instant agreement row for that quote is stored. Output:
`open=1 in_flight=1`, and a second person's tap waits with
`cap-reached`. So a cap of N allows about N/2 deals. The planned tests
that would show it (`many_taps_before_any_acceptance_make_only_cap_quotes`,
`an_agreement_with_a_filed_half_no_longer_counts`,
`a_declined_instant_quote_frees_its_place`) do not exist. The existing
`the_cap_makes_an_instant_offering_wait` stores only an agreement row, so
it cannot see the double count.

*Fix:* in both scans, skip a row whose quote has a `decision:<quote_id>`
row (any state), as the plan says; then the agreement is counted only by
`open_instant`. Use one helper for both scans. Add the three tests above,
each with a real `answer:` row, and one that a cap of 2 allows two
accepted deals.

*Status:* Fixed (d7a5b97a, tests c060a853). `in_flight_rows` skips a row
whose quote has a `decision:` row in any state, and both
`in_flight_instant` and `holds_one` use it. Tests:
`many_taps_before_any_acceptance_make_only_cap_quotes` (cap 2 gives two
quotes and a third waits),
`an_agreement_with_a_filed_half_no_longer_counts`,
`an_agreement_older_than_seven_days_no_longer_counts` and
`a_declined_instant_quote_frees_its_place`. Each seeds real `answer:`
rows. Proof: removing the `decision:` skip made
`an_agreement_with_a_filed_half_no_longer_counts` fail (`left: Waiting,
right: Quoted`); restored, it passes.

### R18 — A failure after either fence is never tried again (major)

*Where:* owner: [answer.rs:174](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L174)
to `:207` and `:603` to `:606`;
[status.rs:171](../../../../crates/roym_transaction/src/app/as_listed_ops/status.rs#L171).
Counterparty: [accept.rs:106](../../../../crates/roym_transaction/src/app/as_listed_ops/accept.rs#L106)
to `:129` and `:261` to `:268`.

*Problem:* Both sides take their fence before they sign, as planned. But
the drive pass skips any tap that has a fence row:

- Owner: `answer_waiting` skips an enquiry with any `answer:` row. After a
  crash, or an error from `person_principal` or `issue_quote` after
  `claim_answer`, the row stays `answering` and is never resumed. The
  plan says "the next call runs `issue_quote` again with the same id".
  `deal.as-listed-status` also skips it (`answered` is true for any row),
  so the owner's Hub shows no line and no "Confirm".
- Counterparty: `auto_accept` claims `autoaccept:` and then calls
  `accept_at`. On a signing error the row is set back to `waiting`, but
  `accept_waiting` skips every tap that has an `autoaccept:` row.

*Failure case:* proved with a throwaway test on the counterparty's side:
the certificate is removed, `auto_accept` answers `Waiting`, the fence row
exists; the certificate is put back and `accept_waiting` runs: the tap is
still `waiting` and there is no agreement. The deal waits until the
quote expires. The planned
`a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts` and the
crash case of `two_answers_of_one_tap_make_one_quote` (with the
injected-error host) do not exist; the existing test calls the answer
twice with no failure.

*Fix:* owner: `answer_waiting` skips only `answered` and `declined` rows,
and resumes an `answering` row with its stored quote id and `formed_as`,
with no new evaluation and no decline (a decline now would overwrite the
fence of a quote that another sync may be signing). Store the sequence in
the row too, so a resume signs the same sequence that the quote id was
derived from. `deal.as-listed-status` shows an `answering` row. Counterparty:
`accept_waiting` does not skip on the fence; when a fence exists, it
calls `auto_accept` with the fence's quote, which `claim` already accepts
as `Held`. Add the two tests.

*Status:* Fixed (owner d7a5b97a, counterparty cfa75974, tests c060a853).
`answer_waiting` skips only answered and declined rows; an `answering`
row is resumed with its stored quote id, sequence and `formed_as`, with
no second evaluation and no decline. `deal.as-listed-status` shows it as
`answer-in-progress`. `accept_waiting` uses a fence's quote instead of
skipping the tap. Tests:
`an_answer_that_reached_its_fence_is_resumed_with_the_same_quote_id`
(drives through `answer_waiting`) and
`a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts`.
Proof: skipping any answer row made the first fail (`left: Answering,
right: Answered`); skipping any `autoaccept:` row made the second fail
(`left: Waiting, right: Accepted`). Restored, both pass.

### R19 — The automatic acceptance does not check that the listing owner signed the quote (major)

*Where:* [accept.rs:88](../../../../crates/roym_transaction/src/app/as_listed_ops/accept.rs#L88)
to `:101`.

*Problem:* D-C3c-17 builds the expected terms with
`terms_from_listing(listing, quote.issuer, …)`. Inside it,
`derive_listing_id(owner_did, slug) == listing_id` is what ties the
quote's signer to the listing's owner. The code passes the listing
envelope's own issuer, so this check is always true, and nothing else on
this path compares the quote's issuer with the listing's issuer
(`file_quote_card` checks only that the quote's counterparty is the
enquiry's issuer). Plan 7.11 lists "who signed is not checked" as a
finding kind to avoid.

*Failure case:* a quote signed by another key Z (not the listing owner)
that copies the listed terms and answers the tap reaches the conversation.
Y's node finds the terms equal and signs an acceptance half that binds Y
to Z, with no person at the screen.

*Fix:* pass `quote.verdict.issuer` (show the quote with `not-complete`
when it is missing). Add a test: the same terms in a quote signed by
another identity give `Shown`.

*Status:* Fixed (cfa75974, test c060a853). The expected terms use
`quote.verdict.issuer`, and a missing issuer shows `not-complete`. Test
`a_quote_signed_by_another_identity_is_shown`. Proof: with
`verdict.issuer` in that place the test failed (`left: Accepted, right:
Shown("not-complete")`); restored, it passes.

### R20 — The tap screen records a viewing that did not happen (major)

*Where:* [as_listed_tap.ts:96](../../../../crates/roym_web/ui/src/screens/as_listed_tap.ts#L96)
to `:145`.

*Problem:* The screen always shows the original texts
(`terms.cancellation_terms.text`, and so on), never a translation. But it
sends `viewed_lang = currentLanguage()` and computes `viewed_via_mt`
from whether the listing has a translation in that language. The
automatic acceptance signs these two values into the agreement receipt
(ML-18). The machine-translation warning is shown after the terms, and
the person does not confirm it; D-C3c-18 says it comes first and the
person confirms it.

*Failure case:* a Hindi Hub opens an English listing with a Hindi
translation: the person reads English, and the signed half says
`viewed_lang: hi, viewed_via_mt: false`. With no Hindi translation, the
person reads the English original, and the half says
`viewed_via_mt: true`, though no machine translation was shown.

*Fix:* show each text in the language the Hub picks (the listing's own
translation when there is one, else the original or a machine translation
with the existing `mtWarning`), and send the language actually shown and
`viewed_via_mt` only when a machine translation was shown. Put the
warning first, with a confirm that enables "Agree and send". Add a Hub
unit test for each of the three cases.

*Status:* Fixed (88daea35). The panel shows the author's translation, the
original, or a machine translation. `viewed_lang` and `viewed_via_mt`
are what it shows. The machine warning comes first, and its confirm
enables the tap. Tests cover the original (`shows every term and sends
the consent with no description`), an author translation
(`shows the author's translation and signs that language`) and a machine
translation (`shows a machine translation first, and signs it after the
person confirms`). Hub tests: 240 passed in the focused run.

### R21 — The struct test does not fail when a field is added (major)

*Where:* [as_listed_accept.rs:316](../../../../crates/roym_transaction/src/app/tests/as_listed_accept.rs#L316)
(`a_changed_field_anywhere_in_the_terms_fails_the_check`).

*Problem:* The plan (AG-44, D-C3c-17) and the exit criterion ask for a
test that serialises a `DealTerms` and a `Commitment` with every field
set, walks the JSON keys, and fails when a key has no row in the test's
table. The test is a hand-written list of 12 changes. It reads no keys.
It also leaves out `home_exchange`, `commitment_id`, `giver`,
`resource_kind`, `kind`, `extension`, `details`, `window`, `location`,
`payment_methods`, `tax_minor`, `eep` and the translations.
`a_quote_that_differs_in_any_field_is_shown` changes only the payee.

*Failure case:* a new optional field in `Commitment` that
`terms_from_listing` does not fill: the test still passes. The exit
criterion "checked once by adding a field locally" cannot be met.

*Fix:* build the test as the plan says: one value with every field set,
a fixed table of `(json path, change)`, a check that every key of the
serialised value has a row, and for each row a check that
`same_terms` is false.

*Status:* Fixed (c060a853, 05847f22). `full_terms` sets every field, `keys` walks
the serialised keys (not into the two free-form blocks), and `TERM_FIELDS`
is the fixed table. Each named field has a typed change, and each change
must make `same_terms` false. Proof: removing `home_exchange` from the
table failed the coverage check with the key in the left set and not the
right; restored, it passes.

### R22 — Planned tests and step 8 are not built, and most of it is recorded nowhere (major)

*Where:* the test files under
[app/tests/](../../../../crates/roym_transaction/src/app/tests/),
[dual_build_parity/as_listed.rs](../../../../crates/roym_web/tests/dual_build_parity/as_listed.rs),
`crates/substrate/tests/`, `crates/substrate/tests/e2e/demo/`;
[change.md:1502](../../../changes/roym-agreements/change.md#L1502).

*Problem:* The slice is marked "Implemented", but these planned items do
not exist (I checked with `git grep -n 'fn <name>'`):

- Owner (section 7.3): `a_changed_offering_is_answered_from_the_current_version`,
  `an_agreement_with_a_filed_half_no_longer_counts`,
  `an_agreement_older_than_seven_days_no_longer_counts`,
  `many_taps_before_any_acceptance_make_only_cap_quotes`,
  `one_counterparty_tapping_twice_gets_one_instant_quote`,
  `a_declined_instant_quote_frees_its_place`,
  `a_full_slot_ends_not_formed_with_slot_taken`,
  `a_removed_slot_is_declined_with_the_hooks_code`,
  `an_unreachable_slot_hook_makes_the_enquiry_wait`,
  `quote_set_runs_the_prepare_hook_once`,
  `formed_as_is_the_same_in_the_first_and_a_later_snapshot`,
  `cancel_of_unconfirmed_instant_agreements_frees_their_slots`, and the unit
  test of `is_open_instant` (one case per condition).
- Counterparty: `a_missing_disclosure_shows_the_quote`,
  `a_withdrawn_enquiry_is_never_accepted`,
  `a_quote_from_a_changed_offering_is_shown`,
  `a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts`,
  `an_expiry_more_than_seven_days_after_issue_is_shown`.
- The substrate native test that calls `listing.get` from `transaction`
  (P11), parity scenario 253 (the driver answer), 254 and 255, the owner's
  half of `roym_as_listed_e2e.rs` (step 5) and the whole file (step 8),
  the Playwright cases, story 50, the Hub thread lines, and the form's
  translation rows.

The backlog and the Deviations name only the two-node file, 254/255, the
Playwright cases, story 50, the thread lines and the translation rows.
The missing service tests, scenario 253 and the P11 test are named
nowhere. The Deviation says 254 and 255 "need two nodes, which the parity
harness does not have". That is not so for 254: `quote.from-listing` needs
only the owner's node with a held enquiry, as the service tests already
build. It also says "the plan's 252 to 256 were taken", but 252 is C3c1's
own scenario and 256 is used here with the plan's meaning. The two bugs
of R17 and R18 are exactly the cases these missing tests cover.

*Failure case:* the visible story of the slice (an instant offering
reaching `agreed` from one tap) is not shown anywhere end to end, and
the exit criteria of section 10 for C3c2 cannot be ticked.

*Fix:* build them in this PR; the plan puts all of them in C3c2. A cut
of any of them needs the owner's yes, and then a row in the backlog and
a correct Deviation. Correct the scenario-number sentence.

*Status:* Deferred ([backlog row](../../deferred-backlog.md#12-roym-transaction-cards-agreement-and-what-m06c-c7-left-open)). The service tests that cover the
two bugs were added in c060a853: the cap with real `answer:` rows, a
filed half, an old agreement, a declined quote and one counterparty
(R17), and the resumed answer and the retried acceptance (R18), plus the
other-issuer test (R19) and the terms struct test (R21). The rest is in
the one backlog row: `roym_as_listed_e2e.rs`, parity 253, 254 and 255,
the substrate native `listing.get` test, the Playwright cases and story
50. The scenario-number sentence is corrected in fcfea327: 253 and 254
need one node, and only 255 needs two. This cut needs the owner's yes;
it is recorded with that pickup trigger.

### R23 — No living doc is updated, and two change docs miss their Deviations (major)

*Where:* the diff touches only `roym-agreements/change.md`,
`deferred-backlog.md` and `status.md` under `docs/`.

*Problem:* AGENTS.md's living-docs close-out and plan section 9. Now
false or incomplete: `developer-guide.md` (the `transaction` method table
lacks `quote.from-listing`, `deal.as-listed-status` and `enquiry.set`'s
`as_listed`; the listing's new fields; the `answer:` and `autoaccept:`
ledger kinds; the `as_listed` backup section and schema version 8);
`system-architecture.md` (agree as listed in the Roym deal flow);
`system-requirements-spec.md`; `roym-integrated-experience-spec.md` (steps
C12 and C14, the listing fields, the hit's action); `TERMINOLOGY.md`;
`traceability-matrix.md`. The plan also asks for Deviations in
`roym-value-exchange` (VX-54 with the slot gap of Q4) and
`roym-extensions` (EX-18's agree-as-listed half). Neither is touched.

*Failure case:* a developer who reads the developer guide does not know
the two methods or the new ledger kinds that a backup import refuses.

*Fix:* update the listed docs in this PR, without milestone or slice ids,
and add the two Deviations rows in the doc that owns each.

*Status:* Fixed (fcfea327). The developer guide has the listing fields,
the `as_listed` enquiry block, the two methods, the `answer:` and
`autoaccept:` ledger rows and backup schema 8. The architecture,
requirements, experience spec, terminology and traceability matrix
describe the flow and the hit's action. The value-exchange and extensions
changes record their deviations and their front matter names the touched
docs. `cargo xtask check-change-docs` passes.

### R24 — Error kinds are read from message text (minor)

*Where:* [accept.rs:121](../../../../crates/roym_transaction/src/app/as_listed_ops/accept.rs#L121)
to `:129` and `:240` to `:244`.

*Problem:* `is_signing_error` decides by `message.starts_with(…)`, and any
other error stores the whole message as the row's `reason`. Plan 7.11:
"`auto_accept` branches on the error's code, never on its message". The
Hub cannot map a free message to a sentence.

*Failure case:* a change of wording in `certificate_error` makes a signing
error a permanent `shown` with an English message as its reason.

*Fix:* put a `data.code` on the certificate errors (as `listing.set` does
for its codes), branch on it, and store only a code as `reason`.

*Status:* Fixed (d7a5b97a, cfa75974). `resolve_principal_and_owner` and
the as-listed certificate map add `data.code` (`signing-not-enrolled`,
`signing-certificate-expired`, `signing-certificate-stale`,
`signing-no-owner`). `auto_accept` branches on `error_code`, and a
refusal with no code is stored as `accept-failed`, never as a message.
Test `a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts`
covers the signing code path.

### R25 — The owner's answer does not check whose listing and whose enquiry (minor)

*Where:* [answer.rs:122](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L122)
to `:139`, `:248` to `:257`.

*Problem:* The plan's first line is "require: this node's owner issued
the listing … and the enquiry's counterparty is not the owner". The code
does neither check. It relies on `catalog` holding only this node's
listings, and treats "not in the catalog" as withdrawn.

*Failure case:* any peer sends an as-listed enquiry with a made-up
`listing_id`; the owner's node signs and sends a `deal-decline` with
`offering-withdrawn` for an offering it never had. `quote.from-listing` on
the owner's own sent enquiry follows the same path.

*Fix:* refuse (`NotAsListed`, nothing signed) when the enquiry is this
node's own (`mine`, or issuer equal to the owner), and check
`current.owner` against the node's owner from `resolve_principal_and_owner`.

*Status:* Fixed (d7a5b97a). `answer_as_listed` refuses when the enquiry's
issuer is this node's owner, and `start` refuses when the catalog's
listing owner is not this node's owner. Both return `NotAsListed` and
sign nothing.

### R26 — Wrong or missing values in what the owner and the person see (minor)

*Where:* [answer.rs:174](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L174)
to `:183`; [status.rs:143](../../../../crates/roym_transaction/src/app/as_listed_ops/status.rs#L143);
`as-listed-cap-reached` and `as-listed-not-complete` in
[en.ftl](../../../../crates/roym_web/ui/src/i18n/en.ftl).

*Problem:* (1) A sync that loses the `answer:` fence to an `answering` row
returns `Waiting("confirm-first")`, which is not the reason. (2) The status
builds its consent with `record_id: current.payload.listing_id`, a listing
id in a record-id field (no effect today, but wrong). (3)
`as-listed-cap-reached` has no number; D-C3c-16 says "its limit of N open
instant agreements". (4) The counterparty's tap screen shows
`as-listed-not-complete`, "… Answer with a quote.", which speaks to the
owner. (5) The cancel action has no confirm dialog that lists the
agreements (D-C3c-16).

*Failure case:* the owner reads "confirm first" for an instant offering;
a person who taps is told to answer with a quote.

*Fix:* return a separate in-progress outcome; use the current record id;
pass `$cap` into the message; give the tap screen its own message; add the
dialog. The owner reviews the Hindi of any changed message.

*Status:* Fixed (d7a5b97a, 88daea35). A lost fence returns
`answer-in-progress`; the status uses the current record id; the cap
sentence takes `$cap` on the listing page and in the enquiry line; the
tap screen has `as-listed-tap-not-complete`; and the cancel action opens
a dialog that lists the agreements. Hindi is updated in both catalogs
and needs the owner's review: `as-listed-answer-in-progress`,
`as-listed-accept-failed`, `as-listed-tap-not-complete`,
`as-listed-cancel-note`, `as-listed-cancel-confirm`,
`as-listed-cancel-item`, and the changed `as-listed-cap-reached`.

### R27 — Inline qualified paths (minor)

*Where:* for example [as_listed.rs:99](../../../../crates/roym_core/src/as_listed.rs#L99)
(`crate::deal::ExtensionRef`),
[listing/as_listed.rs:175](../../../../crates/roym_core/src/listing/as_listed.rs#L175)
and `:179` (`std::collections::HashSet`, `crate::as_listed::translation_langs`),
`super::super::AS_LISTED` three times in `accept.rs`,
`syneroym_roym_core::record::Envelope` in `answer.rs` (`expiry_of`),
`syneroym_roym_core::as_listed::as_listed_ready` in
`roym_federated_query/src/verify.rs`, `crate::app::enquiry_ops::as_listed::AsListedRow`
in `sync/cards.rs`, and `crate::progress::FormedAs::Quote` and
`crate::deal::ExtensionRef` in several tests.

*Problem:* AGENTS.md's import rule (types through `use`, no inline paths
with several `::`).

*Failure case:* none at run time.

*Fix:* import the types and constants; call functions through their
parent module.

*Status:* Fixed (950e02c6). `ExtensionRef`, `HashSet` and
`translation_langs` are imported in the core files; `AsListedRow`,
`Envelope`, `DecisionRow` and `LedgerKind` are imported where they are
used; `super::super::AS_LISTED` and `crate::app::enquiry_ops::as_listed`
are gone. The tests import `FormedAs` and `ExtensionRef`.

### R28 — Plan placements, unused states, missing shapes and repeated helpers (minor)

*Where:* [listings.ts](../../../../crates/roym_web/ui/src/screens/listings.ts)
(693 → 810 lines);
[enquiry_ops/as_listed.rs:83](../../../../crates/roym_transaction/src/app/enquiry_ops/as_listed.rs#L83)
to `:87`; `crates/roym_web/tests/method_shapes/`; `answer.rs` and
`status.rs`; the three new test files.

*Problem:* (1) D-C3c-16 puts the form section in a new
`listings/as_listed_section.ts` because `listings.ts` was 693 lines; it
went into `listings.ts`, now 810. (2) `AsListedState::Answering`,
`Answered` and `Declined` are never used. (3) Step 4 asks for shape
files for the two new methods; there are none. (4)
`load_current_listing` (`answer.rs`) and `load_current` (`status.rs`) are
the same function; the two `answer:` scans in `in_flight_instant` and
`holds_one` are the same loop; `as_listed_owner.rs`,
`as_listed_accept.rs` and `as_listed_enquiry.rs` each define their own
`lang`, `text`, `deal_terms`, listing and `sign_listing` helpers.

*Failure case:* none at run time; the next change to one copy misses the
others.

*Fix:* move the section to its own file; remove the unused states; add
the two shape files; keep one `load_current_listing` and one scan; put
the test helpers in one shared test module.

*Status:* Fixed (88daea35, cfa75974, c060a853, c1ad860b, 05847f22). The form section is in
`listings/as_listed_section.ts` and `listings.ts` is 745 lines. The three
unused `AsListedState` variants are gone. `deal_as_listed_status_missing`
and `quote_from_listing_missing` shapes exist. The two answer scans are
one `in_flight_rows`, and the test helpers are in
`tests/as_listed_fixture.rs`, shared by the three as-listed test files.

### R29 — Each drive does work that grows with all held records (minor)

*Where:* [answer.rs:582](../../../../crates/roym_transaction/src/app/as_listed_ops/answer.rs#L582)
to `:611`; [accept.rs:248](../../../../crates/roym_transaction/src/app/as_listed_ops/accept.rs#L248)
to `:283`;
[enquiry_ops/as_listed.rs:198](../../../../crates/roym_transaction/src/app/enquiry_ops/as_listed.rs#L198).

*Problem:* `answer_waiting` reads and verifies the signature of every
held enquiry on every drive, before it looks at the cheap `answer:` row.
`accept_waiting` reads the whole `quotes` collection once per waiting
tap. A waiting confirm-first tap makes a `catalog` call and a `prepare`
hook call on every drive until its consent ends (48 hours by default).
`conversation_peer` reads only the first 500 direct conversations.

*Failure case:* a node with a few thousand old enquiries verifies them
all on each wake; a person with more than 500 conversations cannot tap
(`as-listed-wrong-conversation`).

*Fix:* read the `answer:` row first and skip `mine` and non-as-listed
rows before verifying (or keep a small index of open taps); query quotes
with a `request_record_id` filter; page `conversation.list` or ask for the
one conversation.

*Status:* Fixed (d7a5b97a, cfa75974). `answer_waiting` reads the `answer:`
row and skips `mine` before any signature check, and looks at the stored
payload's `as_listed` before `verify_enquiry`. `accept_waiting` asks for
the quotes of one enquiry with a `request_record_id` filter, and
`conversation_peer` pages `conversation.list`. The catalog read and the
prepare hook on a waiting confirm-first tap stay on each drive: the plan
requires the current version and its slot to be checked before the fence,
so a removed slot is declined at the next drive in both modes (D-C3c-12,
P20).

### R30 — status.md and the backlog table are wrong (minor)

*Where:* [status.md](status.md), C3c2 row at `a63ee51d`;
[deferred-backlog.md:681](../../deferred-backlog.md#L681).

*Problem:* The C3c2 row changed "Plan by" from Claude Opus 5.5 to
"DeepSeek V4.1 Flash" and left "Implemented by" as "—" (the owner says
Gemini 3.8 Flash wrote the code). In the backlog, an empty line comes
before the four new rows, so they are not part of the table and render as
broken text.

*Failure case:* the record of who did each step is wrong; the four
backlog rows are hard to read.

*Fix:* put back "Claude Opus 5.5" as the plan author and name the
implementer; remove the empty line. (This round sets the row's Step and
code-review columns.)

*Status:* Fixed (fcfea327, 6bff5511). The C3c2 row names
Claude Opus 5.5 as plan author and Gemini 3.8 Flash as implementer, and
the Step is "Fixes done". The empty line before the four backlog rows is
gone.

### R31 — The Hub cannot save a listing without the three as-listed texts (blocker)

*Where:* [editor.ts:286](../../../../crates/roym_web/ui/src/listings/editor.ts#L286)
(`asListedBlock`) and `:320` at `a63ee51d`;
[listings.ts:534](../../../../crates/roym_web/ui/src/screens/listings.ts#L534)
to `:536`.

*Problem:* `buildSetListingParams` always sends `deal_terms`. The form's
three text fields start empty, so an owner who does not use agree as
listed sends three empty `LocalText` values. `validate_as_listed` checks
each text with `LocalText::validate`, which refuses an empty text. For a
listing with translations, the block's translation set is also empty, so
`deal-terms-languages` refuses it too. The unit test did not see this,
because its `emptyForm()` fills the three texts.

*Failure case:* at `a63ee51d`, any listing saved from the Hub with the
three texts empty fails in `listing.set`. That is every ordinary listing,
and every listing with a translation.

*Fix:* send no `deal_terms` when all three texts are empty, and refuse
the form when only some are filled. The implementation session already
has this change in the main checkout, not yet committed (`editor.ts`,
`editor.test.ts`, one new message `listing-error-deal-terms-required` in
`en.ftl` and `hi.ftl`). I read it: it does this, and `TZ=UTC npx vitest
run src/listings src/i18n src/screens/listings` passes (12 files, 224
tests). Two more points for that change:

- The form still accepts `instant` with no texts. Such an offering can
  never be agreed as listed (`no-deal-terms`), and the owner is not told.
  Refuse `instant` with empty texts in the form, or show a line.
- The owner reviews the Hindi of the new message.

*Status:* Open. The fix is committed as `731d0835` (the same four files I
read). The two points above are still open.

### R32 — CI is red: two parity fixtures build a progress snapshot without `formed_as` (blocker)

*Where:* `peer_signed_progress` in
[deal_fixtures.rs:480](../../../../crates/roym_web/tests/dual_build_parity/deal_fixtures.rs#L480)
to `:484`, used by scenario 157
([booking.rs:577](../../../../crates/roym_web/tests/dual_build_parity/booking.rs#L577));
and the tampered progress row of scenario 142
([transaction_cards.rs:500](../../../../crates/roym_web/tests/dual_build_parity/transaction_cards.rs#L500)
to `:510`).

*Problem:* D-C3c-10 makes `AgreementProgressPayload.formed_as` required,
with no serde default. That is the plan, and the code follows it. But
these two parity fixtures write the payload as hand-made JSON and were
not updated. So:

- Scenario 157: every progress card the helper signs fails to parse.
  The node refuses it, and `agreement.get` has no `seq`. The test stops at
  `booking.rs:591` (`left: Null, right: 1`). The later checks of a wrong
  key and a lower `seq` would also pass for the wrong reason: the card is
  refused for the missing field, not for the key or the sequence.
- Scenario 142: `validate_progress`
  ([sections.rs:756](../../../../crates/roym_transaction/src/app/backup/sections.rs#L756))
  refuses the tampered row with `progress '…': invalid row: missing field
  formed_as`. It never reaches the signature check. The code is still
  `-32602`, but the test asserts "does not verify", so it fails at
  `transaction_cards.rs:532`. The test is meant to prove that a re-signed
  bundle with a forged progress envelope is refused by its signature.
  Without the field it does not test that.

Status.md says only parity scenarios 256 to 259 were run while working,
so the full parity suite never ran on this branch before CI.

*Failure case:* CI on `731d0835`: "Test 1/6" fails
`booking::scenario_157_progress_card_wrong_key_or_lower_seq_refused_parity`
and "Test 5/6" fails
`transaction_cards::scenario_142_transaction_export_import_roundtrip_parity`
(both after three tries). Both fail the same way here after `mise run
build:roym` (`cargo nextest run -p syneroym-roym-web --test
dual_build_parity`, the two tests, `--retries 0`).

*Fix:* add `"formed_as": "quote"` to the JSON in `peer_signed_progress`
and to the `snapshot` of the tampered row in scenario 142. I tried this
in a scratch edit: both tests pass on both builds. I then reverted it. To
find other missed fixtures, run the whole `dual_build_parity` binary after
`mise run build:roym`, not only the new scenarios.

*Status:* Fixed (c060a853). Both fixtures carry `"formed_as": "quote"`.
Proof: removing it from `peer_signed_progress` made scenario 157 fail at
`booking.rs:591` (`left: Null, right: 1`); restored, it passes. After
this change the full `dual_build_parity` binary ran on both builds: 276
passed, 1 skipped.

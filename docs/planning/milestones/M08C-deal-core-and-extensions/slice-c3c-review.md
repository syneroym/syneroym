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

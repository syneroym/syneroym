# M08A slice A2b — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a2b-implementation-plan.md](slice-a2b-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Opus 5.5 (Claude Code), 2026-10-30

Checked against the code at `ba939065` on `feat/m08a-a2b-typed-methods`.
Line numbers below are at that commit. The plan was written by the same
model, so a second reviewer with another tool is still useful.

**Summary:** 0 blocker, 1 major, 9 minor.

What was checked and found correct:

- **Method lists.** `catalog` dispatches 15 names
  ([app.rs:95-115](../../../../crates/roym_catalog/src/app.rs#L95)) plus
  the two certificate verbs: 17. `transaction` dispatches 39 names
  ([app.rs:236-278](../../../../crates/roym_transaction/src/app.rs#L236))
  plus the two certificate verbs: 41. The split of Step 5a (31 rows) and
  Step 5b (10 rows) adds up. Both `invoke` functions run
  `admit::require_internal` first, so both wire lists are empty.
- **Handlers and inputs.** Every struct, field, default and line link in
  section 3.2 matches the code, within one or two lines.
  `booking_view` has nine callers (eight in `booking_ops.rs`, one in
  `agreement_ops.rs:392`). The three `AgreementParam` copies are where the
  plan says. `AgreedTerms` has 14 fields, so `QuoteTermsInput` has 13.
- **Outputs.** The null and absent keys of section 3.3 match the `json!`
  calls, including the two answers of `agreement.accept` and the
  `message_id: null` of the already-recorded answers.
- **Sibling calls.** `call_sibling` writes `"{method}: {e:?}"` for a
  transport error, the same text as `sibling_call` and the direct call in
  `publish_to_source`. The mappings in B33 and in the code snippets of
  Step 4, Step 5a and Step 5b keep today's error codes. Only the text of an
  unreadable reply changes, and only for replies a host never sends.
- **Callers.** No caller sends `quote_expires_at_secs` inside `terms`. It
  appears only in peer-signed quote records built by the parity fixtures
  (`fixtures.rs:305`, `:402`), which are not method inputs. Every
  `*.verify` call in the parity and substrate tests and in `roymctl` sends
  text. Only the Hub's "Verify a listing" box sends an object. The
  Playwright specs call none of these methods directly (one `listing.set`
  body capture, one mocked `transaction.signing-status`). `web` only
  forwards. No other crate calls a catalog or transaction method.
- **The new closed-word keys.** None of the 63 A2a shape files has `role`,
  `pair`, `next`, `payment_timing`, `track`, `writer`, `conflict`,
  `card_type`, `revocation_status`, `payment`, `fulfilment`, `where` or
  `event` with a string value. Step 2's proof ("the A2a files do not
  change") will hold.
- **The check.** `assert_service_table` probes with `{"__probe__": 1}`.
  With `deny_unknown_fields`, serde reports the unknown key before it
  checks for missing fields, so every input with required fields passes
  the probe. The only method-like string literals left in the two crates
  are `"availability.get"` in `transaction`, which routes to `catalog` and
  so is not flagged by the `transaction` scan.
- **Test commands.** `syneroym-roym-directory` has unit tests
  (`app/standing/tests.rs`), so Step 4's nextest run over `catalog` (no
  tests) and `directory` does not fail with "no tests to run".
- **Living docs.** The statements the plan names are at the lines it gives
  (developer guide 1687, architecture 1143, traceability matrix 91-93). No
  other living doc names the typed services.

### Owner questions — reviewer view

**Q1 (unknown keys inside record types).** Agree with (a). (b) doubles about
ten types for a small gain, and (c) breaks reading records from a newer
node. One claim in the reason is too broad: a wrong type inside an
internally tagged enum does not get a deep `data.field`. See P10.

**Q2 (`quote.set`'s `terms`).** Agree with (a). `into_agreed_terms` builds
`AgreedTerms` with a struct literal, so the compiler already fails when a
field is added to `AgreedTerms` and not to the input. The planned unit
test is still worth it: it catches a serde attribute that differs (for
example a missing `#[serde(default)]`). Moving the terms parse to the
front changes the order of checks in `quote.set`. See P2.

**Q3 (`*.verify` takes text or an object).** Agree with (a). It is the same
choice as A2a Q9. Every caller except the Hub box already sends text. The
Hub reads `verified`, `reason`, `issuer`, `conversation_address` and
`status`, which are all top-level fields of `ListingVerdict`, so the Hub
fix changes only what it sends.

**Q4 (`roymctl` `--agreement`).** Agree with (a). The option never
filtered anything. The plan names the command wrongly. See P6.

**Q5 (closed word sets in outputs).** Agree with (a). The words do not
change, and the shape files pin them. One more closed word in an output is
not listed: `event` of `booking.history`. See P8.

**Q6 (`directory`'s call to `listing.get`).** Agree with (a). The plan's
snippet keeps today's codes. The only text change is for a reply with no
`envelope` (today `-32603 "listing.get returned no envelope"`, after
`-32603` with the `BadReply` text). That is covered by B33's "unreadable
reply → `-32603`".

### Findings

- [x] **P1 — major.** *Where:* section 5 D-A2b-9; section 3.3 (the list of
  already-recorded sites); Step 5b; section 7.3. Code:
  [fence.rs:50-75](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L50),
  [fence.rs:156-185](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L156),
  [fulfilment_ops.rs:340-348](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L340),
  [payment_ops.rs:380-404](../../../../crates/roym_transaction/src/app/payment_ops.rs#L380).
  *Problem:* the plan says the fences return the "already-recorded"
  answers as `Err(Response::ok(..))`, and proposes `FenceOutcome {
  Claimed, AlreadyRecorded(<output>) }`. The code does more than that.
  When the fence row already exists and holds a half, all three fences
  return `Err(ensure_*_recorded(..).await)`. `ensure_*_recorded` finishes
  an earlier attempt that crashed after the fence was claimed: it writes
  the row, **sends the card**, and returns the normal answer with the real
  `message_id` and send state. It can also return `already-recorded`, or a
  `-32603`. The payment acknowledgement fence also returns `Err(early)`
  from `check_acknowledgement_version`, which itself returns
  `Ok(Some(Response::ok(..)))` — a third shape. Only one site is a literal
  `Err(Response::ok(..))` (fence.rs:66). An implementer who follows the
  plan's names can map the crash-recovery path to `already-recorded`, or
  turn its `-32603` into a success. No test reaches this path: scenario
  172 hits the early return in `fulfilment_sign`, not the fence, and
  section 7.3 does not list the path as untested. *Fix:* in D-A2b-9, write
  the typed form of each function:
  `ensure_payment_request_recorded`, `ensure_payment_ack_recorded` and
  `ensure_fulfilment_recorded` return `Reply<Output>` (the output struct of
  their method). Each fence returns `Result<FenceOutcome<O>, Response>`
  with `enum FenceOutcome<O> { Claimed, Answered(O) }`, where `Answered`
  carries whatever `ensure_*_recorded` or the already-recorded check
  produced, and the error of `ensure_*_recorded` stays an `Err`.
  `check_acknowledgement_version` returns `Result<Option<HalfRecordedOutput>,
  Response>`. List all six sites by line. Add the crash-recovery path to
  section 7.3 as "not tested", and ask the code reviewer to read these
  mappings line by line. If a test is cheap, write the fence ledger row
  directly through the data layer in a parity scenario and call the method
  once.
  *Status:* Fixed (957e6b30).

- [x] **P2 — minor.** *Where:* section 4 B23; Step 5a ("Keep the order of
  the other checks"). Code:
  [quote_ops.rs:42-90](../../../../crates/roym_transaction/src/app/quote_ops.rs#L42).
  *Problem:* today `quote.set` parses `terms` late: after the certificate
  check, the `expires_in_secs` range check, `ensure_collections`, the
  request lookup (`no such request`, "a request cannot be quoted by the
  person who made it") and the quote revision checks. With
  `QuoteTermsInput` the terms are parsed first, before all of them. A
  request with bad terms and, for example, an unknown `request_record_id`
  now gets the terms error instead of `no such request`. The code is
  `-32602` both times, so only the text changes, but the plan says the
  order of every other check stays, and B23 names only the certificate
  check. *Fix:* extend B23 (or add a row): "`quote.set`: a wrong `terms`
  field is now reported before the `expires_in_secs` range check and the
  request and revision checks". Change the Step 5a rule to "keep the order
  of the checks that run after the parse". No test asserts the old order
  (checked: no parity or e2e test sends bad terms together with another
  error).
  *Status:* Fixed (957e6b30).

- [x] **P3 — minor.** *Where:* section 4 B39; D-A2b-8 (`PeerStep`). Code:
  [booking_ops.rs:442-458](../../../../crates/roym_transaction/src/app/booking_ops.rs#L442).
  *Problem:* the counterparty's `booking.history` returns the raw card
  data as `snapshot`. The plan parses it into `BookingProgressPayload` and
  writes it back. The card was signed by the other node. If that node runs
  a newer build that added a field to the payload, the card still verifies
  (the type has no `deny_unknown_fields`), but the new field is now
  dropped from the answer. Today it is returned. This is a wire change for
  a correct request, and the shape files cannot see it (both sides of the
  test run the same build). It is also not consistent with section 5,
  which keeps `CardRow.data` as `Value` because "a card's data is a
  record, not an input". *Fix:* `PeerStep { seq: Option<u32>, snapshot:
  Value, record_id: Option<String> }`, with `seq` read from the data as
  today (`null` when missing). The check allows a `Value` inside an output
  struct. B39 then becomes "no change". If the owner prefers the typed
  snapshot, B39 must say that a newer peer's extra keys are dropped.
  *Status:* Fixed (957e6b30).

- [x] **P4 — minor.** *Where:* Step 5a, Step 5b. *Problem:* Step 5a
  changes `send_card_and_file` to return `SendState` and `booking_view`'s
  `writer` to `Option<BookingWriter>`. Both have callers in the files that
  Step 5b says it converts. `send_card_and_file` is called in
  `payment_ops.rs:123` and `:618` (`record_and_send_ack` returns
  `Result<(String, String), Response>`, so its signature must change),
  `payment_ops/fence.rs:112` and `:244`, `fulfilment_ops.rs:133` and
  `:410`, and `progress.rs:54` (not named anywhere in the plan). The eight
  `booking_view` calls in `booking_ops.rs` pass `Some("self")` or
  `Some("peer")`. Step 5a does not build until all of these change. *Fix:*
  in Step 5a, say that these call sites change their types only (the
  `json!` answers stay until Step 5b), and name `progress.rs`.
  *Status:* Fixed (957e6b30).

- [x] **P5 — minor.** *Where:* section 5 D-A2b-14; Step 2. Code:
  [payment.rs:7-60](../../../../crates/roym_web/tests/dual_build_parity/payment.rs#L7),
  [bundles.rs:116-127](../../../../crates/roym_web/tests/dual_build_parity/bundles.rs#L116).
  *Problem:* the plan finds two copies of `setup_peer_request` and one of
  `setup_active_agreement`. There are more. `payment.rs` has a third
  `setup_peer_request` (`:45`) and a second `setup_active_agreement`
  (`:7`). The `payment.rs` copy is the `fulfilment.rs` one without the
  `timing` argument: it keeps the sample terms' `"after-work"`.
  `bundles.rs` scenario 168 has the same request set-up inline. *Fix:* in
  Step 2, move one `setup_active_agreement(h, conv, timing)` and one
  `setup_peer_request` into `deal_fixtures.rs`, delete the `payment.rs`
  copies, and call `setup_active_agreement(&h, &conv, "after-work")` from
  `payment.rs` (the same params as today). Use `setup_peer_request` in
  `bundles.rs:116-127` too. The `cargo dupes` and ratchet step then
  records the real gain.
  *Status:* Fixed (957e6b30).

- [x] **P6 — minor.** *Where:* "How this is shown to an end user"; Q4; B26;
  Step 6; section 8; exit criterion 8; section 3.4. *Problem:* small fact
  errors.
  (1) The command is `roymctl roym transaction booking list`, not
  `roymctl roym booking list`: `booking` is a subcommand of `transaction`
  ([transaction.rs:142-145](../../../../apps/roymctl/src/commands/roym/transaction.rs#L142),
  [tests/backup.rs:30](../../../../apps/roymctl/tests/backup.rs#L30)). The
  same holds for `booking get --agreement` in Q4.
  (2) Step 6 says `grep -rn "booking" apps/roymctl/tests` prints nothing.
  It prints `tests/backup.rs:30`, which runs
  `roym transaction booking --help`. That test still passes after Step 6,
  so no test changes, but the reason in the plan is wrong.
  (3) Step 6 says "the two lines that send it (`:173-175`)". The code is
  the `if let Some(a) = agreement` block at `booking.rs:172-174`, plus the
  `agreement,` field in the match pattern at `:164`.
  (4) Section 3.4 says the Hub's `quote.set` sends `terms` with 13 keys. It
  sends 11: no `payment_methods` and no `schedule`
  ([messages.ts:791-808](../../../../crates/roym_web/ui/src/screens/messages.ts#L791)).
  `roymctl` sends 12, plus `schedule` when given. All are fields of
  `QuoteTermsInput`, so nothing else changes.
  *Fix:* correct the five places that name the command, the reason in
  Step 6, the line numbers, and the key count.
  *Status:* Fixed (957e6b30).

- [x] **P7 — minor.** *Where:* "How this is shown to an end user".
  *Problem:* AGENTS.md asks for very simple words for a reader who is not
  technical, and "if an end user can see nothing new, say so in one plain
  sentence". The section says that first, then adds two bullets that are
  not for that reader: "Only what the Hub sends inside changes (Q3)" (a
  wire detail and a planning id) and an operator's `roymctl` option (an
  operator, not an end user). *Fix:* keep the one sentence: "An end user
  sees nothing new. Every screen works as before." Move the `roymctl`
  point to a short "An operator ..." bullet with the right command (P6)
  and no question id, or to the developer bullets. Drop "(Q3)" and the
  Hub wire detail.
  *Status:* Fixed (957e6b30).

- [x] **P8 — minor.** *Where:* D-A2b-13; Q5. Code:
  [booking_ops.rs:199-206](../../../../crates/roym_transaction/src/app/booking_ops.rs#L199),
  [ledger.rs:40](../../../../crates/roym_transaction/src/app/ledger.rs#L40).
  *Problem:* the author steps of `booking.history` carry `event`, a closed
  word set (`start`, `cancel`, `half`, `tick`). D-A2b-13 does not list it,
  so the shape file records `"string"` and a renamed word would pass.
  *Fix:* add `event` to the closed-word keys of D-A2b-13 and of section
  7.2. It stays a `String` in the output, because it is copied from the
  stored `StepRow` (stored rows keep their strings, D-A2a-10). Checked: no
  A2a shape file has an `event` key.
  *Status:* Fixed (957e6b30).

- [x] **P9 — minor.** *Where:* D-A2b-7 ("each verify method gets its own
  small typed handler"). *Problem:* seven handlers would each repeat the
  same three steps: take `input.envelope`, read the clock, call one verify
  function. That is the "same shape, different names" duplication that
  `cargo dupes` reports. *Fix:* add one helper beside `VerifyInput` in
  `roym_core::method`, for example
  `pub fn verify_with<V>(input: VerifyInput, verify: fn(&str, u64) -> V) ->
  Reply<V>`, and make each verify handler one line that calls it with
  `transaction::verify_request`, `listing::verify_envelope`, and so on.
  `payment.verify` keeps its own handler, because it picks the verify
  function by record type.
  *Status:* Fixed (957e6b30).

- [x] **P10 — minor.** *Where:* Q1 (the last sentence of the
  recommendation); section 9 (the developer-guide sentence). Code:
  [area.rs:21-34](../../../../crates/roym_core/src/area.rs#L21).
  *Problem:* the plan says a wrong type inside a record type "is still
  refused with `data.field` in every option (for example
  `payment.amount_minor`)". That is true for plain structs. `Area` is an
  internally tagged enum (`#[serde(tag = "kind")]`). serde reads such an
  enum into a buffer first and parses the variant from that buffer, and
  `serde_path_to_error` cannot follow it. A wrong type inside `area` gets
  `-32602` with `data.field = "area"`, not `area.radius_m`. *Fix:* in Q1
  and in the developer-guide sentence, write: "a wrong type inside a
  nested block is refused with `-32602`; `data.field` names the deepest
  field it can, which for a tagged block such as `area` is the block
  itself". Optionally pin this in one Step 7 test (`request.set` with
  `area.radius_m: "x"`), so a later change of the path is seen.
  *Status:* Fixed (957e6b30).

## Plan review round 1 — fixes confirmed — Claude Opus 5.5 (Claude Code), 2026-10-30

Checked `957e6b30` (the plan) and `1e3e0fe7` (this file and status.md)
against P1 to P10, and checked every new fact the fixes added against the
code at `1e3e0fe7`. Every item is fixed as written. No new item is open.

- **P1.** Section 3.3 now lists the six fence sites F1 to F6 with the right
  lines (`fence.rs:54`, `:66`, `:161`, `:168-181`, `fulfilment_ops.rs:346`,
  `payment_ops.rs:380-404`) and explains the crash-recovery path. D-A2b-9
  gives every signature and a mapping per site. The F4 note is right: the
  fence reads the check with `if let Ok(Some(..))`, so an `Err` falls
  through to `payment-acknowledgement-in-flight`. Step 2 adds one
  crash-recovery shape case per method, and section 7.3 lists what stays
  untested (F2, F4, the in-flight errors, the `-32603` branch). I checked
  that the cases can work: `PaymentsRow` and `FulfilmentsRow` have
  `#[serde(default)]` on every half field, so the seeded rows parse; the
  handlers' own already-recorded checks (`payment_ops.rs:76`,
  `fulfilment_ops.rs:72`, `check_acknowledgement_version` with an empty
  list) let the second call reach the fence; and the fence keys do not
  depend on the new half. `seed_row` is at `admission.rs:242` and today
  always opens the `conversation` database, as the plan says.
  One note for the implementer, not a finding: run the three cases in the
  order request, acknowledgement, fulfilment, or on separate deals.
  `payment.request` refuses with `booking-payment-acknowledged` once the
  payment track is acknowledged (`payment_ops.rs:347`). A wrong order
  fails loudly in Step 2, so it cannot hide a bug.
- **P2.** B23 and the Step 5a rule now name the earlier `terms` parse.
- **P3.** `PeerStep { seq: Option<u32>, snapshot: Value, record_id }`; B39
  is now "no change".
- **P4.** Step 5a names every `send_card_and_file` caller (including
  `progress.rs:54`), `record_and_send_ack`'s new return type, and the eight
  `booking_view` calls. The line numbers (`:282`, `:290`, `:300`, `:308`,
  `:342`, `:386`, `:510`, `:544`) are right.
- **P5.** D-A2b-14 keeps one copy of each helper, with the `payment.rs`
  and `bundles.rs` copies. Passing `"after-work"` from `payment.rs` gives
  the same params as today (the sample terms use `"after-work"`).
- **P6.** The command is `roymctl roym transaction booking list` in every
  place, the `roymctl` test reason is right, the lines are right, and the
  Hub key count is 11.
- **P7.** The end-user section is one plain sentence, with the operator
  bullet separate and no question ids.
- **P8.** `event` is in D-A2b-13 and section 7.2.
- **P9.** `verify_with` is in D-A2b-7. All six verifiers it names are
  `fn(&str, u64) -> V` (`transaction.rs:614`, `:658`, `:710`,
  `listing.rs:505`, `booking.rs:330`, `fulfilment.rs:78`), and
  `roym_core::clock` exists.
- **P10.** Q1, the developer-guide sentence and a new Step 7 test say the
  path stops at `area`. I proved it with a small program on the
  workspace's `serde_path_to_error` version: the input of the Step 7 test
  gives path `area` and text ``invalid type: string "x", expected u64``,
  so `parse_value` sets `data.field = "area"`.

The owner answers (all (a)) are recorded under each question, D-A2b-16 and
D-A2b-17 are added for Q2 and Q4, and section 9 now asks for D-A2b-1 to
D-A2b-17 in the change doc. The plan is ready for implementation.

## Code review round 1 — Gemini (Antigravity), 2026-10-31

*Note added in round 2: this round was written by Gemini in Antigravity.
Its title first named Claude Sonnet 5.5 by mistake.*

Checked against commit `7f9f088c` on `feat/m08a-a2b-typed-methods` (PR
[#335](https://github.com/syneroym/syneroym/pull/335)), against the plan and
`main`. Line numbers below are at `7f9f088c`. The implementer was Gemini 3.8
Flash.

What was checked and found correct:

- **Method tables (OF-14, OF-15).** `catalog` dispatches 17 rows (15 catalog
  methods plus the two certificate verbs) in `crates/roym_catalog/src/api.rs`.
  `transaction` dispatches 41 rows (7 pings, 5 request, 6 quote, 4 agreement,
  6 booking, 4 payment, 3 fulfilment, and 6 sync/thread/backup/certificate) in
  `crates/roym_transaction/src/api.rs`. Both tables match section 5 of the
  implementation plan row by row, with typed input and output structs. Every
  input struct derives `#[serde(deny_unknown_fields)]` except stored record
  types inside inputs (Q1 (a)).
- **Method contract tests.** In `crates/roym_web/tests/method_contract/contract.rs`,
  `every_catalog_method_has_typed_input_and_output` and
  `every_transaction_method_has_typed_input_and_output` assert 17 and 41 rows
  and invoke `assert_service_table`. The paths are built from
  `env!("CARGO_MANIFEST_DIR")` (`../roym_catalog/src` and
  `../roym_transaction/src`). `assert_service_table` scans all `.rs` files
  under each directory (excluding `bindings.rs`), confirming zero
  hand-written match arms and no `req.method` use outside
  `Response::method_not_found`.
- **Fences and crash recovery (D-A2b-9).** All six fence sites (F1 to F6) in
  `payment_ops/fence.rs`, `payment_ops.rs`, and `fulfilment_ops.rs` follow the
  exact typed outcome signatures. `claim_payment_request_fence`,
  `claim_payment_ack_fence`, and `claim_fulfilment_fence` return
  `FenceOutcome::Claimed` or `FenceOutcome::Answered(out)`. In
  `ensure_*_recorded`, the crash recovery path writes the missing shared row,
  calls `send_card_and_file` to send the deal card, and returns the real
  `message_id` and send state. Existing rows return `AlreadyRecorded` with
  `message_id: None`. Database failures return `-32603`
  `Response::internal_error`.
- **Wire changes (B23 to B39).** Check ordering in `request.set`, `quote.set`,
  `agreement.accept`, and `quote.decline` runs input parsing before
  certificate/owner checks (B23). In `quote.set`, `terms` is parsed upfront as
  `QuoteTermsInput`, catching invalid fields before range or revision checks.
- **Specific requirements (Q2, Q3, Q4, Q6).**
  - Q2: `QuoteTermsInput` has all 13 fields of `AgreedTerms` minus
    `quote_expires_at_secs`. `into_agreed_terms` constructs `AgreedTerms`.
    Tested in `crates/roym_transaction/src/app/quote_ops/tests.rs` with
    `quote_terms_input_matches_agreed_terms_fields`.
  - Q3: Hub `screens/listings.ts` exports `verifyParams(pasted)` returning
    `{ envelope: pasted.trim() }`. The click handler checks valid JSON display
    and sends string text. Tested in `listings.test.ts`.
  - Q4: `roymctl roym transaction booking list` dropped `--agreement` from
    `BookingCommands::List` and execution. No remaining CLI references or tests
    broken.
  - Q6: `publish_to_source` in `crates/roym_directory/src/app/client_sources.rs`
    calls `listing.get` via
    `call_sibling::<_, ListingKeyInput, Option<ListingGetOutput>>` and maps
    errors per B33. `sibling_call` and `catalog_call` were deleted from
    `roym_transaction`.
- **Parity harness and tests.** 24 modules, 222 scenarios (`fn scenario_`) in
  `crates/roym_web/tests/dual_build_parity/`. Only scenario 40's assertion
  changed to check `data.field == "payment.amount_minor"`. All 10 negative
  validation cases in `contract.rs` test catalog and transaction error paths
  across WASM and native.
- **AGENTS.md standards.** All changed production and test files are <= 725
  lines (well below the 800 line cap). No planning IDs exist in code comments
  or test names (`check-planning-refs.py` clean). Module layout convention
  verified across all 1010 files. All xtask gates pass. Backlog and living
  docs updated as specified.

### Findings

- [x] **R1 — minor.** *Where:* change doc
  [change.md:259-260](../../changes/roym-open-foundation/change.md#L259).
  *Problem:* the change doc decisions D-A2b-4 and D-A2b-5 do not match the
  implementation plan's decisions. Plan D-A2b-4 is "Existing field attributes
  stay" (preserving `#[serde(default)]` and explicit null rules) and D-A2b-5 is
  "Closed sets" (Q5 (a), inputs `ListingStatus`/`BookingState` and outputs
  `SendState`/`BookingWriter`). In the change doc, D-A2b-4 was written as "Page
  fields repeated without flatten" (which is from D-A2a-4) and D-A2b-5 was
  written as "Nullable and tri-state outputs" (which is from D-A2a-15 /
  D-A2b-8). Plan D-A2b-4 and D-A2b-5 were omitted from the decision list.
  *Failure case:* a developer comparing decisions between the plan and change
  doc finds mismatched numbers and no record of the Q5 closed sets decision or
  attribute retention. *Fix:* update `change.md` decisions D-A2b-4 and D-A2b-5
  to record existing field attribute preservation and closed word sets
  (`SendState`, `BookingWriter`, `ListingStatus`, `BookingState`), matching
  section 5 of the plan. *Status:* Fixed (f70e2b13).

- [x] **R2 — minor.** *Where:* status doc
  [status.md:14](status.md#L14). *Problem:* the evidence cell reads "77 method
  shapes (14 catalog + 63 transaction)". In reality, slice A2b added 94 new
  golden shape files in `crates/roym_web/tests/method_shapes/` (25 for catalog
  covering all 17 methods, and 69 for transaction covering all 41 methods).
  Together with A2a's 63 shapes, the repository total is 157 shapes. The
  recorded "77 (14 + 63)" confused A2a's 63 shapes with transaction shapes and
  reported 14 instead of 25 catalog shapes. *Failure case:* an auditor or
  reviewer checking file counts finds a discrepancy between the recorded 77
  shapes and the 94 new / 157 total shape files on disk. *Fix:* update the
  evidence cell in `status.md` to state "94 method shapes (25 catalog + 69
  transaction; 157 total)". *Status:* Fixed (f70e2b13).

- [x] **R3 — minor.** *Where:*
  [method.rs:40](../../../../crates/roym_core/src/method.rs#L40).
  *Problem:* line 40 calls `crate::clock::now_secs()`, leaving an inline
  fully-qualified path with multiple `::` instead of importing `clock` at the
  module header. This violates the AGENTS.md import rule ("proactively remove
  inline fully-qualified paths (lines with multiple ::)"). *Failure case:*
  drifts from workspace import consistency conventions. *Fix:* add
  `use crate::clock;` to the module imports of `crates/roym_core/src/method.rs`
  and simplify line 40 to `clock::now_secs()`. *Status:* Fixed (a1b0f219).

- [x] **R4 — minor.** *Where:*
  [fence.rs:53-77](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L53)
  and [:160-192](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L160).
  *Problem:* the `Ok(Some(_))` collision handling match arm bodies in
  `claim_payment_request_fence` (25 lines) and `claim_payment_ack_fence` (32
  lines) exceed the ~10-line guideline for match arms in AGENTS.md ("Match arms
  must not exceed ~10 lines. Extract complex match arm bodies into named helper
  functions..."). *Failure case:* reduced readability and maintainability of
  fence claim logic compared to `fulfilment_ops.rs:329-340`, which kept its
  collision arm concise (11 lines). *Fix:* extract the collision recovery
  bodies into named private helpers (e.g.,
  `handle_payment_request_fence_collision` and
  `handle_payment_ack_fence_collision`). *Status:* Fixed (1f8d47b4).

Count: 0 blockers, 0 major, 4 minor.

## Code review round 2 — Claude Opus 5.5 (Claude Code desktop), 2026-10-31

Checked `git diff origin/main...HEAD` at `7f9f088c` on
`feat/m08a-a2b-typed-methods` (PR
[#335](https://github.com/syneroym/syneroym/pull/335), still a draft),
against the plan, the change doc and `AGENTS.md`. `025a370b` changes only
this file and `status.md`, so the line numbers below are the same at both
commits. The implementer was Gemini 3.8 Flash. This round re-checks round
1 (R1 to R4) and adds R5 onward. CI passed on `7f9f088c`. Only small
targeted tests were run (listed below).

**Summary of this round:** 0 blocker, 1 major (R5), 7 minor (R6 to R12).
Round 1's R1 to R4 are all kept; R1 and R2 are widened. Open in total:
0 blocker, 1 major, 11 minor.

### What was checked and found correct

- **Method tables (point 1).** `catalog` has 17 rows
  ([api.rs:25-43](../../../../crates/roym_catalog/src/api.rs#L25)) and
  `transaction` 41 rows
  ([api.rs:26-75](../../../../crates/roym_transaction/src/api.rs#L26)).
  Each row has the input and output of the section 5 tables. Every input
  struct has `deny_unknown_fields` (also `SlotInput`); the record types
  inside inputs do not (Q1). Both `invoke` functions are now
  `require_internal`, `api::dispatch`, `method_not_found(&req.method)`.
  The greps of Step 4 and Step 5b (`params.get`, `req.params`,
  `Response::ok`, `sibling_call`, `catalog_call`, and also
  `handle_certificate_verb`) print nothing.
- **The check really runs (point 1).** `contract.rs:42-64` builds both
  source paths from `CARGO_MANIFEST_DIR`. `scan_source_dir` reports a
  folder it cannot read, so a wrong path fails instead of passing empty
  (the A2a R1 risk). The arm detector is unit-tested
  ([method/tests.rs:403](../../../../crates/roym_core/src/method/tests.rs#L403),
  `should_panic(expected = "hand-written match arm")`), and a line
  `"listing.get" =>` matches `is_match_arm`. Run here:
  `cargo nextest run -p syneroym-roym-web --test method_contract -E
  'test(every_catalog_method) | test(every_transaction_method)'`: 2 run,
  2 passed, none skipped. The hand-made arm proof of Step 7 was not
  repeated, because this review does not change code.
- **Fences F1 to F6 and the three `ensure_*_recorded` (point 2).** Read
  line by line against the D-A2b-9 table:
  - F1 ([fence.rs:57-66](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L57)),
    F3 ([:164-168](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L164))
    and F5 ([fulfilment_ops.rs:335-338](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L335))
    are `ensure_*_recorded(..).await.map(FenceOutcome::Answered)`. An `Ok`
    becomes `Answered`; a `-32603` stays `Err`.
  - F2 (`fence.rs:67-75`) and F6 (`payment_ops.rs:367-376`) write
    `message_id: None` and `SendState::AlreadyRecorded`, as before.
  - F4 (`fence.rs:174-188`) keeps `if let Ok(Some(early))`: the booking
    transition runs first, the `Err` of the check is still ignored, and
    the fence still answers `payment-acknowledgement-in-flight`.
  - Inside each `ensure_*_recorded` the branches and their order are the
    same as on `main`: read the row (`-32603` on error), return
    already-recorded only when the row already holds this half, write the
    row (`-32603` on error), transition, send the card, and return the
    fence half's `record_id` with `Some(message_id)` and the real send
    state. Nothing turns an error into success, and nothing turns a
    recovery into already-recorded.
  - The three crash-recovery shape cases
    ([shapes_transaction.rs:372-440](../../../../crates/roym_web/tests/method_contract/shapes_transaction.rs#L372))
    assert the first call's `record_id` and `state != "already-recorded"`.
    Their files (`state: "pending"`) were recorded in `7d1463b9`, before
    any handler changed, and no later commit touched them.
- **B23 (point 3).** `typed` parses before the handler runs, so in
  `request.set`, `quote.set`, `agreement.accept` and `quote.decline` the
  certificate or owner check now runs after the parse. In `quote.set` the
  terms are part of the parse, before the `expires_in_secs` range, the
  request lookup and the revision checks. This is what the plan meant. The
  checks themselves and their texts did not change.
- **Other wire changes (point 3).** B24, B26, B28 to B32, B34 to B36 are
  in the code as section 4 says. B33: `validate_and_apply_slot`
  ([quote_ops.rs:321-334](../../../../crates/roym_transaction/src/app/quote_ops.rs#L321)),
  `claim_seat`
  ([ledger.rs:142-155](../../../../crates/roym_transaction/src/app/ledger.rs#L142))
  and `publish_to_source`
  ([client_sources.rs:221-237](../../../../crates/roym_directory/src/app/client_sources.rs#L221))
  have the plan's `match` arms. The one change that is in no list is R5.
- **Q2.** `QuoteTermsInput` has the 13 fields of `AgreedTerms` without
  `quote_expires_at_secs`, with the same two `default` and
  `skip_serializing_if` attributes. `into_agreed_terms` is a struct
  literal, so a field added on one side only does not compile.
  `quote_terms_input_matches_agreed_terms_fields` fills every optional
  field and compares the JSON. Run here: passed.
- **Q3.** `verifyParams` returns `{ envelope: pasted.trim() }`; the click
  handler keeps `JSON.parse` only for the "not valid JSON" text and sends
  `verifyParams(area.value)`. `listings.test.ts` checks the type and the
  trim.
- **Q4.** `BookingCommands::List` has no `agreement`. No doc, help text or
  `roymctl` test names `booking list --agreement`
  (`apps/roymctl/tests/backup.rs` runs only `--help`).
- **Q6.** `publish_to_source` uses
  `call_sibling::<_, ListingKeyInput, Option<ListingGetOutput>>`. `Ok(None)`
  and `Refused` give `-32602 "no such listing on this installation"`;
  `Transport` and `BadReply` give `-32603`.
- **Tests of section 7 (point 7).** The three `roym_core` unit tests, the
  quote-terms unit test, the two table tests, the extended dispatch and
  off-node loops (now one loop over four tables), the ten error cases
  (each through `post_rpc`, which asserts WASM == native), the Hub test,
  and all required shape cases exist: 25 catalog files and 69 transaction
  files, all added in `7d1463b9`; none of the 63 A2a files changed. The
  four unit tests named above were run here and pass. The parity harness
  still has 222 `fn scenario_`; only scenario 40 changed an assertion (the
  other removed `assert_eq!` lines are in helpers that moved to
  `deal_fixtures.rs`).
- **Size, layout, imports, ids (point 8).** No changed file is over 800
  lines (largest: `method/tests.rs`, 725). Module layout is
  `quote_ops.rs` + `quote_ops/tests.rs`. No planning id is in code or test
  names. Only one inline multi-`::` path was added (R3). No dependency
  changed. The backlog rows of section 8 are in place (§10 narrowed, the
  new §10 and §12 rows, the sibling-helper row in "Recently resolved").
  The developer guide, architecture doc and traceability matrix name the
  four typed services and do not over-claim `directory` or `web`.
- **The verify run.** `xtask/src/verify.rs` has 15 gates. "15 gates" in
  `status.md` is right. A2a's row says 17, but `verify.rs` has not
  changed since #327, before A2a, so A2a's 17 was the wrong number, not
  A2b's 15. `target/gates/` holds a full run from 2026-10-30
  (`nextest`: 4186 passed, 1 skipped).

### Round 1 items, re-checked

- **R1 — kept, widened.** D-A2b-4 and D-A2b-5 hold A2a text, as R1 says.
  Also: D-A2b-3 in the change doc keeps only the second half of the
  plan's rule. The plan's D-A2b-3 is "every input refuses unknown fields
  at its top level (also `SlotInput`); record types inside are not
  changed". The change doc says only the second part. Fix D-A2b-3 too.
  D-A2b-6, -7 and -8 are shorter than the plan but keep the meaning.
- **R2 — kept, widened.** The right numbers are 94 new shape files (25
  catalog, 69 transaction), 157 in total. "14 catalog" has no source in
  the code: the 17 catalog rows have 25 files. The same wrong "77 (14 +
  63)" is in the PR text; fix it there too. Also: the parity harness now
  has 25 module files, because `deal_fixtures.rs` was added (the plan
  asked for it and also said "24 modules", which cannot both be true).
  Write "24 scenario modules plus `deal_fixtures`, 222 scenarios" in
  `status.md`, `task.md` and the PR. "15 gates" is correct (see above).
- **R3 — kept.** `method.rs:40` is the only inline multi-`::` path the
  diff adds.
- **R4 — kept, with a note.** The two long `Ok(Some(_))` arms were the
  same size on `main`; A2b changed one or two lines in each. The rule
  still applies to code this PR touches, and the fix is small. If the fix
  session leaves it, mark it `Deferred` with a backlog row, not `Fixed`.

### Findings

- [x] **R5 — major.** *Where:*
  [agreement_ops.rs:411-486](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L411)
  (`maybe_countersign`), from commit `e0c83c94`. *Problem:* the body was
  rewritten to reuse `sign_agreement_receipt` and `send_card_and_file`.
  The plan says this send does not change (section 3.1, and section 11:
  "No change to what a correct request does"). The PR text and the change
  doc do not name it. The rewrite changes behaviour in four places:
  1. A failed card send now returns `Err` (`:478-483`). On `main` it
     returned `Ok(true)` with no card filed.
  2. A payload that fails `validate()` now returns `Err`
     (`sign_agreement_receipt` gives `-32602`, mapped at `:459`). On
     `main` it returned `Ok(false)`. The new issuer check in
     `sign_agreement_receipt` is a third new `Err`.
  3. `person_principal` now runs before `decide_booking` (`:436` vs
     `:443`), and a new `ensure_collections` call runs (`:441`).
  4. The error text is the `Debug` print of a `Response` (`:459`).
  `maybe_countersign(..).await?` is called from `file_incoming_card`
  ([sync.rs:577](../../../../crates/roym_transaction/src/app/sync.rs#L577)),
  and an `Err` there ends the whole `transaction.sync` with `-32603`.
  *Failure case:* the quote author's node syncs a conversation that holds
  the counterparty's agreement receipt, and `conversation.send` refuses
  the countersigned card (or `file_own_card` fails). The agreement row is
  already written with the author's half (`:465`). `transaction.sync`
  answers `-32603`, the incoming card row is not written, `last_seq` is
  not saved, and the later cards of that batch are not filed in this
  call. On `main` the same call succeeded with `countersigned: 1`. The
  next sync finds the half and returns `Ok(false)`, so the card is still
  never sent, as on `main`. If `validate()` ever fails for a row, every
  sync of that conversation fails the same way, because the half is never
  written. *Fix:* restore the body of `maybe_countersign` from `main`
  (`agreement_ops.rs:408` there). Keep only the type changes A2b needs.
  If the reuse of `sign_agreement_receipt` and `send_card_and_file` is
  wanted, do it in a later change with a test, and keep the old outcomes:
  `Ok(false)` for a payload that does not validate, `Ok(true)` when the
  send fails, `decide_booking` before `person_principal`, no new
  `ensure_collections`. Either way, the fact that a countersigned card
  whose send fails is never sent again is older than A2b; add a backlog
  row for it under §12. *Status:* Fixed (16465e9b).

- [x] **R6 — minor.** *Where:*
  [contract.rs:267-376](../../../../crates/roym_web/tests/method_contract/contract.rs#L267).
  *Problem:* three error cases send more than one error and pass only
  because `serde_json::Map` is sorted, so the key under test is read
  first:
  - cases 2 and 3 (`:276-310`) send `slot_id`, which `SlotInput` does not
    have;
  - case 7 (`:328-346`) sends `terms` with 5 of the 13 fields; the plan
    asked for "a valid request so the parse is the only error";
  - case 10 (`:358-375`) sends `summary` (not a field) and leaves out
    `description` and `data_use_notice`. It also does not pin the message
    text, which the plan asked for.
  *Failure case:* someone renames `area` to `zone`, or `serde_json` gets
  `preserve_order` through a dependency feature. Case 10 then reports
  `summary` or a missing field, and the test fails for a reason that has
  nothing to do with tagged blocks. In the other direction, a real
  regression in the field path can hide behind another error. *Fix:* in
  each case send an otherwise valid request: remove `slot_id` from the
  slots, send full terms (reuse `valid_quote_params` from the parity
  fixtures and add one key), send `description` and `data_use_notice`
  instead of `summary`. Pin the text of case 10 with one
  `assert_eq!` on the message. *Status:* Fixed (64a389bd).

- [x] **R7 — minor.** *Where:* the ten comments `// 1. listing.get ..` to
  `// 10. request.set ..` in
  [contract.rs:270-358](../../../../crates/roym_web/tests/method_contract/contract.rs#L270).
  *Problem:* the comments number the list of Step 7 of the plan. The
  comment rule forbids test numbers that point at a planning doc (A2a had
  the same finding, R10). *Failure case:* a case is added or removed and
  the numbers no longer match anything. *Fix:* drop the numbers, or split
  the cases into named test functions (for example
  `listing_get_with_a_number_names_listing_id`). *Status:* Fixed (64a389bd).

- [x] **R8 — minor.** *Where:* six "why" comments were deleted and not
  moved, beside code that is still there:
  [listing_ops.rs:501](../../../../crates/roym_catalog/src/app/listing_ops.rs#L501)
  (the host filters by status),
  [:513](../../../../crates/roym_catalog/src/app/listing_ops.rs#L513)
  (a row whose envelope does not parse still lists, with an empty title),
  [:538](../../../../crates/roym_catalog/src/app/listing_ops.rs#L538)
  (history rows are keyed by `record_id`; `supersedes` gives the exact
  order),
  [limits.rs:43](../../../../crates/roym_catalog/src/app/limits.rs#L43)
  (filtered at the host, and why it is still not an indexed scan),
  [backup.rs:126](../../../../crates/roym_catalog/src/app/backup.rs#L126)
  (indexes are added lazily), and
  [request_ops.rs:214](../../../../crates/roym_transaction/src/app/request_ops.rs#L214)
  (the record is stored before the card is sent, and why that order).
  *Problem:* these explain invariants and trade-offs, which is what the
  comment rule wants kept. A typing slice has no reason to remove them.
  *Failure case:* a later change moves `put_bytes` after the send in
  `request.set`, because nothing says the order matters. *Fix:* restore
  the six comments from `main`. *Status:* Fixed (d58e90f5).

- [x] **R9 — minor.** *Where:*
  [shapes_transaction.rs:138-171](../../../../crates/roym_web/tests/method_contract/shapes_transaction.rs#L138)
  and [:237-270](../../../../crates/roym_web/tests/method_contract/shapes_transaction.rs#L237);
  [:183-205](../../../../crates/roym_web/tests/method_contract/shapes_transaction.rs#L183)
  and [:293-318](../../../../crates/roym_web/tests/method_contract/shapes_transaction.rs#L293);
  [agreement_ops.rs:317-329 and :351-363](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L317).
  *Problem:* the "slot quote by this node, accepted by the peer" set-up
  (34 lines) is written twice in `shapes_transaction.rs`. The
  "peer quote on this node's request, synced" set-up is written twice
  there too, and is the same as `deal_fixtures::setup_accepted_counterparty_quote`
  except for the RPC helper and the ids. D-A2b-14 moved such set-up into
  `deal_fixtures.rs` so that it has one copy. In production code,
  `AgreementRow` → `AgreementView` is written twice (`agreement.get` and
  `agreement.list`; `cargo dupes` reports it). *Failure case:* a change to
  how a slot quote is set up is made in one copy; the other shape case
  then records a different state. *Fix:* add
  `setup_author_slot_quote_accepted_by_peer(h, conv, listing_name) ->
  String` to `deal_fixtures.rs` and call it from both functions; give
  `setup_accepted_counterparty_quote` the quote sequence and message id
  as arguments, or add a variant that does not call `agreement.accept`,
  and reuse it. Add `impl From<AgreementRow> for AgreementView` and use
  it in both handlers. *Status:* Fixed (1f8d47b4, 64a389bd).

- [x] **R10 — minor.** *Where:*
  [agreement_ops.rs:104-109](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L104).
  *Problem:* `booking_opt` is `Some(booking_view)` for the quote author
  and `Some(None)` otherwise, but `booking_view` is already `None` for
  everyone else. Both branches give `Some(booking_view)`. *Failure case:*
  a reader looks for the case where the two differ, and there is none.
  *Fix:* delete `booking_opt` and write `booking: Some(booking_view)`
  in the output. *Status:* Fixed (1f8d47b4).

- [x] **R11 — minor.** *Where:*
  [transaction api.rs:53-58, :65](../../../../crates/roym_transaction/src/api.rs#L53),
  [payment_ops.rs:27](../../../../crates/roym_transaction/src/app/payment_ops.rs#L27).
  *Problem:* the handlers were renamed to the verb (`request_ops::get`,
  `quote_ops::list`, `booking_ops::history`, `payment_ops::verify`), but
  four booking handlers keep the old prefix (`booking_ops::booking_get`,
  `booking_list`, `booking_start`, `booking_cancel`). Also,
  `HalfRecordedOutput` lives in `app.rs` but is re-exported from
  `payment_ops` with `pub use`, and the `fulfilment.sign` row names it as
  `payment_ops::HalfRecordedOutput`. *Failure case:* a reader of the
  table thinks `fulfilment.sign` returns a payment type, and looks for
  `booking_ops::get` and does not find it. *Fix:* rename the four booking
  handlers to `get`, `list`, `start`, `cancel`; delete the `pub use` and
  import `HalfRecordedOutput` from `crate::app` in `api.rs`. *Status:*
  Fixed (1f8d47b4).

- [x] **R12 — minor.** *Where:* change doc
  [change.md:285-310](../../changes/roym-open-foundation/change.md#L285).
  *Problem:* the Deviations are split into "Different from the plan" and
  "Planned wire changes", as asked, and the
  `roym_directory_e2e.rs` edit is correctly under the first. Two entries
  are not precise:
  - B33 (`:307`) says the sibling calls "preserve -32602 / -32603
    mapping". They do not all: a reply that does not parse (also a
    malformed slot row, B32) was `-32603` in `validate_and_apply_slot` and
    is now `no-such-slot`. That is the one change B33 makes.
  - "Different from the plan" does not list R5. If R5 is not reverted,
    it must be listed there.
  *Failure case:* a reader of the change doc thinks no sibling error
  changed, and thinks `maybe_countersign` is as before. *Fix:* rewrite
  the B33 entry: "same as before, except that an unreadable or malformed
  `availability.get` reply is now `no-such-slot` (`quote.set`) or
  `slot-unavailable` (booking), not `-32603`". Add R5 to "Different from
  the plan" only if the rewrite is kept. *Status:* Fixed (f70e2b13).

Count of this round: 0 blocker, 1 major, 7 minor. Open in this file: 0
blocker, 1 major (R5), 11 minor (R1 to R4, R6 to R12).


## Code review rounds 1 and 2 — fixes confirmed — Claude Opus 5.5 (Claude Code desktop), 2026-10-31

Checked the fix commits `16465e9b`, `a1b0f219`, `1f8d47b4`, `d58e90f5`,
`64a389bd`, `f70e2b13` and `85ceb769` against R1 to R12. Every item in the
repository is fixed as written. No new item is open.

- **R5.** The body of `maybe_countersign` is the same as on `main`. A diff
  of the two functions shows only the typed `method::call_sibling` path
  and `&row` for `put_row`. So the old outcomes are back: `Ok(false)` for
  a payload that does not validate or a booking conflict, `Ok(true)` when
  the send fails, `decide_booking` before `person_principal`, and no
  `ensure_collections`. The older "a failed countersign send is never sent
  again" gap has a row in `deferred-backlog.md` §12.
- **R4.** The two collision arms are now `handle_payment_request_fence_collision`
  and `handle_payment_ack_fence_collision`. The bodies moved without a
  change in behaviour.
- **R3, R10, R11.** `clock::now_secs()` is imported. `booking_opt` is gone
  and the output is `Some(booking_view)`, which gives the same JSON. The
  four booking handlers are `get`, `list`, `start` and `cancel`, and the
  `pub use` of `HalfRecordedOutput` is gone.
- **R8.** The plain comments in `backup.rs`, `limits.rs`, `listing_ops.rs`
  and `request_ops.rs` are now the same as on `main`.
- **R9.** Both shape tests call `setup_author_slot_quote_accepted_by_peer`
  and `setup_synced_counterparty_quote` from `deal_fixtures.rs`, and
  `setup_accepted_counterparty_quote` reuses the second one.
  `impl From<AgreementRow> for AgreementView` is used by `agreement.get`
  and `agreement.list`. No file under `tests/method_shapes/` changed.
- **R6, R7.** Each error case now sends one error only: no `slot_id`, full
  terms from `valid_quote_params`, and `description` plus
  `data_use_notice` in `request.set`. The case for `request.set` pins
  the full message text. The case numbers are gone.
- **R1, R12.** D-A2b-3, D-A2b-4 and D-A2b-5 now match the plan, and the
  B33 entry names the one changed error.
- **R2.** `status.md` and `task.md` have the right counts.

Tests run here, after `mise run build:roym` (the WASM files are newer
than the last source change): `method_contract` 19 passed, 0 skipped
(the 4 table tests, the error cases and all shape tests);
`syneroym-roym-transaction` 1 passed; 11 parity scenarios on agreements,
countersign and booking (129, 130, 151 to 157, 164, 166) passed.

Two things outside this file:

- The PR text was updated to reflect the new counts (94 new shape files /
  157 total, 24 scenario modules plus `deal_fixtures`) and the B33 error text.
- The evidence cell in `status.md` was updated with the passing CI on
  `85ceb769` (2026-10-31).

The code review of A2b is closed.

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

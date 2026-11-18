# M08C slice C2 — review

*Reader: the developer who fixes the C2 plan or code, and the project owner.
This file holds every review finding for slice C2 (parts C2a and C2b).
Plan review findings are P1, P2, …; code review findings are R1, R2, ….
Ids are never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code), 2026-11-18

Checked [slice-c2-implementation-plan.md](slice-c2-implementation-plan.md)
on branch `feat/m08c-c2-disclosure` at `8cb98f10` (the plan commit, on top
of `main` at `ba5a5597`). `git pull` could not run (the sandbox blocks
github.com), so the branch was not refreshed. `main` had no newer commit
when the plan was written, so this is not expected to change a finding.
The plan was written by Claude Opus 5.5; this round runs on Claude Sonnet
5.5, so the reviewer is a different model from the author.

Read: AGENTS.md, session-strategy.md, the plan, task.md, status.md, both
change docs named in the prompt (roym-agreements: "Terms", the field
tables, "Selective disclosure", "Methods", `## Deviations`;
roym-value-exchange: VX-11 and "Disclosure of deal evidence"), and the C1
review file (kinds of findings before). task.md's C2 section names the same
two change docs as the prompt, so no change doc is missing from the list.

**Summary: 12 findings: 0 blocker, 3 major, 9 minor.**

### What was checked and found correct

- **The RFC 9901 vector.** I computed `SHA-256` over the ASCII bytes of
  `WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0`
  and base64url-encoded it: the result is
  `X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0`, as the plan and AG-19 say.
  The string decodes to `["_26bc4LT-ac6q2KI6cBW5es", "family_name",
  "Möbius"]` (with spaces), as the plan says.
- **File and line links.** About 70 checked, nearly all right:
  `quote_ops.rs:101`, `:141`, `:149`, `:169`, `:372`, `:393`, `:69` to
  `:94`; `agreement_ops.rs:296`, `:323`, `:423`; `decide.rs:683`;
  `payment_ops.rs:124`, `:177`, `:263`, `:553`, `:586`, `:644`;
  `fulfilment_ops.rs:130`, `:219`, `:351`, `:523`; `app.rs:52`, `:157`,
  `:216`, `:272`, `:327`, `:472`, `:537`, `:546`; `sync.rs:286`, `:434`,
  `:509`, `:526`, `:540`; `backup.rs:480`; `card.rs:6`, `:37`, `:63`
  (`MAX_CARD_BODY_BYTES` is `160 * 1024` = 163,840); `transaction.rs:163`,
  `:175`, `:185`, `:332`; `deal.rs:345`, `:503`, `:568`;
  `payment.rs:55`, `:96`, `:143`; `fulfilment.rs:45`; `method.rs:47`;
  `decline.rs:34`; `envelope.rs:7`, `:100`, `:102`;
  `record_signer.rs:56`, `:73`; `roym_core/src/backup.rs:40`;
  `messages.ts` is 1,018 lines and the form code is where the plan says
  (`:861`, `:956` to `:975`); `render.ts:36` does have the dead
  `booking-progress` case. The `send_card_and_file` callers are exactly the
  15 the plan lists. File sizes in 7.8 are right (`decide.rs` 715,
  `backup.rs` 727, `sections.rs` 707, `app.rs` 680, `payment_ops.rs` 656,
  `sync.rs` 635, `fixtures.rs` 773, parity `transaction.rs` 821 and its
  oversized entry, `roym_deal_e2e.rs` 714).
- **The hidden sets** match the agreements doc tables: 5 hidden fields in
  `DealTerms` (`listing_id`, `scope`, `cancellation_terms`, `refund_terms`,
  `dispute_path`), 10 in `Commitment`, 1 / 3 / 1 in the settlement records.
  `validate_commitment_kind` (`deal.rs:345`) reads only visible fields, so
  D-C2-17's split is clean. `validate_extension_and_details` and
  `validate_commitment_values` read hidden fields, and the plan moves them
  to "when shown" (D-C2-16) correctly.
- **Who reads a signed payload.** `QuotePayload` and
  `AgreementReceiptPayload` are used only in `roym_transaction` (and
  `roym_core`). No other Roym service, the Hub or `roymctl` parses a deal
  envelope. `thread.rs:72` to `:90` reads only `quote_id` and
  `terms.quote_expires_at_secs` (both stay visible) from a card's `data`.
  The Hub draws `card.data` and `agreement_payee` (from the plain
  `AgreementRow.terms`, `thread.rs:119`). So "keep `data` plain" works.
  Story 40 (`40-roym-languages.spec.ts:266`) calls `agreement.verify` and
  reads only `role`, `viewed_lang` and `viewed_via_mt`, which stay visible.
- **Access control.** No method becomes wire-reachable; `quote.measure`,
  `deal.present` and `deal.verify-presentation` all sit behind
  `admit::require_internal` like every `transaction` method
  (`app.rs:272`). The transaction service imports `syneroym:events` but
  does not use it, so no event row can carry a hidden value.
- **The 96 KiB number against the card.** The plan's arithmetic holds
  (disclosure array 98,800 = 96 KiB + 3 bytes of quoting and comma for each
  of at most 165 disclosures; card body 113,583 < 163,840). It is
  correct for the limit it names. P1 is about the limits after it.
- **Strict reveal and RFC 9901 section 7.1.** The plan's rules (every digest
  has a disclosure, no digest twice, name in the hidden set, name not a
  visible field, integers only, `_sd` strictly ascending) cover the RFC's
  reject cases. The name `_sd` itself can never pass, because it is in no
  hidden set.
- **Both builds.** `NativeAppHost` (`host.rs:668`), `GuestHost`
  (`guest.rs:431`), `HostState` (`capabilities_services.rs:52`) and `TestHost`
  (`testing.rs:213`) are all the `AppSigning` / `signing::Host`
  implementers. `crates/wit_interfaces/src/signing_host.rs` and `signing.rs`
  generate bindings from the WIT file, so they need no edit. The six Roym
  crates are the ones whose `world.wit` imports `syneroym:signing`.
  `check-roym-deps` already allows `syneroym-signed-record`, and that crate
  already has `sha2`.
- **Change docs.** `AG-*` rows and the encoding are in roym-agreements;
  VX-11 is in roym-value-exchange and is met through the AG rows, so no
  deviation row is needed there. The C2 deviation rows are already in the
  agreements doc (`## Deviations`, "Written when the M08C C2 slice was
  planned") and match the plan.
- **Plan form.** "How this is shown to an end user" is there, right after
  the goal, with 4 bullets for C2a, one plain sentence for C2b, and 3
  developer bullets. The status row for C1b already says "Merged".

### View on the owner questions

The owner has not answered Q1 to Q7. This is only the reviewer's view.

- **Q1 (split C2a and C2b).** Agree with (a). The two risks are different
  (wide and simple, against narrow and hostile input). One note: C2a is
  still large. The test migration in P2 is most of its size, so the plan
  should size it before the owner relies on "L".
- **Q2 (where salts come from).** Agree with (a). Parity needs the same
  bytes on both builds, and the host is the only shared place. The cost is
  larger than the plan says: it also touches the second parity harness and
  its guest fixture (P3). Option (b) is the weaker choice for the reason
  the plan gives. Option (c) is also right to reject.
- **Q3 (where the primitives live).** Agree with (a). No change to the
  dependency allow list.
- **Q4 (`quote.measure`).** Agree with (a), with one correction: the
  measure must count what `quote.set` adds before it signs (P4).
- **Q5 (rules that read a hidden field).** Agree with (a). The parties'
  nodes are the only ones that act on ML-18 and AG-39, and they hold the
  plain terms. Check that every place that files or imports a record
  really calls `check_viewed` (the plan lists them).
- **Q6 (no demo for C2b).** Agree with (a).
- **Q7 (decline note).** Agree with (a). The field tables are explicit, and
  a decline is never in a presentation.
- **New question for the owner (from P1): Q8.** Does the 96 KiB limit stay
  if the messaging path cannot carry a card that large? See P1 for the
  options.

### Findings

- [x] **P1 — major.** *Where:* plan section 2.2 point 1, the test
  `largest_quote_card_fits_in_the_card_body` (7.2), AG-40, and the C2
  deviation row "The 96 KiB limit against the card body (checked)".
  *Problem:* The plan checks 96 KiB only against `MAX_CARD_BODY_BYTES`
  (163,840). A card is not sent as its body. The conversation host sends
  `DeliveryPayload` with `body: Vec<u8>`, and `serde_json::to_vec` writes
  every body byte as a decimal number plus a comma
  (`crates/conversation/src/envelope.rs:24`, `crypto.rs:421` to `:423`).
  For a body of base64url text that is about 3 to 4 bytes per byte. Olm
  then encrypts it and the envelope is written as JSON again (I did not
  measure the Olm encoding). The sender's outbox and the mailbox have their
  own caps:
  - a deposit at the receiver's mailbox is sealed, and a sealed message may
    be at most `MAX_SEALED_BYTES` = 64 KiB
    (`crates/mailbox_format/src/lib.rs:37`, `message.rs:339`). The proxy's
    deposit path fails with `DepositFailure::Seal(TooLarge)`
    (`crates/router/src/proxy/mailbox.rs:60`);
  - a queued call may be at most 256 KiB (`proxy_outbox.rs:50`, checked at
    `:288`);
  - the conversation body limit is 262,144 (`conversation/src/store.rs:92`).
  So the real ceiling for a card that must go through the mailbox is
  roughly 13 to 16 KiB of card body, not 160 KiB. C2 makes every card
  bigger than today: base64url adds 33% to every text, and each hidden field
  adds a 22-character salt plus a 43-character digest. A typical
  multi-commitment quote with some text can pass 13 KiB.
  *Failure case:* Bob signs a quote whose card body is 25 KiB. Alice's node
  is offline and has a mailbox. `quote.set` answers ok, the quote is signed
  and stored, but the sender cannot seal the message for the mailbox, so it
  never reaches the `deposited` state. A quote of 100 KiB may also be
  refused by the outbox cap even when Alice's node is online but slow to
  answer. No test and no Hub line warns Bob. The "49 KiB to spare" in the
  plan and the deviation row is true only for the card-body check.
  *Fix:* (1) Measure the real expansion: send the largest card the plan
  builds through `conversation.send` and through the mailbox seal in an
  in-crate or two-node test, and record the numbers in the deviation row in
  place of the card-body-only number. (2) Put the owner's decision in Q8:
  keep 96 KiB and accept that a large quote reaches an offline peer only
  when it comes back online (state it in the change doc and the Hub line),
  or lower the disclosure limit to what the mailbox can carry, or ask
  `conversation` for a binary body encoding (a backlog row). (3) Whatever
  the owner decides, the Hub size line should show the number that can
  actually be delivered. (4) The two-node test must prove the decision: a
  quote near the chosen limit reaches an online peer and an offline peer
  with a mailbox.
  *Status:* Fixed (ba8c42a8): new owner question Q8 with the measured expansion (×3.36 as numbers, about ×4.47 after encryption) and the 64 KiB sealed cap, about 14 KiB of card body; recommendation (a) keep 96 KiB, the Hub notes when a quote is too large for a mailbox (`quote.measure` returns `card_bytes`, `mailbox_fits`), a backlog row for a compact body encoding. Step 6 measures the real ceiling (`MAILBOX_CARD_BODY_BYTES`, D-C2-22) and adds the two-node mailbox tests; the deviation row no longer claims 49 KiB to spare for delivery. Checked: a failed deposit falls back to direct delivery (`router/src/proxy/mailbox.rs:268` to `:275`).

- [x] **P2 — major.** *Where:* step 5 ("In-crate tests: section 7.2"),
  section 7.1 and 7.2, section 7.8.
  *Problem:* About 20 existing test files build the signed payload types
  directly with plain `DealTerms` or plain settlement fields, and they stop
  compiling when step 5 changes the types: `roym_core/src/record/tests/wire_words.rs`
  (9 sites), `transaction/tests.rs`, `transaction/tests/{viewed,verify,lang}.rs`,
  `deal/tests.rs`, `payment/tests.rs`, `fulfilment/tests.rs`, and in
  `roym_transaction/src/app/tests/` `restore.rs` (10), `recover.rs`,
  `decide.rs`, `settle.rs`, `accept.rs`, `time.rs`, `cancel.rs`,
  `decline.rs`, `decline/filing.rs`, `refuse/helpers.rs`,
  `refuse/writer.rs`, and `backup/tests.rs` (about 100 sites in all; counted
  with `grep -cE 'card_body\(|QuotePayload|AgreementReceiptPayload|…'`).
  The plan lists only the new test files and the parity fixtures. It names
  no shared test builder, so each file will grow its own "conceal and sign"
  helper. That is the "copied helper" kind of finding (R20 of C1). It also
  means the claim "after step 5 the `roym_core` and in-crate `transaction`
  tests pass" has no work behind it.
  *Failure case:* the implementer reaches step 5, finds 20 files that do not
  compile, and writes 20 small helpers or one in the wrong crate. The
  review then finds both copies and a test that signs a quote with a
  hard-coded salt that the real filer refuses.
  *Fix:* Add to step 4 one shared builder in `roym_core::testing` (it
  already holds `TestHost`): `conceal_quote(terms, salt_seed) ->
  (QuotePayload, Vec<String>)` and the same for the three settlement
  records and a `card_body` shortcut; step 5 lists the test files above and
  migrates each to it. In 7.8, name which of them are near the 800-line
  cap (`app/tests/restore.rs`, `transaction/tests.rs`) and where new cases
  go. `wire_words.rs` must keep scanning the plain form *and* the decoded
  disclosures (see P9).
  *Status:* Fixed (ba8c42a8): D-C2-20 adds one shared builder, `roym_core::testing::sd`; section 2.2 point 12 lists the ~20 files; step 5 moves each to it; 7.8 names the files near the cap and where new cases go; Q1 sizes C2a.

- [x] **P3 — major.** *Where:* step 3 ("`fresh-salts` on the host"),
  D-C2-13, and the plan's list of implementers.
  *Problem:* Adding a function to `syneroym:signing` and an argument to
  `NodeRecordSigner::with_clock` has callers the plan does not name:
  - the host-surface parity harness `crates/app_host_native/tests/dual_build_parity/`
    (`helpers.rs:674` and `:741` call `with_clock`; `signing.rs` tests every
    signing function on both builds, "function for function", with the
    guest fixture `test-components/dual-build-fixture`, whose generated
    `src/bindings.rs` and `{"op": …}` table must gain `fresh-salts`);
  - `crates/core/src/record_signer.rs:420` and `:425` (tests);
  - `permitted_differences.rs` (`:23`, `:89` use `RecordClock::Fixed`).
  The plan's only test for the host function is "one WASM test in
  `crates/sandbox_wasm`". The fixture is outside the workspace (it
  cross-compiles to `wasm32-wasip2`), so it will not fail the build; it will
  silently stay without the new function while the harness claims parity.
  *Failure case:* `cargo nextest run --workspace` fails to compile
  `app_host_native`'s parity tests after step 3. If the implementer only
  fixes the compile error, the two builds are never compared for
  `fresh-salts`, and a difference (for example, native returns 16 bytes and
  the guest sees an error for count 0) is found by users.
  *Fix:* Add to step 3: the three call sites above; a `fresh-salts` op in
  `dual-build-fixture` (rebuild it with the same command the other fixture
  changes use) and one scenario in `app_host_native` parity `signing.rs`:
  with `RecordSalts::Seeded` both builds return the same salts, and a count
  of 0 and of 257 give the same error on both. Add `dual-build-fixture` to
  the files that change in 7.8.
  *Status:* Fixed (ba8c42a8): step 3 and D-C2-13 name all six `with_clock` callers, `permitted_differences.rs`, the `fresh-salts` op in `dual-build-fixture` with its rebuild command, and two scenarios in `app_host_native` parity `signing.rs`; 7.8 names the fixture; exit criteria cover both parity harnesses.

- [x] **P4 — minor.** *Where:* D-C2-11, Q4, test `quote_measure_matches_quote_set`.
  *Problem:* `quote.set` fills the `window` of every slot commitment from
  the catalog before it builds the terms (`quote_ops.rs:133` to `:136`,
  `validate_and_apply_slot` at `:287` to `:340`), and `window` is a hidden
  field with a disclosure of about 90 characters. `quote.measure` as written
  builds `DealTerms` from the form and measures it without that step, so a
  quote with slot commitments is measured smaller than `quote.set` signs
  it. The difference is small (under 100 bytes per slot commitment) but
  `quote.measure` is the number the Hub shows as "fits".
  *Failure case:* the line says "95.9 KiB of 96 KiB", `quote.set` refuses
  `quote-too-large`.
  *Fix:* In `measure`, for each commitment with `giver` quote-author and
  `resource_kind` service-in-slot, set a placeholder window of the longest
  realistic digits (for example `{earliest_secs: 9_999_999_999,
  latest_secs: 9_999_999_999}`) when `window` is empty, and say in the
  method's doc that the answer is an upper bound for the slot case. The
  test must include one slot commitment.
  *Status:* Fixed differently (ba8c42a8): in place of a placeholder window, `quote.set` and `quote.measure` share one `prepare_terms` that runs the real slot step, so the measure is exact; the expiry is measured at its longest. `quote_measure_matches_quote_set` has a slot commitment.

- [x] **P5 — minor.** *Where:* step 4 (`DisclosureIndex::build`) and D-C2-7.
  *Problem:* The plan decodes and hashes every disclosure and only then
  checks the count. A stranger's card may be 160 KiB. A card can hold
  thousands of tiny disclosures that each decode, hash and go into a map
  before the first check fails. The cost is small, but the order matters
  for a verifier of stranger data, and `deal.verify-presentation` (C2b) has
  the same input with a 64-record limit and no disclosure-count limit.
  *Failure case:* 5,000 distinct short strings in one card cost 5,000
  decodes plus SHA-256 before refusal. Not a crash, but unbounded work per
  card is what these limits are for.
  *Fix:* Refuse before decoding any disclosure when the list is longer than
  the number of hidden fields the record can have (`5 + 10 × commitments`
  for a quote, 1 or 3 for a settlement record; for a presentation, the sum
  over its records). Add `disclosure-count-too-high` to `SdError` and one
  table case.
  *Status:* Fixed (ba8c42a8): D-C2-21, `DisclosureIndex::build(disclosures, max_count)` refuses a longer list, or one oversized disclosure, before any decode (`disclosure-count-too-high`); C2b uses the sum over records; two table cases in 7.1.

- [x] **P6 — minor.** *Where:* step 6 (parity, method shapes) and C2b step 2.
  *Problem:* `crates/roym_web/tests/method_contract/contract.rs:60` asserts
  `table.len() == 37` for the `transaction` method table. The same test runs
  `assert_service_table` (`roym_core/src/method/check.rs:33`), which refuses
  a method that does not route to the service, an input that does not
  deny unknown fields, and an output typed as a bare `Value`. Adding
  `quote.measure` (C2a) makes the count 38; C2b makes it 40. The plan
  names `shapes_transaction.rs` and `transaction_validation.rs` but not
  this count or these input and output rules for `quote.measure`,
  `deal.present` and `deal.verify-presentation`.
  *Failure case:* the contract test fails on a number, and the implementer
  may "fix" it without checking the table.
  *Fix:* Add the file and the two new counts to step 6 and to C2b step 2,
  and say that each new input type has `deny_unknown_fields` and each
  output is a typed struct.
  *Status:* Fixed (ba8c42a8): step 6 (37 → 38) and C2b step 2 (38 → 40) name `contract.rs:60`, and say each new input is `deny_unknown_fields` and each output a typed struct.

- [x] **P7 — minor.** *Where:* section 4 "Stays the same" (`roymctl`) and
  section 9.
  *Problem:* The plan says `roymctl` "prints plain `agreement.get` output"
  and stays the same. But `roymctl roym agreement payment verify` calls
  `payment.verify` and prints the verdict
  (`apps/roymctl/src/commands/roym/agreement.rs:292` to `:299`), and the
  same module has `fulfilment verify`, `quote verify` and `agreement
  verify`. After C2a the payload they print is the wire form (digests, no
  `method`, `reference` or note). The developer guide shows `roymctl`
  examples that print these verdicts.
  *Failure case:* an operator verifies a payment acknowledgement with
  `roymctl` and no longer sees the method, with no word in the docs.
  *Fix:* Change the sentence in section 4, list the four `roymctl` verify
  commands, and add one line to the developer guide: a verdict shows the
  signed form; disclosed fields appear only with C2b's `disclosures` input.
  Check `roymctl`'s own tests for a payload read.
  *Status:* Fixed in part (ba8c42a8): only one `roymctl` verify verb exists, `roymctl roym transaction payment verify` (`agreement.rs:110`, `:290` to `:298`); there is no `quote`, `agreement` or `fulfilment` verify verb, no guide example prints a verdict, and no `roymctl` test reads its payload. Section 4 and 2.2 point 13 now say so; step 5 changes that verb's help text; section 9 adds the guide line.

- [x] **P8 — minor.** *Where:* section 9 (living docs).
  *Problem:* Three more statements become false and are not listed:
  `docs/system-architecture.md:1215` ("`fulfilment-receipt` … carries
  `terms` with `terms.lang`", the plan names only `:1203` to `:1211`) and
  `:1415` ("the receipts' terms" in the record-text paragraph);
  `docs/developer-guide.md:1786` ("record text (… and the receipts'
  terms)"). The Hub still draws plain text from `card.data`, so `:1415` and
  `:1786` need only a clause: the receipt holds digests; the text on
  screen is the party's own plain copy.
  *Failure case:* a reader of the architecture doc believes a shown
  fulfilment receipt carries the terms.
  *Fix:* Add the three lines. The developer guide should also say that
  `quote.get`, `quote.list` and `quote.history` now return `terms`
  (section 4's table) and that `quote.measure` exists.
  *Status:* Fixed (ba8c42a8): section 9 adds architecture `:1213` to `:1215` and `:1415`, guide `:1786`, and the guide lines for `quote.get`/`.list`/`.history` `terms` and `quote.measure`.

- [x] **P9 — minor.** *Where:* step 5 and `crates/roym_core/src/record/tests/wire_words.rs`.
  *Problem:* That test scans every signed record sample for a fixed
  seller or buyer word (`wire_words.rs:1` to `:6`: "each sample fills every
  optional field"). After the switch the signed samples no longer hold the
  hidden text, so the scan passes without covering `scope`,
  `description`, notes and so on. The check loses most of its value without
  any failure.
  *Fix:* Scan the plain form (the one `CardRow.data` holds) and the decoded
  disclosure values of each sample in addition to the wire form.
  *Status:* Fixed (ba8c42a8): step 5 and 2.2 point 12: `wire_words.rs` scans the wire form, the plain form and each decoded disclosure value.

- [x] **P10 — minor.** *Where:* step 3 (`NodeRecordSigner::fresh_salts`,
  "`System` uses `getrandom::fill`").
  *Problem:* `crates/core/Cargo.toml` has `rand` (used at
  `event_bus.rs:112` with `rand::rng()`), and no `getrandom`. The plan does
  not say to add the dependency. `check-dep-versions` also asks for a
  `major.minor` requirement.
  *Fix:* Use `rand::rng().fill_bytes` (already in the crate, from the OS
  generator), or add `getrandom.workspace = true` and say so. Pick one in
  the plan.
  *Status:* Fixed (ba8c42a8): `System` salts use `rand::rng().fill_bytes`; `rand` is already in `crates/core/Cargo.toml:31`. No new dependency.

- [x] **P11 — minor.** *Where:* D-C2-7, D-C2-4 and the filers (step 5).
  *Problem:* The plan does not say what a filer does with a repeat of a
  record it already holds. Today `file_quote_card` marks a repeat
  (`already_held`, `sync.rs:477`) and still shows the card as verified. With
  C2a the same record may arrive again with no disclosures (a resend from an
  older node) or with the same set in another order. Two cases need a
  rule: the stored set must never be replaced by a different one, and a
  repeat that carries a valid set must not be refused as "salt repeated"
  because its salts are already in the store.
  *Fix:* State the rule: the strict reveal runs on the card's own set; the
  store write is "put if absent" (or equal); a repeat with no disclosures is
  filed as a repeat when the stored set exists. Add two cases to
  `disclosure.rs` in 7.2.
  *Status:* Fixed (ba8c42a8): D-C2-4 (the store is a `create`, never replaced, `disclosures-differ`) and D-C2-7 (the strict reveal runs on the card's own set; equal set or no set is a repeat); three cases in 7.2.

- [x] **P12 — minor.** *Where:* D-C2-4 (`DisclosureRow … index on
  conversation`).
  *Problem:* No code in the plan queries `DISCLOSURES` by `conversation`
  (the backup exports the whole collection; every other read is by
  `record_id`). The index costs a write on every row and a column in the
  schema for nothing. `QUOTE_HISTORY`, the sibling collection, has none.
  *Fix:* Drop the index and the `conversation` field, or name the reader
  that needs it (a conversation delete, if one exists, would be that
  reader; the plan does not describe one).
  *Status:* Fixed (ba8c42a8): D-C2-4 drops the `conversation` field and the index.

## Plan review round 2 — Claude Opus 5.5 (Claude Code), 2026-11-18

**Reviewer note.** This round runs on Claude Opus 5.5, the same model that
wrote the plan and folded round 1. So it does not meet the
session-strategy rule that a different tool reviews a plan. Round 1 was
Claude Sonnet 5.5. The owner decides whether another tool checks round 2.

Checked the plan at `4a312271` (branch `feat/m08c-c2-disclosure`, pulled;
fixes in `ba8c42a8`) against the code on this checkout (`main` is
`ba5a5597`). First each round 1 item was checked against the code. Then
the parts of the plan that the fix commit changed (Q1, Q2, Q4, Q8, D-C2-4,
D-C2-7, D-C2-11, D-C2-13, D-C2-20 to D-C2-22, steps 3, 5, 6 and 7, 7.1 to
7.10, 8, 9, 10) were reviewed again for new problems.

**Summary: round 1 items: 12 of 12 fixed and confirmed. New findings: 5
(0 blocker, 2 major, 3 minor). Open: 5.**

### Round 1 items

- **P1.** Confirmed. Q8 is accurate. A failed deposit logs "will try
  directly again" and returns the direct error
  (`crates/router/src/proxy/mailbox.rs:268` to `:275`). `classify` turns
  `Transport`, `Timeout` and `ServiceNotFound` into `Unreachable`
  (`crates/conversation/src/transport.rs:711`). The outbox defers an
  unreachable item with a backoff from its age and counts no attempt
  (`crates/conversation/src/outbox.rs:258` to `:265`), so the
  `max_pending_age_secs` of 30 days (`store.rs:95`) is the only limit, as
  Q8 says. The factors are consistent (3.36 × 4/3 ≈ 4.47; 65,536 / 4.47 ≈
  14.3 KiB). **Correction to my own round 1 text:** the 256 KiB cap of
  `proxy_outbox.rs:50` is not on the conversation path. `conversation` has
  its own queue and only mirrors the proxy outbox's dispositions
  (`transport.rs:47`, `:680`). The fix rightly does not cite it. The new
  tests for Q8 have problems of their own: P13 and P14.
- **P2.** Confirmed. Section 2.2 point 12 lists the files. The line counts
  in 7.8 are right (`decide.rs` 716, `recover.rs` 658, `backup/tests.rs`
  658, `accept.rs` 645, `refuse/writer.rs` 604, `restore.rs` 601,
  `settle.rs` 571, `wire_words.rs` 477, `transaction/tests.rs` 440).
  `roym_core::testing` is gated by `cfg(any(test, feature = "test-host"))`
  (`lib.rs:35`). `roym_transaction` already turns on `test-host` in its
  dev-dependencies (`Cargo.toml:41`), and `roym_web` does not yet, as the
  plan says. One gap in D-C2-20 is P16.
- **P3.** Confirmed. The six `with_clock` call sites are right. The fixture's
  `op` table is at `test-components/dual-build-fixture/src/app.rs:47`, and
  its build command is `mise.toml:44` with package
  `syneroym-test-dual-build-fixture`.
- **P4.** Confirmed. "Fixed differently" is better than my placeholder: one
  `prepare_terms` makes the measure exact.
- **P5.** Confirmed.
- **P6.** Confirmed.
- **P7.** Confirmed, and the fixer is right that my round 1 text was wrong.
  `apps/roymctl/src/commands/roym/agreement.rs` has one verify verb,
  `PaymentCommands::Verify` (`:111`, `:290` to `:298`). There is no
  `quote`, `agreement` or `fulfilment` verify verb.
- **P8.** Confirmed. `system-architecture.md:1213` to `:1215` and `:1415`
  and `developer-guide.md:1786` say what the plan quotes.
- **P9.** Confirmed.
- **P10.** Confirmed (`rand::rng()` is a CSPRNG seeded from the OS; the
  code needs `use rand::RngCore` for `fill_bytes`).
- **P11.** Confirmed: `AppDataLayer::create` exists
  (`crates/app_host/src/lib.rs:242`). It answers `Some(id)` when the row
  is already there, and the test host does the same
  (`roym_core/src/testing/data_layer.rs:181` to `:193`). One part of the
  rule cannot happen: P15.
- **P12.** Confirmed.

### View on the owner questions

Q1 to Q7: no change from round 1.

- **Q8 (96 KiB against the mailbox).** Agree with (a). The fallback to
  direct delivery is real and has no attempt limit, so a large quote is
  late, never lost within 30 days. Option (b) would block real quotes, and
  option (c) belongs to `conversation`. (a) works only if the Hub note is
  right. With D-C2-11 as written, the note shows for every quote (P14).

### New findings

- [x] **P13 — major.** *Where:* step 6 ("Two-node (Q8)"), 7.4, AG-40 in
  section 3, "How this is shown to an end user" (developer bullet), exit
  criteria.
  *Problem:* The two new two-node tests use "`common/mailbox_world.rs` for
  the mailbox and `Restart`". That world has a sender guest that queues
  calls, a mailbox node and a receiver with a *greeter*
  (`crates/substrate/tests/common/mailbox_world.rs:1` to `:3`). It has no
  `conversation` service and no Roym app. The only world with conversation
  messages and a mailbox is local to `mailbox_conversation_e2e.rs` (`World`
  at `:81`, `boot_b` at `:59`, using `conversation_fixture` and
  `conversation_peer::Peer`). It deploys a conversation fixture, not Roym.
  No test world has Roym and a mailbox together. Such a world is new
  harness work that the plan does not size (Q1's 3,800 lines do not include
  it). Each mailbox round also waits: `DEPOSIT_WAIT` is 60 s and
  `ANSWER_WAIT` 150 s (`mailbox_world.rs:36` to `:40`), so two Roym tests
  add several minutes to nextest.
  *Failure case:* the implementer reaches step 6 and either builds a
  second Roym-and-mailbox harness by hand (against "Reuse the test
  harness"), or drops the tests. Then the AG-40 row and the exit criterion
  have no test.
  *Fix:* Prove the delivery facts at the layer that owns them. Move
  `mailbox_conversation_e2e.rs`'s world into `common/` (or extend that
  file) and add two cases: a message whose body is a real quote card of
  `MAILBOX_CARD_BODY_BYTES` shows `deposited`; a body 2 KiB larger stays
  `pending` and becomes `delivered` after the peer restarts. These two
  cases also replace the "seals / does not seal" pair, with no copy of the
  encoding steps (see P17). Keep the Roym part in-crate:
  `quote_measure_says_when_a_card_does_not_fit_a_mailbox` already checks
  `mailbox_fits`. Update the AG-40 row, the developer bullet and 7.4 to
  match. If the owner wants a Roym-level mailbox test anyway, add the new
  world to Q1's estimate and to 7.8's time risks.
  *Status:* Fixed differently (38ca7afb): no Roym-level mailbox test. Step 6 adds one case (not two) to `mailbox_conversation_e2e.rs`, using its own `World`, with real quote card bodies from the shared builder; the in-crate `quote_measure_says_when_a_card_does_not_fit_a_mailbox` keeps the Roym side. AG-40, the developer bullet, 7.4, 7.8 and the exit criteria updated; `roym_disclosure_e2e.rs` is now a C2b file only.

- [x] **P14 — major.** *Where:* D-C2-11 (`card_bytes`,
  `card_body_upper_bound`), step 4 (`card.rs`), step 7, Q8 (a).
  *Problem:* `card_bytes` is "the length of `card_body` built with … a
  placeholder envelope whose issuer, signature and delegation certificate
  have their longest lengths". The longest delegation certificate the
  signer accepts is 16 KiB (`crates/core/src/record_signer.rs:121`). As an
  escaped string inside the card, that alone is more than the about 13 KiB
  that `MAILBOX_CARD_BODY_BYTES` will be (D-C2-22). So `mailbox_fits` is
  false for every quote, and the Hub shows "This quote is large…" under
  every quote form. A real certificate is far smaller.
  *Failure case:* Bob writes a one-line quote of ₹100. The form says "2.1
  KiB of 96 KiB" and, under it, that Alice gets it only while both devices
  are online. That is false, and it teaches people to ignore the note.
  Story 50 checks only "of 96 KiB", so no test catches it.
  *Fix:* Measure with the certificate this node really signs with:
  `quote.set` already resolves it (`resolve_principal_and_owner`,
  `quote_ops.rs:106`); `measure` calls the same function and passes the
  real `delegation_json` (or none for `Principal::Service`) to
  `card_body_upper_bound`. The issuer (`did:key`) and the signature have
  fixed lengths. Only the timestamps and the record id need a longest
  value. Add a case to `quote_measure_says_when_a_card_does_not_fit_a_mailbox`:
  a typical two-commitment quote has `mailbox_fits: true`, and
  `card_bytes` is at least the real card's length for the same terms.
  *Status:* Fixed (38ca7afb): D-C2-11 measures with the certificate from `resolve_principal_and_owner`; `card_body_upper_bound` takes `delegation_json`; the 7.2 test gains the typical-quote and upper-bound cases.

- [x] **P15 — minor.** *Where:* D-C2-4, D-C2-7 (`disclosures-differ`),
  the 7.2 test `a_repeat_with_a_different_set_is_refused_and_the_first_set_stays`,
  and the agreements doc's new row "A stored set of disclosures is never
  replaced".
  *Problem:* A set that passes the strict reveal for a record is the only
  such set. Every digest in `_sd` must be matched, each digest is the
  SHA-256 of exactly one disclosure string, and no extra disclosure is
  allowed. So two different sets cannot both pass for one `record_id`,
  short of a SHA-256 collision. The strict reveal runs before the store
  (D-C2-7). So `disclosures-differ` cannot happen on filing. The import
  also checks each set strictly first (D-C2-12). The named test cannot be
  built with a valid second set. An implementer will then build it with an
  invalid set, which the strict reveal refuses with another code, and the
  test proves something other than its name.
  *Fix:* Drop `disclosures-differ` from `SdError` and from the doc row.
  Keep the `create` write. It protects the stored set at no cost. State the
  reason: a strictly valid set is unique. Rename the test to
  `a_repeat_with_a_bad_set_is_refused_and_the_stored_set_stays`. Change the
  doc row to: "A repeat with the same set, or with none, is filed as a
  repeat; a repeat whose set fails the strict reveal is refused; the
  stored set is never replaced."
  *Status:* Fixed (38ca7afb): `disclosures-differ` dropped from `SdError`, D-C2-4, D-C2-7 and the agreements doc row, with the reason; test renamed to `a_repeat_with_a_bad_set_is_refused_and_the_stored_set_stays`; the same-set test also tries another order.

- [x] **P16 — minor.** *Where:* D-C2-13, D-C2-20 (`seeded_salts`, "the same
  rule as `RecordSalts::Seeded`"), `TestHost`'s counter.
  *Problem:* The seeded rule (`SHA-256(seed ‖ n)`, first 16 bytes) is to
  exist in three places: `RecordSalts::Seeded` in `crates/core`,
  `TestHost`'s counter and `roym_core::testing::sd::seeded_salts`. The two
  in `roym_core` cannot reach the one in `crates/core`, and they cannot add
  `sha2`. `test-host` is a feature of `roym_core`, so its code may use only
  `roym_core`'s `[dependencies]`, which `check-roym-deps` limits
  (`xtask/src/main.rs:182` to `:196`). So the plan asks for three copies
  of one rule, and nothing keeps them equal. The parity harness depends on
  them being equal.
  *Fix:* Put one `pub fn seeded_salt(seed: u64, n: u64) -> Salt` in
  `syneroym_signed_record::disclosure` (that crate has `sha2`, and both
  `syneroym-core` and `roym_core` already depend on it). Call it from
  `RecordSalts::Seeded`, `TestHost` and `testing::sd`. Add one unit test
  that pins its first output.
  *Status:* Fixed (38ca7afb): one `seeded_salt(seed, n)` in `syneroym_signed_record::disclosure`, called by `RecordSalts::Seeded`, `TestHost` and `testing::sd`; pinned by `seeded_salt_first_output_is_pinned`.

- [x] **P17 — minor.** *Where:* step 6, "The mailbox size" (the
  measurement test).
  *Problem:* The test "builds the same `DeliveryPayload` encoding, Olm
  encryption and `deliver` request that `transport.rs:210` to `:230`
  builds", and adds a `test-support` function only "if the encoding steps
  are not reachable". A hand-built copy measures the copy. If
  `conversation` later changes its encoding, the copy still passes and
  `MAILBOX_CARD_BODY_BYTES` becomes wrong. The input is also "bytes of
  base64url text". The real card body is JSON with an escaped envelope,
  and the number-array growth depends on the byte values (bytes 100 and
  up, such as the letters `d` to `z`, take three digits). So the factor
  differs a little from a real card.
  *Fix:* Measure through the real path only. Either use P13's
  conversation-level two-node cases, or add the `test-support` function
  unconditionally: `sealed_size_for_body(body: &[u8]) -> usize` in
  `conversation`, which calls the real encode and encrypt functions. Feed
  it a real quote card built with the shared builder (D-C2-20), not
  synthetic text.
  *Status:* Fixed (38ca7afb): the hand-built copy and its fallback are gone. D-C2-22 sets the value from the real delivery path (P13's case and the router's deposit warning, which names the sealed size); the bodies are real quote cards.

## Plan review round 3 — Claude Opus 5.5 (Claude Code), 2026-11-18

**Reviewer note.** This round also runs on Claude Opus 5.5, the plan's own
model. The same note as round 2 applies.

Checked the plan at `7e791833` (fixes in `38ca7afb`) against the code on
this checkout. First each round 2 item was checked against the code. Then
the parts that the fix commit changed (the status line, 2.1, Q1, AG-40 in
section 3, D-C2-4, D-C2-7, D-C2-11, D-C2-13, D-C2-20, D-C2-22, steps 1, 3,
4 and 6, 7.1, 7.2, 7.4, 7.8, 10, and the agreements doc's new rows) were
reviewed again for new problems.

**Summary: round 2 items: 5 of 5 fixed and confirmed. New findings: 2
(0 blocker, 0 major, 2 minor). Open: 2.**

### Round 2 items

- **P13.** Confirmed. `mailbox_conversation_e2e.rs` is 226 lines. `World`
  is at `:45` and `world()` at `:81`. It has `stop_b`,
  `restart_b_and_pull` and `deposits_so_far`. `send_message` takes any
  body text (`common/conversation_fixture.rs:194`). The budgets are at
  `:37` to `:39`, not `:36` to `:38` (no effect).
  `crates/substrate/Cargo.toml:134` is the `syneroym-roym-core`
  dev-dependency, and its `test-host` feature has no dependencies of its
  own (`crates/roym_core/Cargo.toml:11`). The direct retry after the
  restart fits the budget: the outbox retries on a backoff by the
  message's age (`conversation/src/outbox.rs:655`). Between 127 s and
  255 s of age the step is 128 s. Each try costs at most 5 s
  (`MAILBOX_DIRECT_ATTEMPT_TIMEOUT`, `router/src/proxy/mailbox.rs:30`).
  So at least one try falls inside the 150 s `ANSWER_BUDGET` after B is
  back.
  One small difference, inside the 1 KiB margin: the fixture sends
  `text/plain` (`dual-build-fixture/src/app/dispatch.rs:198`), and Roym
  sends `application/vnd.roym.card+json` (`roym_core/src/card.rs:27`).
  This is 20 more bytes in `DeliveryPayload.content_type`, or about
  90 sealed bytes. The value the test measures is therefore slightly
  high for a real card, by about 20 body bytes. The margin covers it.
- **P14.** Confirmed. `resolve_principal_and_owner` is at `app.rs:281`,
  and `quote.set` calls it at `quote_ops.rs:106`. One gap in the new text
  is P19.
- **P15.** Confirmed. `disclosures-differ` is gone from `SdError`, D-C2-4,
  D-C2-7 and the agreements doc row, and the reason is stated.
- **P16.** Confirmed. `check-roym-deps` allows `syneroym-signed-record`
  (`xtask/src/main.rs:188`). `roym_core` (`Cargo.toml:15`) and
  `syneroym-core` (`Cargo.toml:26`) both depend on it, and it has `sha2`
  (`crates/signed_record/Cargo.toml:15`).
- **P17.** Confirmed. The hand-built copy is gone. The router's warning
  carries the seal error (`router/src/proxy/mailbox.rs:268` to `:274`),
  and that error names the sealed size and the 64 KiB maximum
  (`mailbox_format/src/deposit.rs:49`).

### New findings

- [x] **P18 — minor.** *Where:* step 6, "The mailbox size", the case
  `a_card_of_the_mailbox_size_waits_at_the_mailbox_and_a_larger_one_at_the_sender`;
  7.8 (time).
  *Problem:* The case proves that the larger card is not deposited by
  waiting the full `DEPOSIT_BUDGET` (60 s) and seeing no `deposited`.
  That proves only that nothing happened. A card that was never tried
  also shows no `deposited`.
  *Failure case:* An outbox change stops the sender from trying the
  larger card while B is away. The case still passes, and it costs a
  fixed 60 s on every run.
  *Fix:* Wait for a positive signal. The router counts each failed
  deposit as `substrate.proxy.mailbox.deposit_failed`
  (`router/src/proxy/mailbox.rs:269`), in the same process recorder that
  `deposits_so_far()` reads. Wait until that counter grows. Then check
  that the larger card's state is `pending` and that
  `deposits_so_far()` grew by one only. Drop the 60 s wait from 7.8's
  estimate.
  *Status:* Fixed (393669ec): step 6 reads `deposit_failed` before the larger card and waits until it grows, then checks `pending` and one deposit only; 7.8 drops the fixed wait. The budget lines (`:37` to `:39`), the retry timing and the content-type note from the round 3 confirmations are also in step 6.

- [x] **P19 — minor.** *Where:* D-C2-11 (`quote.measure`'s errors and the
  "service principal" branch).
  *Problem:* `measure` now calls `resolve_principal_and_owner`. That
  function refuses when the node has no owner, when signing is not
  enrolled, and when the certificate is expired or stale (`app.rs:285` to
  `:305`). D-C2-11 lists only "a body that does not parse, or a slot step
  that fails" as errors. Also, "or none for a service principal" cannot
  happen: `person_principal` always returns `Principal::Delegated`
  (`roym_core/src/signing.rs:211`).
  *Failure case:* A person who is not enrolled opens the quote form. The
  Hub's size line gets `signing-not-enrolled`, a case the plan does not
  name, so the Hub code and the Hub unit test do not cover it.
  *Fix:* In D-C2-11, say that `measure` answers the same signing errors
  as `quote.set`, and that the Hub hides the size line on any error.
  Remove the "service principal" branch, and make
  `card_body_upper_bound`'s `delegation_json` a `&str`, not an `Option`.
  *Status:* Fixed (393669ec): D-C2-11 lists the parse, slot and signing errors and says the Hub hides the line on any of them; the service principal branch is gone and `delegation_json` is `&str`; new service test `quote_measure_answers_signing_not_enrolled_like_quote_set` and a `signing-not-enrolled` Hub case.

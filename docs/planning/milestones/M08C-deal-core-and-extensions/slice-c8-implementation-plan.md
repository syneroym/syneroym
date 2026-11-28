# M08C slice C8 — Close-out: implementation plan

*Reader: the developer (or AI) who does slice C8, and the project owner,
who answers the questions in section 2.1. You do not have the planning
conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-28, by Claude Opus 5.5, on
`feat/m08c-c7b-record-forward` (that is `main` at `79344661` plus the
C7b work of PR #400). PR #400 merged on 2026-11-28 as squash commit
`954af3fc`. Plan review round 1 is in
[slice-c8-review.md](slice-c8-review.md). Every fact about code and docs
below was read on `main` at `79344661` (`git show origin/main:…`,
`git grep … origin/main`), unless it says "C7b". The owner has not
answered the questions in section 2.1.**

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - "Writing for readers": every living doc and change doc that C8 edits
    is written in short sentences and simple English, with exact names;
  - comments and test names say why, never which plan, slice or review
    finding (C4 R28, C1 R42(b));
  - "Mandatory Living-Docs Close-out", "Mandatory Deferred-Backlog
    Update" and "Mandatory Demo Close-out";
  - no hard-coded ports in a new substrate test (`common::alloc_ports`,
    `SubstrateTestContext`);
  - test files at most 800 lines; functions at most 100 lines;
  - parity runs need a fresh `mise run build:roym` (no parity scenario
    is added here, but the whole parity binary runs in `mise run verify`).
- [task.md](task.md), section 3 (C8), section 5 (criteria finished later),
  section 7 (backlog rows), and section 4 (order: C8 comes after C6 and
  C7).
- The four change docs, their acceptance tables and their `## Deviations`:
  [roym-agreements](../../changes/roym-agreements/change.md) (`AG-*`),
  [roym-extensions](../../changes/roym-extensions/change.md) (`EX-*`),
  [roym-message-forwarding](../../changes/roym-message-forwarding/change.md)
  (`FW-*`), and the overview
  [roym-value-exchange](../../changes/roym-value-exchange/change.md)
  (`VX-*`). In each doc, the `## Deviations` win over the main text.
- The fifth doc that task.md's "Change docs" table names for M08C:
  [roym-multilingual](../../changes/roym-multilingual/change.md) (status
  `done`). The agreements doc's "Differences" table asks for one change
  there (D-C8-6).
- [prompt-templates.md](../../prompt-templates.md), "T8 — Close a
  milestone". C8 overlaps with it (Q1).
- The review files of C1 to C7. What C8 must not repeat is in
  section 7.6.

### Terms used in this plan

- **Criterion row:** one row of a change doc's "Acceptance criteria"
  table, for example `AG-9`. Its last column is **Test**.
- **Named test:** a test file plus the test function names (or a parity
  scenario number, or a Playwright case number) that prove the row. A
  test kind alone ("Unit.", "Service.", "Two-node.") is not a named test.
- **Gap:** a criterion row where no test on `main` checks one part of
  what the row says.
- **Differences table:** the table "Differences from the overview and
  sibling changes" in the agreements doc (`change.md:1060`). Each row
  names a rule of another doc that the agreements doc changed, and the
  doc that C8 updates.
- **Living docs:** `docs/system-requirements-spec.md`,
  `docs/system-architecture.md`, `docs/roym-integrated-experience-spec.md`,
  `docs/developer-guide.md`, `docs/TERMINOLOGY.md`,
  `docs/planning/traceability-matrix.md`, `docs/demo/demo.md`.

## 1. What C8 does, in one paragraph

C8 closes M08C. It writes a named test into every built criterion row of
the agreements, extensions and forwarding docs, and fixes the test names
that do not exist. It adds the small tests that are missing, where a row
asks for something that no test checks (Q2). It does not change product
code. It makes the living docs describe what M08C built: the services
table, the records table and the state diagram of the integrated spec,
the deal workflow of the requirements spec, the slot rule and the states
in the architecture doc, and the scheduling methods in the developer
guide. It applies each row of the Differences table to its target doc.
It sets `roym-agreements` and `roym-extensions` to `status: done`.
`roym-message-forwarding` is already `done` (C7b). The overview stays
`draft`, and its M08C task is ticked. It closes the task.md section 7
backlog rows and fixes the backlog rows that M08C made false. It brings
status.md, task.md, the M08 overview and the roadmap up to date (Q1).

## How this is shown to an end user

An end user sees nothing new from C8. It adds tests and fixes documents;
the Hub and the node behave exactly as before.

How a developer can see it work:

- Open any of the three change docs: every built `AG-*`, `EX-*` and
  `FW-*` row names a test file and test function, and
  `git grep -n 'fn <name>'` finds each one.
- `cargo nextest run -p syneroym-roym-transaction` and
  `cargo nextest run -p syneroym-substrate --test roym_deal_e2e`: the new
  tests of section 7 pass.
- `cargo xtask check-change-docs`: passes with two more docs at
  `status: done`.

## 2. Points for the owner

### 2.1 Questions

#### Q1. Does C8 also do the milestone close (template T8)?

T8 ("Close a milestone") repeats most of C8: check every criterion with
its test, check the living docs and the backlog, and mark the milestone
done in `M08-overview.md` and task.md. T8 is meant as a fresh audit by
the tool that reviewed most slices, and it changes no code.

- **(a) C8 does both. The C8 code review uses the T8 checklist. No
  separate T8 session.** C8 marks M08C done in `M08-overview.md`, task.md
  and `roadmap.md`, in the same PR. The code review of C8 is done by a
  different tool, so it is the fresh audit.
- (b) C8 does task.md section 3 only. A T8 session follows, after C8
  merges, and marks the milestone done.
- (c) Skip C8 as a slice and run T8 alone. T8 changes no code, so the
  missing tests (Q2) need their own fix slice.

**Recommendation: (a).** Two sessions would read the same 30,000 lines
of plans and reviews, and the second would find what the first already
fixed. The cross-tool rule holds: C8's code reviewer is not its
implementer.

#### Q2. What does C8 do where a row has no test for one of its parts?

The check found gaps in 13 rows (section 3, rows marked **gap**). Each
missing test is small: one service test, one unit test, one Hub test,
or one more step in an existing two-node test. None needs new product
code, if the code does what the row says.

- **(a) C8 adds the missing tests (section 7.1, T1 to T13).** If a new
  test shows that the code does not do what the row says, C8 stops on
  that row. It records the gap as a backlog row and asks the owner
  whether a fix slice is needed. C8 itself never changes product code.
- (b) C8 adds no tests. Each gap becomes a Deviation in its change doc
  ("not tested") and a backlog row.
- (c) Split: C8a (the tests, with full `mise run verify` and a code
  review) and C8b (docs only, with `check-change-docs`).

**Recommendation: (a).** task.md says each row "names a test on `main`".
A row whose test checks only half of it would be a false claim in a
`done` doc. The tests are cheap. One PR keeps the evidence and the
claims together.

#### Q3. Criterion text that a Deviation narrowed

Some rows say more than was built, and a Deviation already says so. For
example, EX-5 says "the deal core calls the reserve hook … on the
resource owner's node", but each giver reserves on its own node (the
extensions doc, Deviations, C3b1). The Differences table says the target
doc is "updated at close-out".

- **(a) Keep the criterion text as written. In the Test column, after the
  named tests, add "Narrowed: see Deviations, \"<bullet title>\"."** Fix
  only design sentences that state a false fact (D-C8-4).
- (b) Rewrite each criterion's text to say what was built.

**Recommendation: (a).** The criterion text is the record of what was
asked. The Deviation is the record of what changed and why. Rewriting
the criterion loses the first record and makes the Deviation look wrong.

#### Q4. The backlog row whose target is "M08C C3c2 follow-up"

The row "The two-node as-listed test, parity 255, the native wiring test
and the extra Playwright cases are not built"
(`deferred-backlog.md:690`) has target "M08C C3c2 follow-up (step 8)".
None of them was built. The owner approved this cut in C3c R22, with a
pickup trigger.

- **(a) Target "M08D", because M08D D3 changes `as_listed`; keep the
  pickup trigger.**
- (b) Target "TBD", with the pickup trigger only.
- (c) Build them in C8.

**Recommendation: (a).** T8 asks that a row aimed at the closing
milestone moves to a named later milestone with a reason. M08D rewrites
the same code, so a two-node test written then tests the final shape.

#### Q5. Hindi messages whose owner review is not recorded

status.md records an owner review for C1a, C2a, C3b1, C3b2, C4b and C6b.
For these rows it lists messages as "ready for owner review" or "Open",
with no later note that the owner reviewed them:

- C1b: 7 keys (`decline-title`, `messages-decline-note-placeholder`,
  `messages-decline-note`, `quote-superseded`, `quote-expired`,
  `progress-fork`, `messages-waiting-for-other-side`);
- C3a1 and C3a2: 6 keys (`ext-details-heading`, `ext-not-checked`,
  `ext-scheduling-slot-listing-id`, `ext-scheduling-slot-slot-id`,
  `ext-scheduling-step-started`, `messages-accept-extension-unknown`);
- the Engage fix: `directory-engage-button`;
- C7a: the `forward-*` keys of its row; C7b: the `record-forward-*` and
  `contact*` keys of its row.

- **(a) C8 copies every such key, with its English and Hindi text from
  `en.ftl` and `hi.ftl`, into one table in the C8 row's evidence. The
  owner reviews them once, before C8 merges.**
- (b) The owner says they are already reviewed; C8 records that.

**Recommendation: (a),** unless (b) is true. A milestone should not close
with user-visible text that nobody checked.

#### Q6. The agreements doc's `review.md`

`docs/planning/changes/roym-agreements/review.md` holds the review rounds
of the agreements doc (P-ids). The rule for doc rounds is to remove the
working files and keep `change.md`. But four planning files link to it:
`slice-c1-review.md:74`, `slice-c3a-implementation-plan.md:45`,
`slice-c3c-implementation-plan.md:101` and `task.md:278`.

- **(a) Keep it.**
- (b) Remove it and change the four links to point at the change doc.

**Recommendation: (a).** It records owner decisions that the change doc
cites (for example open question 14, P45), and the links show that the
slice plans relied on it. Removing it saves nothing.

#### Q7. The branch name

The prompt for this plan named `feat/m08e-c8-closeout`. The slice is in
M08C, and earlier branches are `feat/m08c-…`.

- **(a) `feat/m08c-c8-closeout`.**
- (b) `feat/m08e-c8-closeout`, as given.

**Recommendation: (a).**

### 2.2 Points found while reading

1. **C8 must wait for C7b.** FW-7 is built only in C7b (PR #400). On
   `main`, the Test column of FW-1 to FW-15 is empty; C7b fills it and
   sets the forwarding doc to `status: done`. Step 0 starts only after
   PR #400 is merged.
2. **status.md is behind git.** On `main`, the rows of C0, C4a, C7a and
   the Engage fix say "Implemented" or "Code reviewed". Git has them
   merged: C0 as ADR-0030 (PR #366), C4a as `933fb399` (PR #383), C7a as
   `79344661` (PR #399), the Engage fix as `98d21db9` (PR #373). The C7
   row says "Plan reviewed (2)". C8 sets each to "Merged" with its PR.
3. **The card count is wrong in two change docs.** The code has 10 card
   types (`crates/roym_core/src/card.rs:11` to `:22`: enquiry, quote,
   agreement-receipt, agreement-progress, payment-request,
   payment-acknowledgement, fulfilment-receipt, deal-decline, step,
   commitment-failure). Widgets are chat messages, not cards (extensions
   doc, Deviations, C4a). But the agreements doc says "the fixed card
   table grows from seven types to twelve", counting `widget` and
   `widget-answer` (`change.md:1026`), its Differences row says "It grows
   to 12" (`:1072`), and the extensions doc says the cards are "`step`,
   `widget` and `widget-answer` … from seven types to ten"
   (`roym-extensions/change.md:149`). The C4 plan (`:398`) left this to
   C8. The integrated spec's card table is already right ("ten templates
   plus one fallback").
4. **Five test names in the agreements doc do not exist** (C4 R2 was the
   same kind of finding). AG-44: `a_quote_matching_terms_from_listing_is_auto_accepted`
   (the real test is `a_quote_with_the_listed_terms_is_accepted_with_no_person`,
   `as_listed_accept.rs:188`). AG-45: `cancel_of_unconfirmed_instant_agreements_frees_their_slots`
   (none) and `a_failing_certificate_on_transaction_answers_as_confirm_first`
   (real: `a_failing_certificate_makes_the_offering_wait`,
   `as_listed_owner.rs:312`). AG-49: five names (real ones in section 3).
   AG-50: `driver_e2e_advances_quotes_without_client_sync` (real:
   `two_nodes_reach_agreement_through_driver_without_manual_sync`,
   `roym_driver_e2e.rs:135`). AG-51: `formed_as_is_recorded_in_snapshot`
   and the file `tests/presentation.rs` (the file is `tests/present.rs`
   and has no `formed_as` test). `check-change-docs` does not check test
   names, so step 1 checks each with `git grep -n 'fn <name>'`.
5. **AG-9 names the wrong refusal code.** A cancel by anyone but the
   quote author is refused with `quote-author-only`
   (`crates/roym_transaction/src/app/progress_ops.rs:238`). A cancel after
   a commitment moved is refused with `commitment-moved` (`:92`). The row
   says every other cancel gets `commitment-moved`. No test checks
   `quote-author-only`. C8 adds the test (T3) and a Deviation bullet
   (D-C8-5).
6. **The two EX-18 test paths are both right.** The criterion row names
   `crates/roym_core/src/as_listed/tests.rs` (the `terms_from_listing`
   tests). The Deviation names `crates/roym_core/src/listing/as_listed/tests.rs`
   (the `listing.set` instant rule, `:98`, `:111`). These are two modules.
   Nothing to fix.
7. **The Playwright deal cases use a stubbed RPC.** All five cases of
   `crates/substrate/tests/e2e/tests/roym-deal.spec.ts` replace the node
   with `page.route('**/rpc')` (lines 90, 113, 211, 367, 477). They test
   the Hub only. The rows that name them (AG-23, AG-46 to AG-48, AG-52)
   ask for a Hub E2E test, so this is enough. Story 50 is the real
   two-node browser run.
8. **The agreements doc's Tasks list is not ticked for C1 to C3, the
   owner review and the C0 ADRs** (`change.md:1210` to `:1214`). C1 to
   C3c are merged. The owner review happened (task.md section 3, "The
   agreements doc gets one more review first"). Of the "two C0 ADRs",
   ADR-0030 (forwarding) was written; the deal-grants ADR moved out of
   M08C with C5 (task.md section 6, question 3). Open question 2 ("belongs
   to a C0 ADR") is decided by ADR-0030.

## 3. What C8 meets

C8 meets task.md's C8 bullets. It meets no criterion row of its own. This
section is the full check of every M08C row. For each row it gives the
named tests on `main` that C8 writes into the Test column, and what C8
must add. Paths: `RC` = `crates/roym_core/src`, `RT` =
`crates/roym_transaction/src/app`, `RS` = `crates/roym_scheduling/src/app`,
`PAR` = `crates/roym_web/tests/dual_build_parity`, `ST` =
`crates/substrate/tests`, `UI` = `crates/roym_web/ui/src`.

**ok** = the named tests cover the row. **names** = the row names tests
that do not exist; C8 writes the real ones. **gap** = one part has no
test; C8 adds the test in section 7.1 (if Q2 (a)).

### 3.1 Agreements (`AG-*`), owned by the agreements doc

| Row | Result | Tests on `main` to write in the Test column | C8 adds |
| --- | --- | --- | --- |
| AG-1 | gap | `ST/roym_deal_e2e.rs` `each_party_quotes_the_other_at_once` (`:120`) only sends both quotes; nobody accepts | T1 |
| AG-2 | ok | `RC/deal/tests.rs` `every_giver_can_give_every_built_resource_kind` (`:117`); `ST/roym_deal_e2e.rs` `service_for_service_and_goods_deals_close` (`:247`) | — |
| AG-3 | ok | `RC/deal/tests.rs` `quote_for_an_enquiry_and_for_a_want_give_equal_receipts` (`:383`) | — |
| AG-4 | ok | `RC/deal/tests.rs` `commitment_count_table` (`:143`), `missing_or_wrong_commitment_fields_are_refused` (`:173`); `ST/roym_statement_e2e.rs` `both_sides_close_a_three_commitment_deal` (`:116`) | — |
| AG-37 | ok | `RC/deal/tests.rs` `commitment_ids_must_follow_their_position` (`:189`); `RT/quote_ops/tests.rs` `deal_terms_input_assigns_commitment_ids_and_expiry` (`:10`) | — |
| AG-5 | ok | `RC/deal/tests.rs` `unknown_currency_is_refused` (`:202`) | — |
| AG-6 | ok | `RC/deal/transition/tests.rs` `fulfilment_table_covers_every_state_and_event` (`:14`), `currency_table_covers_every_state_and_event` (`:97`), `progress_apply_closes_only_when_every_commitment_is_final` (`:310`) | — |
| AG-7 | ok | `RT/tests/time.rs` `unsettled_commitment_ends_unconfirmed_after_its_track_end` (`:216`), `tick_before_the_end_changes_nothing` (`:307`) | — |
| AG-8 | gap | `RC/deal/transition/tests.rs` `ended_unconfirmed_says_whether_the_giver_claimed` (`:224`); `UI/deals/agreement_panel.test.ts` "renders ended-unconfirmed with two sentences" (`:201`). The presentation test `presentation_without_disclosures_shows_kinds_and_outcomes` (`RC/presentation/tests.rs:520`) holds no `ended-unconfirmed`, `failed` or `cancelled` commitment | T2 |
| AG-9 | gap | `RT/tests/cancel.rs` `cancel_is_refused_once_a_commitment_moved` (`:253`), `cancel_releases_writer_seat_holds` (`:290`); `RC/deal/transition/tests.rs` `cancel_needs_every_commitment_open` (`:428`). Nothing checks "only the quote author cancels" | T3; Deviation (D-C8-5) |
| AG-10 | ok | `ST/roym_deal_e2e.rs` `decline_reaches_the_other_node` (`:377`), `enquiry_decline_and_withdrawal_reach_the_other_node` (`:449`); `RT/tests/decline.rs` `counterparty_can_decline_quote_v2_after_v1_refused` (`:478`) | — |
| AG-11 | ok | `RC/progress/tests.rs` `snapshot_from_another_signer_is_not_filed` (`:329`); `PAR/booking.rs` scenario 157 (`:561`, the lower `seq`) | — |
| AG-12 | gap | `RT/tests/refuse/version.rs` `accepting_a_superseded_version_is_refused` (`:19`); `ST/roym_deal_e2e.rs` `refused_superseded_acceptance_shows_on_that_version` (`:517`) does not check that the deal is still `quoted`; `RT/tests/refuse/writer.rs` `two_syncs_send_one_refusal` (`:106`); `RT/sync/tests.rs` `test_progress_fork_detection` (`:12`) | T4 |
| AG-35 | gap | `RT/tests/refuse/version.rs` `revision_after_an_end_is_refused` (`:184`: expired, declined), `revision_after_acceptance_is_refused` (`:86`). Not after `not-formed`, `cancelled`, `closed`; no test of two quote ids for one enquiry that both form | T5 |
| AG-43 | ok | `RT/tests/refuse/writer.rs` `two_syncs_send_one_refusal` (`:106`), `a_countersigned_half_before_expiry_still_forms_the_agreement` (`:201`); `RT/tests/accept.rs` `counterparty_files_author_countersigned_half_delivered_after_expiry` (`:602`) | — |
| AG-41 | ok | `RT/tests/accept.rs` `authors_own_accept_is_refused` (`:242`); `RT/tests/decide.rs` `writer_countersigns_in_sync_and_sends_snapshot_one` (`:251`) | — |
| AG-13 | ok | `RT/tests/accept.rs` `repeated_accept_returns_the_same_half` (`:273`); `RT/tests/settle.rs` `repeated_sign_and_ack_return_the_same_record` (`:492`); `RT/tests/decline.rs` `repeated_decline_returns_the_same_record` (`:177`) | — |
| AG-42 | ok | `RT/tests/restore.rs` `restored_writer_continues_seq_from_its_cards` (`:274`), `restored_writer_reuses_its_sent_countersign` (`:386`), `restored_writer_paginates_to_find_countersign_after_sync_window` (`:531`) | — |
| AG-14 | gap | `ST/roym_booking_e2e.rs` `a_losing_concurrent_booking_is_arbitrated_and_the_winner_completes` (`:198`); `RT/tests/decide.rs` `losing_the_last_seat_ends_not_formed` (`:286`); `RT/tests/recover.rs` (`:241` to `:592`, eight crash and recovery tests). `refused_confirm_stub_ends_not_formed` (`:568`) has one commitment: no test refuses a `confirm` after another `confirm` succeeded | T6 |
| AG-15 | ok | `RT/tests/accept.rs` `counterparty_holds_its_own_seat_before_signing` (`:314`), `taken_seat_answers_reserve_failed_and_signs_nothing` (`:345`) | — |
| AG-16 | ok, narrowed | `ST/roym_reservation_e2e.rs` `the_writer_marks_a_failed_slot_and_the_giver_releases_it` (`:258`); `RT/tests/commitment_failure.rs` `a_refused_confirm_signs_one_failure_and_sends_it` (`:98`); `RT/tests/commitment_failure_writer.rs` (`:134`); `PAR/commitment_failure.rs` scenario 251. The Deviation "AG-16: the test is in three parts" already records the missing clock (`change.md:1471`) | — |
| AG-36 | ok | `RC/deal/transition/tests.rs` `a_failed_commitment_cancels_the_open_ones` (`:395`); `RC/progress/tests.rs` `failure_cascades_to_unsettled_commitments` (`:227`); `RT/tests/commitment_failure_writer.rs` `a_commitment_with_a_half_keeps_its_path` (`:161`), `each_giver_releases_the_reservations_of_ended_commitments` (`:289`) | — |
| AG-17 | ok | `RC/deal/sd/tests.rs` `no_signed_payload_holds_a_hidden_value` (`:76`) | — |
| AG-18 | ok | `RC/presentation/tests.rs` `agreement_with_price_hidden_and_scope_shown_verifies` (`:210`); `ST/roym_disclosure_e2e.rs` `a_third_node_verifies_an_agreement_with_the_price_hidden` (`:121`) | — |
| AG-34 | ok | `RC/presentation/tests.rs` `quote_alone_with_price_hidden_verifies` (`:246`), `quote_and_half_with_different_terms_are_refused` (`:274`); `ST/roym_disclosure_e2e.rs` `a_third_node_verifies_a_quote_before_any_agreement` (`:184`) | — |
| AG-38 | ok | `RC/presentation/tests.rs` `snapshot_bound_to_the_quote_authors_half_is_the_writers` (`:311`), `snapshot_from_another_key_is_writer_not_checked` (`:415`) | — |
| AG-19 | ok | `crates/signed_record/src/disclosure/tests.rs` `rfc9901_digest_vector` (`:7`), `rfc9901_disclosure_with_spaces_decodes` (`:13`), `our_encoder_output_is_pinned` (`:23`); `RC/disclosure/tests.rs` `a_disclosure_whose_digest_is_not_in_the_record_is_refused` (`:7`) | — |
| AG-20 | gap | `RC/deal/sd/tests.rs` `both_halves_copy_the_quote_terms_byte_for_byte` (`:261`); `RT/backup/tests/disclosures.rs` `import_refuses_agreement_terms_that_differ_from_the_quote` (`:361`). The refusal when a half is filed by sync ("terms differ from the quote", `RT/sync.rs:523`) has no test | T7 |
| AG-21 | ok | `RC/disclosure/tests.rs` `a_salt_of_another_length_is_refused` (`:79`), `a_salt_used_twice_in_one_record_is_refused` (`:93`); `RT/tests/disclosure/cards.rs` `each_quote_version_gets_fresh_salts` (`:191`) | — |
| AG-40 | ok | `RC/deal/sd/tests.rs` `details_over_8_kib_is_refused` (`:275`), `disclosures_over_96_kib_are_refused` (`:300`); `RT/tests/disclosure/cards.rs` `quote_set_refuses_quote_too_large` (`:242`); `UI/screens/quote_size.test.ts`; the size line checked by `sendQuote` in `ST/e2e/hub-helpers.ts` (`:402`), used by stories 40 and 50 | — |
| AG-22 | ok | `RC/deal/tests.rs` `a_quote_with_and_without_money_validate_the_same_way` (`:353`); `ST/roym_deal_e2e.rs` `service_for_service_and_goods_deals_close` (`:247`) | — |
| AG-23, AG-33, AG-27 to AG-30, AG-46, AG-47, AG-48, AG-52 | ok | Already named; every name checked on `main` | — |
| AG-44 | names, gap | `RC/as_listed/tests.rs` `each_mapping_row_builds_its_field` (`:109`), `each_refusal_has_its_code` (`:223`); `RT/tests/as_listed_terms.rs` `a_changed_field_anywhere_in_the_terms_fails_the_check` (`:358`); `RT/tests/as_listed_accept.rs` `a_quote_with_the_listed_terms_is_accepted_with_no_person` (`:188`), `a_quote_that_differs_in_any_field_is_shown` (`:218`); `UI/screens/as_listed_tap.test.ts`; `PAR/as_listed.rs` scenarios 253, 254; story 50. No test of "a missing disclosure shows a normal quote" | T8 |
| AG-45 | names, gap | `RC/listing/as_listed/tests.rs` `instant_needs_an_offering_open_to_anyone` (`:78`), `instant_needs_a_slot_offering_or_not_limited` (`:98`), `instant_is_allowed_on_a_slot_offering_that_is_not_limited` (`:111`); `RT/tests/as_listed_owner.rs` `the_cap_makes_an_instant_offering_wait` (`:246`), `a_failing_certificate_makes_the_offering_wait` (`:312`), `many_taps_before_any_acceptance_make_only_cap_quotes` (`:384`), `an_agreement_with_a_filed_half_no_longer_counts` (`:409`), `a_declined_instant_quote_frees_its_place` (`:479`), `one_counterparty_tapping_twice_gets_one_instant_quote` (`:501`). No test of: the cancel action, a full slot on the instant path, the Hub's `instant` gate on signing, the owner's reason line, the counterparty's waiting line | T9, T10 |
| AG-49 | names, gap | `RT/tests/as_listed_owner.rs` `a_withdrawn_offering_is_declined_with_offering_withdrawn` (`:182`), `a_missing_offering_is_declined_with_offering_withdrawn` (`:207`), `an_expired_consent_is_declined_with_consent_expired` (`:222`), `a_resumed_answer_whose_consent_passed_ends_without_a_card` (`:563`); `RT/tests/as_listed_accept.rs` `a_quote_issued_after_the_consent_time_is_shown` (`:273`). No test of "answers from the current version", or of a held decline or withdrawal before signing | T11 |
| AG-50 | names, gap | `ST/roym_driver_e2e.rs` `two_nodes_reach_agreement_through_driver_without_manual_sync` (`:135`), `stopped_quote_author_restarts_and_countersigns` (`:195`); `RT/tests/drive.rs` (15 tests, e.g. `drive_syncs_at_most_eight_conversations_and_keeps_the_rest`, `:110`); `crates/substrate/src/runtime/wake/tests.rs`. No test of "no call from `conversation` to `transaction`" | T12 |
| AG-51 | names | `RC/progress/tests.rs` `formed_as_is_kept_by_every_apply` (`:359`); `RC/presentation/tests.rs` `a_presentation_with_only_the_latest_snapshot_shows_formed_as` (`:364`); `RT/tests/accept.rs` `a_snapshot_whose_formed_as_changed_is_refused` (`:672`); `RT/tests/as_listed_owner.rs` (`:124`, `:147`) | — |
| AG-31 | ok | `crates/roym_web/tests/method_contract/contract.rs` `every_method_answers_off_the_node_as_its_wire_rule_says` (`:192`, over `transaction_api::method_table()`, both builds); `PAR/transaction_validation.rs` scenario 144. The kind is a contract test, not a service test: the Test column says so | — |
| AG-32 | ok | `RC/transaction/tests.rs` `halves_differ_only_in_role_and_what_was_read` (`:406`); `RC/transaction/tests/viewed.rs` (`:20`, `:53`) | — |
| AG-39 | ok | `RC/deal/tests.rs` `every_text_shares_one_language_and_one_translation_set` (`:324`). `quote.set` and the verifier both call `DealTerms::validate`; step 1 confirms this with `git grep` and the Test column says so | — |
| AG-24 to AG-26 | later | Built in M08E and M08H (task.md section 5). The Test column says "Built in M08E" or "M08H" | — |

### 3.2 Extensions (`EX-*`), owned by the extensions doc

Every test named in a filled `EX-*` row exists on `main`. Eight built rows
have an empty Test column.

| Row | Result | Tests on `main` to write |
| --- | --- | --- |
| EX-4 | gap | `RT/tests/extension.rs` `scheduling_and_one_shot_pass_the_same_states` (`:281`) compares only the end states, from an agreed deal. C8 adds T13 |
| EX-5 | ok, narrowed | `RT/tests/hooks.rs` `a_refused_second_reserve_releases_the_first` (`:89`), `a_commitment_with_no_reserve_hook_calls_no_hook` (`:136`), `a_fault_on_the_second_reserve_releases_the_first` (`:313`); `RT/tests/accept.rs` `taken_seat_answers_reserve_failed_and_signs_nothing` (`:345`); `RT/tests/decide.rs` `losing_the_last_seat_ends_not_formed` (`:286`). Narrowed: Deviation "EX-5: each giver reserves on its own node" |
| EX-6 | ok | `RT/tests/accept.rs` `counterparty_holds_its_own_seat_before_signing` (`:314`); `RS/tests/reserve.rs` `the_last_seat_goes_to_one_of_two_reserves` (`:7`), `a_stale_seat_read_cannot_take_a_held_seat` (`:34`); `ST/roym_booking_e2e.rs` (`:198`) |
| EX-14 | ok, indirect | `RC/services/tests.rs` `every_service_has_a_table_in_the_roym_manifest` (`:57`), `every_hook_service_is_a_dependency_of_transaction` (`:229`); `ST/roym_app_e2e.rs` `test_roym_app_e2e_lifecycle` (`:310`, deploys `roym.toml`, which holds `[services.scheduling]`). The Test column says that the service ships as one more table of `roym.toml` |
| EX-17 | ok | `ST/roym_booking_e2e.rs` (`:198`); `PAR/catalog.rs` scenario 48; `PAR/booking.rs` (`:83`, `:138`, `:358`); `RT/tests/hooks.rs` `the_hooks_go_to_the_service_the_manifest_names` (`:146`). "No slot code in `catalog` and `transaction`": step 1 runs the C3b exit grep again and writes its result in the Deviation |
| EX-20 | ok | `RS/tests/lapse.rs` `a_reservation_carries_its_expiry` (`:27`), `an_unconfirmed_reservation_lapses_when_its_seat_is_needed` (`:36`) |
| EX-21 | ok, narrowed | `RS/tests/reserve.rs` `two_reserves_of_one_key_return_one_reservation` (`:18`), `a_repeat_reserve_returns_the_same_reservation` (`:58`); `RS/tests/release.rs` `release_frees_the_seat_and_a_second_release_changes_nothing` (`:7`), `release_of_an_unknown_key_succeeds` (`:21`), `release_frees_a_confirmed_reservation` (`:28`); `PAR/scheduling.rs` scenario 247. Narrowed: "EX-21: a release also frees a confirmed reservation" |
| EX-22 | ok, narrowed | `RS/tests/confirm.rs` `confirm_after_a_release_is_reservation_lost_and_takes_no_seat` (`:28`); `RS/tests/lapse.rs` `confirm_after_the_expiry_succeeds_while_nobody_took_the_seat` (`:84`), `confirm_after_a_takeover_is_reservation_lost` (`:96`); `RT/tests/commitment_failure.rs` (`:98`); `ST/roym_reservation_e2e.rs` (`:258`); `UI/cards/templates/templates.test.ts` "renderCommitmentFailure" (`:599`). Narrowed: "EX-22 is narrower than the text" |
| EX-9, EX-15, EX-16 | later | M08G; moved out (Deviations). The Test column says so |
| EX-1 to EX-3, EX-7, EX-8, EX-10 to EX-13, EX-18, EX-19 | ok | Already named |

### 3.3 Forwarding (`FW-*`), owned by the forwarding doc

C7b fills every `FW-*` row and sets the doc to `done`. On `main`, FW-1 to
FW-6 and FW-8 to FW-15 have their tests (`RC/chat/tests.rs`,
`PAR/forward.rs` scenarios 278 to 288, `ST/roym_forward_e2e.rs`,
`UI/chat/*.test.ts`, story 20). FW-7's tests are only in C7b
(`RC/record_forward/tests.rs`, `PAR/forward_record.rs` scenarios 289 to
292, `UI/chat/record_forward.test.ts`, Playwright case 44, story 50).
C8 checks, after PR #400 merges, that every name in the FW table exists on
`main`. C8 adds no `FW-*` text, except the FW-7 pointer of D-C8-4.

### 3.4 Overview (`VX-*`), owned by the overview

The overview stays `draft` (M08E to M08H build more of it). For the 15
rows M08C builds (VX-1, VX-2, VX-5 to VX-12, VX-45, VX-46, VX-54 to
VX-56), the Test column of seven is filled ("Through AG-n …"). The other
eight are empty: VX-1, VX-5, VX-6, VX-8, VX-9, VX-10, VX-11, VX-12. C8
writes "Through AG-n." in each, with the AG rows that cite it (section 5,
D-C8-3). It does not copy test names into the overview: two copies would
go stale apart.

## 4. Behaviour-change audit

### Wire changes (between nodes)

None.

### Local method changes (one node)

None. No method, input, output, error code or stored row changes.

### Visible behaviour changes (Hub)

None.

### What changes

- New tests only (section 7.1), in existing test files or new sibling
  test files. No production `.rs`, `.ts`, `.wit`, `.toml` or `.ftl` file
  changes. Step 8 checks this with
  `git diff --name-only origin/main... -- ':!*.md' ':!**/tests/**' ':!**/tests.rs' ':!*.test.ts'`,
  which must print nothing.
- Docs: the three change docs, the overview, the multilingual change doc
  (one Deviation bullet), the living docs, the backlog, task.md,
  status.md, `M08-overview.md` and `roadmap.md` (Q1 (a)).

### Stays the same

- Every component size: no product code changes, so task.md section 1
  gets no new size table.
- No Hindi message is added or changed.
- No demo story is added or changed (D-C8-8).

## 5. Decisions this plan makes

### D-C8-1. The form of a Test column

Each built row's Test column lists named tests, grouped by kind, in this
form: `Unit: <path> (<fn>, <fn>); Service: <path> (<fn>); Parity:
scenarios <n>, <n>; Two-node: <path> (<fn>); Hub: <path>; E2E: <spec>
(case <n>); Demo: <story>.` Paths start at the repo root (as the filled
rows already do). No line numbers: they move. A row that a Deviation
narrowed ends with "Narrowed: see Deviations, \"<bullet title>\"." (Q3
(a)). A row built later says "Built in M08E." (or the milestone).

### D-C8-2. Where each new fact goes

Each criterion, decision and deviation goes to the change doc that owns
it, and only there:

- AG rows, the AG-9 code, the card count sentence (`:1026`) and the
  Differences table: the agreements doc.
- EX rows, the reserve design sentence (`:167`) and the card sentence
  (`:149`): the extensions doc.
- FW-7 pointer: the forwarding doc.
- VX pointers and the five overview rows of the Differences table: the
  overview.
- `LocalText`: the multilingual doc (D-C8-6).

### D-C8-3. VX rows point at AG rows

An empty VX Test column gets "Through AG-n, AG-m." with the AG rows whose
text cites that VX (agreements doc, line 98: "Each row refines an overview
criterion"). VX-1 → AG-1. VX-5 → AG-52. VX-6 → AG-3. VX-8 → AG-6, AG-7.
VX-9 → AG-11, AG-12. VX-10 → AG-14, AG-15. VX-11 → AG-17, AG-18, AG-34.
VX-12 → AG-7, AG-8. The seven filled rows stay as they are.

### D-C8-4. The Differences table, row by row

Each row is applied to its target. Then the row's last column gets
"Done (C8)." only if the target now says the result. The rows:

| Row | Target | What C8 writes |
| --- | --- | --- |
| VX-45, one log | Overview, VX-45 (`:205`), Terms "Event log" (`:80`), "Event log and statement" (`:613`) | Agreement events are in `transaction`'s log; EEP events will be in `obligation`'s own log; the statement joins both. The VX-45 criterion text stays (Q3 (a)); a Deviation bullet says this |
| Selective disclosure | Overview `:351`, `:360` | A fixed set of hidden fields per object; ids, parties, kinds and structure stay visible; the author cannot change the set. Name the type `AgreementReceiptPayload` with no line number (the table's `transaction.rs:234` is also stale: it is `:175` on `main`) |
| Agreement life | Overview `:338` | `enquiry → quoted → accepted → agreed → closed`, with the ends `declined`, `expired`, `not-formed`, `cancelled`; every end ends the quote id |
| Declared value | Overview Terms `:79` | The quote author writes every value, and the other party agrees by signing |
| Barter | Overview Terms `:75` and every use of the word (`grep -n -i barter`: lines 51, 75, 104, 273, 275, 346, 615, 647, 703, 730) | Remove the term. Use "mixed exchange" where the text means a deal with no currency commitment. Open question `:703` is answered: say so, with a link to the agreements doc's open question 1 |
| EX-5 | Extensions design `:167` | "Each giver's `transaction` calls the `reserve` hook of its own extension service, on its own node, when it signs. The writer reserves inside its decision." The criterion row: Q3 (a) |
| Card table | Extensions `:149`; agreements `:1026` and the row `:1072` itself | "These travel as one new card type, `step`. Widgets and widget answers are chat messages with their own content types. The card table grows from seven types to ten (with `deal-decline` and `commitment-failure`)." Fix the Differences row to say 10 |
| EX-21 | Extensions EX-21 row | Q3 (a): the Deviation exists |
| FW-7 | Forwarding FW-7 row | Q3 (a): the Deviation "Forwarded records (FW-7)" exists (C7b) |
| `LocalText` | Multilingual doc | D-C8-6 |
| M08C task.md | — | Already done (C6's title, no "barter"). Mark it done |
| Line reference `transaction.rs:192` | Overview `:351` | See "Selective disclosure" above |
| Integrated spec C12, C14 | — | Already done (`roym-integrated-experience-spec.md:395`, `:403`). Mark it done |

### D-C8-5. The AG-9 Deviation

One bullet in the agreements doc's Deviations: "**AG-9: a cancel by
anyone but the quote author is refused with `quote-author-only`.** The
row says every other cancel gets `commitment-moved`. That code is for the
quote author's cancel after a commitment moved
(`crates/roym_transaction/src/app/progress_ops.rs`). The test is
`cancel_by_the_counterparty_is_refused` in `cancel.rs`."

### D-C8-6. The multilingual doc

The multilingual doc is `done`. Its text says each record has one
`translations` list (`roym-multilingual/change.md:248`) and names
`QuotePayload.translations: Vec<QuoteTranslation>` and `RequestPayload`
(`:281`, `:580`). Neither type exists now: a deal record has a
`LocalText` per free-text field, with one set of translation languages
per record (agreements doc, AG-39). C8 adds one bullet at the end of its
`## Deviations` (`:552`): "**Deal records use a `LocalText` per field
(M08C).** Each free-text field of an enquiry, quote and settlement record
is a `LocalText` with its own translations. All `LocalText` fields of one
record have the same set of translation languages (`translation-set-mismatch`
otherwise). This replaces the one `translations` list for these records;
`QuoteTranslation` and `RequestPayload` no longer exist. Listings and
profiles keep their `translations` list." Step 1 checks the last sentence
in `RC/listing.rs` and `RC/person.rs` before it is written. It does not
rewrite the old bullets: they were true when written.

### D-C8-7. Living docs: fix what M08C made false, nothing more

C8 changes the living-doc statements in section 9 and no others. It adds
no new sections. The test for each edit: the statement is true on `main`
after C7b, and it names no slice or change doc.

### D-C8-8. No new demo story

C8 adds nothing a viewer can see. Its change docs' `demo:` lists already
name stories 20, 50 and 60. C8 checks that each listed story shows its
change: story 50 (deal, statement, quote forward), story 60 (step), story
20 (poll, forward). The demo close-out of C8 itself is
`- none: close-out adds tests and docs only` in the status.md row; C8 has
no change doc of its own.

### D-C8-9. New tests reuse the existing test hosts

Every new service test uses the helpers of the file it joins (for
example `setup_agreed_deal` in `RT/tests/settle.rs:217`, the refusal
helpers in `RT/tests/refuse/helpers.rs`, the reserve-hook stub of
`RT/tests/hooks.rs`). The two-node test extends `roym_deal_e2e.rs` and its
`SubstrateTestContext` helpers. No new test host and no new harness.
Test names say what is checked, with no planning ids.

## 6. Step-by-step changes

Each step leaves the tree building and its tests passing.

### Step 0 — Start after C7b merges

1. Check that PR #400 is merged: `git log --oneline origin/main | grep
   'C7b\|#400'`. If not, stop.
2. `git switch main && git pull && git switch -c feat/m08c-c8-closeout`
   (Q7).
3. Read the four change docs again on the merged `main`. C7b changed the
   forwarding doc, the backlog and some living docs; the line numbers in
   this plan for those files may move. Re-read every `file:line` of this
   plan in a file that changed since `79344661`
   (`git diff --stat 79344661 origin/main -- docs crates`).

### Step 1 — The criterion tables (docs only)

1. Agreements doc: write the Test column of every row in section 3.1 in
   the form of D-C8-1. Replace the wrong names of AG-44, AG-45, AG-49,
   AG-50, AG-51. Leave the gap rows' new test names for step 2 to 7 (each
   step writes its own).
2. Check every name you wrote: for each `fn` name,
   `git grep -n 'fn <name>' -- crates` must print one line; for a parity
   scenario, `git grep -n 'scenario_<n>_' -- crates/roym_web/tests`; for a
   Playwright case, `git grep -n '"<n>\.' -- crates/substrate/tests/e2e`.
   Write a small loop for this and keep its output for the review file.
3. Extensions doc: the eight empty rows of section 3.2; the EX-17 grep
   (the C3b exit criterion: no `availability`, `slot` or `seat` code in
   `crates/roym_catalog/src` and `crates/roym_transaction/src`) run again,
   with its result written in the EX-17 cell.
4. Overview: the eight VX pointers (D-C8-3).
5. Confirm the AG-39 claim: `quote.set` and the quote verifier call
   `DealTerms::validate`. If one does not, AG-39 becomes a gap: write a
   test in the file of that path, like T3.

### Step 2 — Unit tests in `roym_core` (T2, T12)

See section 7.1. `cargo nextest run -p syneroym-roym-core presentation
services`.

### Step 3 — Service tests in `transaction`: refusals (T3, T5, T7)

`cargo nextest run -p syneroym-roym-transaction cancel refuse`.

### Step 4 — Service tests in `transaction`: reserve and extension (T6, T13)

`cargo nextest run -p syneroym-roym-transaction recover extension`.

### Step 5 — Service tests in `transaction`: agree as listed (T8, T9, T11)

`cargo nextest run -p syneroym-roym-transaction as_listed`.

### Step 6 — Hub tests (T10)

`cd crates/roym_web/ui && TZ=UTC npx vitest run src/screens/listings.test.ts`
(then return the shell to the repo root).

### Step 7 — Two-node tests (T1, T4)

`cargo nextest run -p syneroym-substrate --test roym_deal_e2e`.

After steps 2 to 7: each new test is named in its criterion row. If a
test fails because the code does not do what the row says, stop on that
row (Q2 (a)): keep the test with `#[ignore = "<the behaviour that is
missing>"]` out of the commit, add a backlog row, and ask the owner.

### Step 8 — Change docs: Deviations, Differences, Tasks, status

1. Agreements doc: the AG-9 bullet (D-C8-5); the card count (`:1026`);
   the Differences table (D-C8-4); the Tasks list ticks (section 2.2,
   point 8: C1, C2, C3 and the owner review ticked; the C0 ADRs line says
   "ADR-0030 written; the deal-grants ADR moved out with C5"); open
   question 2 says "Decided by ADR-0030"; the stale "C7b follows" style
   lines, if any, made past tense; `status: done`; `living-docs-touched`
   complete (add every doc of section 9 not yet listed); `demo:` checked.
2. Extensions doc: the design sentences of D-C8-4; the Tasks list: "Write
   the M08C slice plans" ticked; "ADR: deal grants" and "Extension
   services: event subscription, deal grants" marked "Moved out of M08C
   (Deviations, EX-15 and EX-16)", not ticked; `status: done`;
   `living-docs-touched` complete.
3. Overview: D-C8-4 rows; Tasks: tick the M08C line and remove "Barter."
   from it; a Deviation bullet for the VX-45 split; `status` stays
   `draft`.
4. Multilingual doc: D-C8-6.
5. `cargo xtask check-change-docs`.

### Step 9 — Living docs

Section 9. Then `grep -n -i -E 'barter|BookingMode|booking mode|request\.set|request\.\*'`
over the seven living docs must find only text that is still true (for
example "request" as a plain English word in a user story).

### Step 10 — Backlog, task.md, status.md, overview, roadmap

1. Backlog: section 8.
2. task.md: line 5 status line ("Status: done, 2026-…"); a "C8 result"
   line in section 3; section 7's table gets a last column "Result" with
   each row's outcome (section 8 below); section 5 unchanged (it says what
   later milestones finish).
3. status.md: the rows of section 2.2, point 2; the C8 row (with the Q5
   Hindi table, if (a)).
4. `M08-overview.md`: row 3 (`:29`) loses "barter" ("mixed exchanges");
   the "Progress" paragraph (`:37`) says M08C is done, with the date, and
   what moved to later milestones (task.md section 5). Q1 (a) only.
5. `roadmap.md:65`: the M08C row says "done", with the date. Q1 (a) only.

### Step 11 — Checks

1. Import cleanup over the new test code (AGENTS.md).
2. `cargo +nightly-2026-04-06 fmt --all`.
3. `cargo dupes report` filtered to the changed test files (AGENTS.md
   recipe). A new test that copies a helper uses the helper instead.
4. `mise run verify`, no `--skip`, in the background, sandbox off (the
   memory note: `cargo test --workspace`, `cargo audit` and e2e need it).
   Record the result in the C8 row.

## 7. Tests

### 7.1 New tests

Each test asserts one rule of its row. If Q2 is (b), this section is
replaced by Deviation bullets and backlog rows.

| Id | Row | File (lines on `main`) | Test name | What it does and checks |
| --- | --- | --- | --- | --- |
| T1 | AG-1 | `ST/roym_deal_e2e.rs` (605) | `each_party_quotes_the_other_and_both_deals_agree` | Alice quotes Bob and Bob quotes Alice, in one conversation, as in `each_party_quotes_the_other_at_once`. Bob accepts Alice's quote, Alice accepts Bob's. Each writer countersigns in `transaction.sync`. Both agreements reach `agreed` on both nodes. Alice is the writer of one and the counterparty of the other |
| T2 | AG-8 | new `RC/presentation/tests/outcomes.rs` (the folder exists: `extension.rs`, `records.rs`, `settlement.rs`, `validation.rs`) | `outcomes_show_with_no_disclosures` | One presentation with the receipt halves and a snapshot only, no disclosures. Four commitments: `ended-unconfirmed` with `claimed: true`, with `claimed: false`, `failed` with a reason, `cancelled` with `other-commitment-failed`. The verified view shows each state, `claimed` and each reason code |
| T3 | AG-9 | `RT/tests/cancel.rs` (319) | `cancel_by_the_counterparty_is_refused` | On the counterparty's node, `agreement.cancel` of an open agreement answers `quote-author-only`. The agreement row does not change |
| T4 | AG-12 | `ST/roym_deal_e2e.rs` | extend `refused_superseded_acceptance_shows_on_that_version` | After the refusal shows, assert the deal is still `quoted` on both nodes; then accept the latest version, and assert `agreed`. If the test already does the second part, add only the first assert |
| T5 | AG-35 | `RT/tests/refuse/version.rs` (284) | `revision_after_not_formed_cancelled_or_closed_is_refused`; `two_quote_ids_for_one_enquiry_both_form` | First: for each end (`not-formed`, `cancelled`, `closed`), `quote.set` of a new version of that quote id answers `quote-ended`. Second: two quotes that answer one enquiry are both accepted and both reach `agreed` |
| T6 | AG-14 | `RT/tests/recover.rs` (626) | `a_refused_second_confirm_releases_the_confirmed_first` | The writer gives two commitments with a reserve hook. The hook stub confirms the first and refuses the second. The deal ends `not-formed` with the extension's reason, and the stub saw a `release` for both, the confirmed one too |
| T7 | AG-20 | new `RT/tests/refuse/terms.rs` (with the helpers of `refuse/helpers.rs`) | `a_half_whose_terms_differ_is_not_filed` | The counterparty's node files a countersigned half whose `terms` differ from the held quote by one field. `transaction.sync` does not file it, and the deal stays as it was |
| T8 | AG-44 | `RT/tests/as_listed_accept.rs` (454) | `a_quote_with_a_missing_disclosure_is_shown` | An as-listed quote whose card leaves out one disclosure is not accepted with no person; it shows as a normal quote |
| T9 | AG-45 | `RT/tests/as_listed_owner.rs` (639); if it would pass 800 lines, a new sibling `as_listed_cancel.rs` | `cancel_of_unconfirmed_instant_agreements_frees_their_places`; `a_full_slot_on_the_instant_path_ends_not_formed_with_slot_taken` | First: the one-action cancel (the method `listings.ts:204` calls; step 5 finds its name in `deal.as-listed-status` or `agreement.cancel`) cancels each unconfirmed instant agreement, and `deal.as-listed-status` shows the places free. Second: an instant quote for a slot whose last seat is gone ends `not-formed`, `slot-taken`, with no confirm-first fallback |
| T10 | AG-45 | `UI/screens/listings.test.ts` (211) | three cases | (1) The listing form offers `instant` only when `transaction.signing-status` says installed. (2) The owner's status line shows the reason from `deal.as-listed-status`. (3) The cancel action calls the node and redraws. The counterparty's waiting line: step 6 finds where the Hub draws it (search `messages-waiting-for-other-side`); if it has a test, name it; if not, add one case there |
| T11 | AG-49 | `RT/tests/as_listed_owner.rs`; `RT/tests/as_listed_accept.rs` | `a_changed_offering_answers_from_the_current_version`; `a_declined_enquiry_is_never_accepted_with_no_person`; `a_withdrawn_enquiry_is_never_accepted_with_no_person` | First: the owner changes the listing after the tap; the quote is built from the current version, so the counterparty shows it as a normal quote. Second and third: the counterparty's node holds a decline (or its own withdrawal) of the enquiry; an as-listed quote then is not signed with no person |
| T12 | AG-50 | `RC/services/tests.rs` (250) | `conversation_does_not_depend_on_transaction` | Reads `app/roym.toml` as the other tests there do. The `conversation` table's `depends_on` does not name `transaction`. A sibling call needs a declared dependency, so this is the proof that the driver adds no call from `conversation` to `transaction` |
| T13 | EX-4 | `RT/tests/extension.rs` (300) | change `scheduling_and_one_shot_pass_the_same_states` | Record the `(CommitmentState, AgreementState)` pair after each step for each deal, and compare the two lists, not only the last pair. Rename only if the name no longer says what it checks |

No new parity scenario: every gap is about one node or about two real
nodes. No new Playwright case.

### 7.2 Runs

- Targeted runs in steps 2 to 7.
- `mise run verify` with no `--skip` (step 11). It runs the whole
  parity binary, so run `mise run build:roym` first.

### 7.3 Size risks

- `RT/tests/as_listed_owner.rs` is 639 lines; T9 and T11 add about 120.
  If the file would pass 800, put the new tests in a new sibling file in
  the same `tests/` folder.
- `RT/tests/accept.rs` (732) and `settle.rs` (739) get nothing.
- `ST/roym_deal_e2e.rs` 605 + T1 and T4 (about 80) stays under 800.
- The agreements doc is 1,541 lines; the file-length check covers source
  files, not docs.
- No production file changes, so no component size changes.

### 7.4 Time risks

- T1 is one more two-node test in `roym_deal_e2e` (about the time of
  `service_for_service_and_goods_deals_close`). It runs in the
  `substrate-e2e` nextest group.
- Reading all 30,000 lines of plans and reviews is not needed again: this
  plan holds what C8 needs. Read the change docs and the files you edit.

### 7.5 Not tested

- AG-11's higher-`seq` rule has no `transaction` service test; parity
  scenario 157 checks it on both builds. Enough.
- AG-16 has no clock: the Deviation records it.
- AG-37 and AG-39 test the shared validator, which `quote.set` and the
  verifier both call (step 1.5 checks).
- EX-14 is shown indirectly (section 3.2).
- AG-30's `available` and `unavailable` forms come in M08E.

### 7.6 How C8 avoids the kinds of findings in earlier reviews

- A test name in a doc that does not exist (C4 R2, C1 R5): step 1.2
  greps every name.
- A Deviation in the wrong doc, or a summary in place of a deviation (C6
  R3, R23; C3c R23): D-C8-2 names the owner of each fact.
- A living-doc statement still false after the slice (C2 R7, C3c R23, C7
  R9): section 9 lists each statement with its line, and step 9 greps.
- status.md counts with no command, or over-claims (C3c R12, C6 R9, C4
  R20): the C8 row names each command and its summary line, and the CI
  run.
- Backlog rows that are stale or break the table (C1 R15, C3c R30):
  section 8 names each row by title; no empty line inside a table.
- Rewriting reviewer text in a review file (C1, C2, C3c, C7): C8 changes
  no review file. Its own review file is written by its reviewer.
- A Hindi review not recorded (C1 R42(a), C4 R30): Q5.
- Test names that claim more than they check (C3b R15, C7 R19): each
  name in 7.1 says only what its asserts check.
- A fix that overreaches (C1 P24): C8 does not write into owner answers
  in any plan or change doc.

## 8. Deferred backlog

Rows of [deferred-backlog.md](../../deferred-backlog.md) on `main`, by
title. C7b may add rows; step 0 re-reads the file.

**task.md section 7 (13 rows):** C8 records each result in task.md.

| Row (title) | Line | State on `main` | C8 does |
| --- | --- | --- | --- |
| Payment and fulfilment records name the deal parties | resolved `:763` | Resolved (C1a) | Result "resolved" |
| The receipt card does not show what each party read | resolved `:764` | Resolved (C1a) | Result "resolved" |
| Widget labels and extension step names | resolved `:755` | Resolved (C4b); step names are derived (`developer-guide.md:1868`) | Result "resolved" |
| Translation hook points for widget answers, … | `:586` | Open, widget answers removed, targets M08G and M08D | Result "narrowed as planned" |
| Services can only pull events | `:555` | Open, target with the extension-services row | Result "moved, as planned" |
| Extension services cannot receive events or take calls from a counterparty | `:556` | Open | Result "open, as planned" |
| A widget definition larger than 8 KB is refused | `:557` | Open | Result "open, as planned" |
| Hub transaction action panel … (cases 33-39) | `:670` | Open: booking steps and payment requests | Result "narrowed as planned" |
| More demo stories | `:716` | Open; its text "Only two stories exist" is false: six exist (10, 20, 30, 40, 50, 60) | Rewrite the text to list what is still missing; result "updated" |
| One Roym output carries another node's card data as an untyped `Value` | `:545` | Open, `PeerStep` gone | Result "updated as planned" |
| `RECORD_TYPES` is not read by any verifier | `:672` | Open; the line range in it has moved | Drop the line numbers (name the file and the constant); result "open" |
| A request made from a directory hit does not name the listing | resolved `:761` | Resolved (PR #373) | Result "resolved" |
| Agree as listed refuses some offerings | `:584` | Open, target M08D or later | Result "open, as planned" |

**Rows M08C made false (fix in place):**

- "The two-node as-listed test, parity 255, …" (`:690`): target per Q4.
- "No signed rejection and no quote withdrawal" (`:666`): its premise
  ("the card set at seven types and `RECORD_TYPES` carries no rejection
  type") is false since `deal-decline` (C1b; the row "Decline of a
  deal is local and unsigned" is in "Recently resolved"). Step 10 checks
  what is left: a
  quote author still cannot withdraw a sent quote (only decline an
  enquiry, AG-10). Rewrite the row to that one point, keep its pickup
  trigger, and drop `D-06C-3` from it.
- "Two concurrent `request.set` / `quote.set` calls …" (`:663`): the
  method is `enquiry.set` and the file `enquiry_ops.rs`. Rename only.

**New rows:** one for each Q2 stop (a test that found the code does not
do what its row says), if any. None expected.

**Open in-code markers (section 14):** no row names `roym_transaction`,
`roym_scheduling` or the `roym_core` deal, extension or widget code, and
`git grep -n -E 'TODO|FIXME'` there finds none. Nothing to add or remove.

## 9. Living docs and the change docs at close-out

The change docs first (step 8, section 5). Then these living-doc
statements, each read on `main` at `79344661`. Step 0 re-reads them after
C7b.

`docs/roym-integrated-experience-spec.md`:

- `:98` "forwards JSON-RPC to the four services below" → the six
  services; `:155` "the other four services" → six; `:690` "Roym's five
  services" → the right count (read the sentence: it may mean the
  services other than `web`).
- `:103` the Transaction row: "Requests" → "Enquiries"; API `request.*`
  → `enquiry.*`; add `payment.*`, `fulfilment.*`, `step.*`; `receipt.*`
  is only `receipt.ping`, so drop it (the list in
  `crates/roym_transaction/src/api.rs`).
- Records table (`:477` to `:491`): add `deal-decline` and `step`, both
  in `RECORD_TYPES` (`crates/roym_core/src/record.rs`). The paragraph at
  `:508` to `:512` ("Two types in the card table … are not in the list
  above") → name the card types that are not in the records list, from
  the code.
- State diagram (`:541` to `:546`): add `accepted` and `declined`, as
  `deal_state` reports them (`developer-guide.md:2691`).
- Slot rule (`:554` to `:557`): "the quote author's Transaction service
  decides … claiming the slot" → each giver's node reserves through its
  `scheduling` service; the writer confirms its own; a refused confirm
  ends in a `commitment-failure`.
- The payment rule, from the agreements close-out list: add one sentence
  near the `payment-request` card row (`:789`): one payment request and
  one acknowledgement per `currency` commitment.
- `:862` the record name `request` → `enquiry`.
- `formed_as`: one sentence where the as-listed path is described
  (`:395` to `:405`): the snapshot says how the agreement formed.

`docs/system-architecture.md`:

- `:620` "The data of five Roym services" → six, with `scheduling`
  (`scheduling.export`, `crates/roym_scheduling/src/api.rs:36`).
- `:1248` "The four agreement states" → the deal states of
  `deal_state`, and which are final.
- `:1268` "One slot of the catalog … The quote author's node claims the
  seats" → slots are in `scheduling`; each giver's node reserves.
- The `transaction` service's collections, from the agreements close-out
  list: one sentence that names them (`event_log`, `event_log_state`,
  `deal_defaults`, `ledger` and the others in `developer-guide.md:2737`).
  Step 9 reads the full list from the code before writing.

`docs/developer-guide.md`:

- §6.7 (`:2676` to `:2699`): say that each card has a `disclosures`
  field next to its envelope (`crates/roym_core/src/card.rs:55`).
- The `scheduling` client methods `availability.set`, `.get`, `.list`,
  `.remove`: no living doc names them. Add a short paragraph to the Roym
  client section, next to the extensions part (`:1844` to `:1929`). If
  it holds a tagged code block, story 30 must run it (C7 R8).
- `formed_as` in the snapshot fields, where `agreement.progress` is
  described.

`docs/system-requirements-spec.md` (deal workflow):

- `:695` "Request and clarify. A request …" → enquiry.
- `:697` the old tracks (`Scheduled`, `InProgress`, `Completed`,
  `Cancelled`, `Conflict`, `EndedUnconfirmed`) and
  `BookingProgressPayload` → the commitment states and the progress
  snapshot.
- `:701` `InteractionReceipt` and `FulfilmentRecord` → the agreement
  receipt and the settlement records.
- `:720` to `:722` `Scheduled`, `BookingProgressPayload`, `Conflict`,
  `ConflictReason::SlotTaken`, `AlreadyDecided` → `agreed`, the progress
  snapshot, `not-formed` with `slot-taken`, the repeat rule (AG-13).

`docs/planning/traceability-matrix.md`:

- `[ROY-TYP]` (`:93`) and `[ROY-CHK]` (`:95`) say "all six services" →
  seven (with `scheduling`), and link `crates/roym_scheduling/src/api.rs`.
- `[APP-AGREE]` (`:59`), `[TXN-SLT]` (`:86`), `[VRT-PAY]` (`:47`):
  check that each link and test still exists.

`docs/TERMINOLOGY.md`: already has every term of the agreements close-out
list (`:67` to `:98`). No change.

`docs/demo/demo.md`: `:1292` "send him a request" — check the Hub's own
word (`en.ftl`). Change it to "enquiry" only if the Hub says so.

## 10. Exit criteria

C8 is done when all of these hold:

- [ ] PR #400 (C7b) is merged, and C8 starts from that `main`.
- [ ] Every built `AG-*`, `EX-*` and `FW-*` row names its tests in the
      form of D-C8-1; every name is found by `git grep` (step 1.2 output
      kept); every row built later names its milestone.
- [ ] Every gap of section 3 has its test from section 7.1, and the test
      passes; or (Q2 stop) a backlog row and an owner answer.
- [ ] No production file changed (section 4, the `git diff` check).
- [ ] The eight empty VX rows point at their AG rows.
- [ ] Every Differences-table row is applied and marked done (D-C8-4).
- [ ] `roym-agreements` and `roym-extensions` are `status: done`, with
      `living-docs-touched` and `demo:` complete;
      `roym-message-forwarding` is `done` (C7b); the overview is `draft`
      with its M08C task ticked; the multilingual doc has its bullet.
- [ ] Every living-doc statement of section 9 is true; the step 9 grep
      finds nothing false.
- [ ] The backlog rows of section 8 are updated; task.md section 7 has
      its Result column.
- [ ] status.md: every M08C row is "Merged" with its PR (section 2.2,
      point 2); the C8 row has the evidence; the Hindi table (Q5).
- [ ] task.md, `M08-overview.md` and `roadmap.md` mark M08C done (Q1
      (a)).
- [ ] `cargo xtask check-change-docs` passes.
- [ ] `mise run verify` with no `--skip` passes; the result is in the C8
      row.

## 11. What C8 does not do

- No product code change. A test that shows the code is wrong stops C8
  on that row (Q2 (a)).
- No new parity scenario, Playwright case or demo story.
- No new living-doc section, and no rewrite of a living doc's language
  beyond the statements it fixes.
- No rewrite of an ADR, of a review file, or of an owner's answer in a
  plan or change doc.
- No criterion text rewritten (Q3 (a)).
- No build of the work that task.md section 5 gives to later
  milestones: EEP kinds (M08E, M08H), the Want-Bid test (M08F),
  reputation (M08G), EX-9, EX-15, EX-16, fetch by hash (EX-11).
- No build of the cut as-listed tests (Q4).
- No change to the overview's `status`.

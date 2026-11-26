# M08C slice C6 — Mixed exchanges, event log, statement and settlement: implementation plan

*Reader: the developer (or AI) who implements slice C6, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan reviewed round 1 (Claude Sonnet 5.5, 2026-11-26; folded by Gemini 3.8 Flash, 2026-11-26).
Owner answers recorded 2026-11-26.**

**Name.** The planning prompt called this slice "Barter, event log and
statement". [task.md](task.md) calls it "Mixed exchanges, event log,
statement and settlement", and the agreements doc does not use the word
"barter" (its "Terms" section, owner decision on its open question 1). This
plan uses the task.md name. The rule that no code, message or doc may say
"barter" covers product features, UI copy, API fields, and production code.
It does not forbid arbitrary test fixture values (such as the `barter-tokens`
payment method in `templates.test.ts`), and it does not rename git branch
names.

**Size and PRs.** [task.md](task.md) sizes C6 as L. Checked against the
code, it is about 1,400 lines of Rust, 1,600 lines of TypeScript and as
many lines of tests. The plan recommends two PRs (Q1):

- **C6a — Event log, statement and closing on the node (M).** The pure
  log, statement and close rules in `roym_core`; the `event_log`
  collection and the derive step on every `transaction.sync`;
  `eventlog.list`, `eventlog.rebuild`, `statement.export`,
  `agreement.close`, and the saved deal terms (`deal.defaults-get`,
  `deal.defaults-set`); `roymctl`; the parity scenarios and a two-node
  test. Nothing on the Hub screen is new.
  Branch: `feat/m08e-c6-barter-statement` (this branch; it holds the plan
  commit, cut from `main` at `edd71c11`). The branch name says `m08e`; the
  slice is M08C. The name was given in the prompt and is kept.
- **C6b — The deal screens in the Hub (M).** The quote form with any mix
  of commitments, the totals and the difference, the agreement panel with
  the settlement actions and the two closing actions, the change summary,
  the form defaults, the new words, "Suggest changes", the statement
  screen with its two downloads, the Hindi messages, the Playwright cases
  and the end of story 50. Branch: `feat/m08c-c6b-deal-screens`, cut from
  `main` after C6a merges.

**Base.** Every file path and line number below was checked on `main` at
`edd71c11` (2026-11-26, C4b merged, PR #384). Links are relative to this
folder.

**Dependencies.** [task.md](task.md) section 4: "C6 needs only C2. It
comes last, so that the deal screens are changed once more, at the end,
with every commitment kind and extension in place." The order line puts C6
after C4. [status.md](status.md) says "Merged" for C2a, C2b, C3a1 to
C3c2 and C4b. The C4a row still says "Code reviewed", but its squash commit
`933fb399` (PR #383) is on `main` (point 1 in section 2.2). C6 must not
run beside the Engage fix (done) or beside C7 in the Hub's `messages.ts`
(section 7.8).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments, test names, or `.wit`, `.toml`,
    `.json`, `.ftl` files: no "M08C", "C6a", "AG-27", "VX-46", "D-C6-3",
    "Q2" or review ids. ADR numbers are allowed.
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test
    `.rs` files at most 800 lines. Several files this slice calls are near
    the cap (section 7.8). New code goes into new files.
  - **Function size.** `clippy::too_many_lines` at 100. Add no
    suppression.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - **Ports.** Two-node tests use the shared harness
    (`crates/substrate/tests/common/roym.rs`, `roym_flow.rs`), never a
    port literal.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): "C6", section 1 (the component size limit),
  section 4, section 5 (the AG-30 and AG-48 rows), section 6 question 2
  (settlement actions in the Hub), section 7 (the Hub action panel row).
- [roym-agreements](../../changes/roym-agreements/change.md):
  - the acceptance rows AG-23 (`:148`), AG-27 to AG-30 (`:155` to `:158`),
    AG-33 (`:149`), AG-46 to AG-48 and AG-52 (`:165` to `:168`);
  - "Settlement records" (`:547` to `:586`), above all `settlement_note`
    (`:582`) and "Closing in one action" (`:586`);
  - "Commitments" (`:661` to `:709`): the final states and the track
    window;
  - "Mixed exchanges and declared values" (`:854` to `:871`);
  - "Event log and statement" (`:873` to `:977`);
  - "Methods" (`:979` to `:1012`): `agreement.close`, `eventlog.list`,
    `eventlog.rebuild`, `statement.export`, the `*.ping` row;
  - "Deviations" (`:1219` to `:1507`), above all the C1 notes on AG-23
    and AG-33 (`:1243` to `:1247`).
- [roym-value-exchange](../../changes/roym-value-exchange/change.md):
  VX-2 (`:146`), VX-5 (`:151`), VX-56 (`:154`), VX-7 (`:156`), VX-55
  (`:162`), VX-45 and VX-46 (`:205`, `:206`), "Event log and statement"
  (`:611` to `:617`), "Deviations" (`:742` to `:769`).
- The C4 plan ([slice-c4-implementation-plan.md](slice-c4-implementation-plan.md))
  for the layout of this plan, and the C1 to C4 review files. Section
  7.11 says how this plan avoids the kinds of findings they hold.

### Terms used in this plan

- **Event log:** the `event_log` collection inside `transaction`. One
  row per agreement event this node takes part in. Rows are only created,
  never changed. It is local data, not a signed record. Do not confuse it
  with the host's **event bus** (`syneroym:events`), where `transaction`
  already records `transaction.card` and `transaction.progress` events
  for the wake ([app.rs:448](../../../../crates/roym_transaction/src/app.rs#L448),
  [progress_ops.rs:137](../../../../crates/roym_transaction/src/app/progress_ops.rs#L137)).
  C6 does not change the event bus.
- **Derive step:** the function that builds the log rows that the
  records a node holds imply, and creates the missing ones. Every
  `transaction.sync` runs it. `eventlog.rebuild` runs it over every
  agreement.
- **Settling half:** the receiver's settlement half of one commitment
  (`fulfilment-receipt` or `payment-acknowledgement` with role
  `receiver`). It moves the commitment to its success end.
- **Claim:** the giver's half of one commitment. It moves the commitment
  to `claimed`.
- **Statement:** the output of `statement.export` for a period: every
  commitment settled in the period, the ones still open at the end, and
  totals per currency.
- **Saved deal terms:** the quote author's own defaults for the
  cancellation, refund and dispute texts and the quote's validity. A local
  setting of the author's node, not a record (agreements doc `:867`).
- **Agreement panel:** the new Hub block that shows one agreement: each
  commitment with its value and state, the totals, and the settlement and
  closing actions (Q2).

## 1. What C6 builds, in one paragraph

Today a Roym deal can already hold several commitments on each side, but
the Hub can only build one service and one payment, shows no totals, and
has no button to settle anything. C6 finishes the deal screens. Bob writes
an **offer** (the Hub's new word for a quote) with any mix: a lesson he
gives, a sack of wheat and ₹100 that Alice gives. Most fields are already
filled from his listing and his saved deal terms. Alice sees what she
gives, what she receives and the difference, and taps **Agree**. After the
deal forms, each person sees each commitment with its value and its
state, and taps "I delivered", "I paid" or "Received" on a commitment, or
"I gave everything" / "I received everything" on the whole agreement. Each
node writes an **event log** of its deals from the records it holds, and a
person downloads a **statement** for a period as CSV or JSON. When a quote
has several versions, the counterparty sees one summary of what changed,
and can tap "Suggest changes" to ask for a new version in the chat. No
signed record, card or state changes. Every new method is local-only.

### Why C6 is split in two (Q1)

C6a is node code with pure rules and service tests: the log, the
statement and the close. A reviewer can read it with no Hub, and its
result is visible through `roymctl` and a two-node test. C6b is all Hub
code: a new quote form, a new panel and a new screen, the words and the
demo. The C3c2 and C4b reviews found most of their late findings in the
Hub and the stories; a separate Hub PR keeps those rounds away from the
node code, as C4 did.

## How this is shown to an end user

C6a:

- Nothing in the Hub is new. A developer can close an agreement and
  download a statement with `roymctl`.

C6b:

- Bob opens Alice's enquiry and taps "Make an offer". The form already
  has his listing's price, currency, payee and his usual cancellation,
  refund and dispute texts, in a section "More terms" that is closed. He
  types what the lesson is, adds "Alice gives: a sack of wheat, ₹400" and
  keeps "Alice pays ₹100". He taps "Send offer".
- Alice sees the offer: "You give ₹500. You receive ₹600. You receive
  ₹100 more than you give." She taps "Agree".
- After the deal forms, both see the agreement with three lines, each
  with its value and its state ("promised").
- Bob taps "I delivered" on the lesson. Alice taps "Received" on it, and
  "I gave everything" for her wheat and money; the Hub lists what it will
  sign, and she confirms. Each line moves to its end state, and the
  agreement shows "closed".
- Alice opens "Statement", picks this month, and downloads the CSV and the
  JSON. The CSV has one line per commitment with its value and its
  counterparty.
- If Bob had sent a second version of the offer, Alice would first see
  "What changed since the first offer" for each side, and could tap
  "Suggest changes" to write to Bob about that version.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core eventlog statement` (C6a):
  every log event kind, the two times, the statement period edges and the
  totals.
- `cargo nextest run -p syneroym-substrate --test roym_statement_e2e`
  (C6a): a three-commitment deal is closed by both sides with
  `agreement.close`; both nodes list the same commitments in the same
  period.
- `roymctl roym transaction agreement close --agreement <rec> --as
  receiver --commitment c1`, then `roymctl roym transaction statement
  --from 2026-11-01 --to 2026-12-01` (C6a): the statement as JSON.
- `mise run demo` (C6b): story 50's new third case ends with Alice's
  statement download.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for
the recommended answer.

#### Q1. How many PRs?

- (a) One PR. About 4,500 lines with tests. One close-out.
- (b) Two PRs: C6a (node) and C6b (Hub), as above.

**Recommendation: (b).** The reason is in "Why C6 is split in two". C6a
has a visible result through `roymctl`. C6b relies on a merged and tested
API.

Owner answer: (b), 2026-11-26.

#### Q2. Where are the agreement actions in the Hub?

task.md asks for "the agreement screen" with each commitment's state, the
totals, the settlement actions and the closing actions. The Hub has no
agreement screen today. Each agreement shows as cards in the thread: the
receipt card ([agreement_receipt.ts:62](../../../../crates/roym_web/ui/src/cards/templates/agreement_receipt.ts#L62),
one `amount_minor` total at `:193`) and the progress card, which writes
each commitment as `c1: promised`
([agreement_progress.ts:114](../../../../crates/roym_web/ui/src/cards/templates/agreement_progress.ts#L114)).

- (a) An **agreement panel** in the thread, drawn once per agreement under
  its newest receipt or progress card, from `agreement.get`. It shows each
  commitment (who gives what, value, state), the totals, the
  per-commitment actions and the two closing actions.
- (b) A new "Deals" tab with a list of agreements (`agreement.list`) and a
  detail view.
- (c) Both.

**Recommendation: (a).** A person settles a deal where they talk about
it, as story 50 does. A list of deals across conversations is useful, but
it is new navigation that no criterion asks for. It is a backlog row.

Owner answer: (a), 2026-11-26. It needs P1's panel state.

#### Q3. Where is the statement screen?

- (a) A new tab "Statement".
- (b) A section in the Backup tab.

**Recommendation: (a).** A statement is not a backup: it is for a person
(or an accountant), it has a period, and it never goes back into a node.
Adding a tab is four small edits
([navigation.ts:11](../../../../crates/roym_web/ui/src/navigation.ts#L11),
[main.ts:151](../../../../crates/roym_web/ui/src/main.ts#L151),
[main.ts:180](../../../../crates/roym_web/ui/src/main.ts#L180) and both
`.ftl` files).

Owner answer: (a), 2026-11-26.

#### Q4. Where are the saved deal terms kept?

AG-48 fills the quote form "from … the author's saved deal terms". Nothing
saves them today: the Hub keeps only the language in `localStorage`
([i18n.ts:11](../../../../crates/roym_web/ui/src/i18n/i18n.ts#L11)), and
`profile` holds the name, address, languages and about text. The
agreements doc says they are "local settings of the author's own node, not
a record" (`:867`).

- (a) One row in a new `transaction` collection `deal_defaults`, read and
  written by two new local-only methods `deal.defaults-get` and
  `deal.defaults-set`, and carried in the `transaction` backup bundle
  (`SCHEMA_VERSION` 8 to 9).
- (b) `localStorage` in the Hub. Lost on a new browser or device, and not
  in the backup.
- (c) New fields in `profile`.

**Recommendation: (a).** The terms are deal data and belong to the
service that signs quotes. A node setting survives a new browser, and a
restore. `profile` holds the person's public card data; deal defaults do
not belong next to it.

Owner answer: (a), 2026-11-26. It needs P9's constant and import check.

#### Q5. Is the event log in the backup bundle?

The agreements doc says export and import "carry disclosures and the event
log" (`:1009`), and "every entry can be derived from the records and
snapshots a node holds" (`:918`).

- (a) The bundle does not carry the log. `transaction.import` ends with
  the derive step over every imported agreement.
- (b) The bundle carries the log, and the import checks every row against
  the records in the same bundle.

**Recommendation: (a).** A log row is a pure function of verified
records. An imported row that does not match them would put a false line
in a statement, so (b) would need a check that is the derive step again.
(a) needs no new backup section, keeps `backup.rs` (788 lines) and
`backup/sections.rs` (732 lines) from growing, and gives the same rows.
The one loss: `recorded_at_secs` becomes the import time. It only orders
`eventlog.list`. This is a deviation of the agreements doc (section 9).

Owner answer: (a), 2026-11-26. The claim that it keeps backup.rs small is not right, but the fallback in step 7 is.

#### Q6. Who fills `counterparty_name` in the statement?

The statement row has `counterparty_name` "from the owner's contacts; not
signed" (agreements doc `:935`). Contacts are in `profile`
([api.rs:35](../../../../crates/roym_profile/src/api.rs#L35)).
`transaction` depends on `conversation`, `scheduling` and `catalog` only
([roym.toml:83](../../../../crates/roym_core/app/roym.toml#L83)).

- (a) `statement.export` leaves it absent. The Hub fills it from
  `contacts.list` into both downloads.
- (b) `transaction` gets a `depends_on` edge to `profile` and fills it.

**Recommendation: (a).** No new service edge for a display name. The Hub
already builds a DID-to-name map this way
([groups.ts:434](../../../../crates/roym_web/ui/src/screens/groups.ts#L434)
to `:459`). `roymctl` shows the DID, which is what a developer needs.

Owner answer: (a), 2026-11-26.

#### Q7. What does "Suggest changes" send?

AG-52: it "starts a message in the same conversation that names that
quote version". A Roym text message has no reply or reference field
today.

- (a) An ordinary text message. The Hub fills the compose box with one
  line in the writer's language, "About offer version 2:", and the person
  writes the rest and sends it.
- (b) A new message content type that carries the quote's `record_id`, so
  the reader's Hub can draw the line in the reader's language and link to
  the card.

**Recommendation: (a).** (b) changes `conversation` and the Hub chat,
which C7 also changes (task.md section 4), for a line a person can read.
The version number is the one both Hubs show on the card (D-C6-12).

Owner answer: (a), 2026-11-26.

#### Q8. Is `INR` also the default currency of the listing editor?

AG-48 makes `INR` the quote form's last fallback. The quote form's
default is `EUR` today ([messages.ts:938](../../../../crates/roym_web/ui/src/screens/messages.ts#L938)),
and so is the listing editor's
([listings.ts:438](../../../../crates/roym_web/ui/src/screens/listings.ts#L438)).

- (a) Both default to `INR`.
- (b) Only the quote form.

**Recommendation: (a).** The quote form fills its currency from the
listing first. Two different defaults would make an offer from a new
listing `EUR` and an offer with no listing `INR`.

Owner answer: (a), 2026-11-26.

#### Q9. Which size limit applies to `transaction`?

C6a adds the log rows, the statement, the close and the defaults to
`transaction`. These are new stored shapes and their code, but no new
signed record.

- (a) +15% (the "new record code" limit), as C3c1 used for the driver.
- (b) +40 KB.

**Recommendation: (a).** The estimate is +60 to +120 KB (section 7.9):
the statement and log types with their `serde` code, the derive step, and
four new methods in the dispatch future. +40 KB would force the statement
out of `transaction`, which the agreements doc decided against (`:877`).

Owner answer: (a), 2026-11-26. Agreed, as an explicit owner decision, because C6 adds no signed record but still gets the +15% limit.

#### Q10. Demo

task.md: "C6 finishes [story 50] (totals, settlement, statement)". Story 50
has two cases today: a one-lesson deal with a decline, and agree as listed
([50-roym-deal.spec.ts:19](../../../../crates/substrate/tests/e2e/demo/50-roym-deal.spec.ts#L19),
`:167`).

- (a) A third case: Bob offers a lesson for a sack of wheat plus ₹100,
  Alice sees the totals and agrees, both settle in the Hub, and Alice
  downloads her statement. Cases one and two stay, with the new words.
- (b) Change case one into the mixed deal.

**Recommendation: (a).** Case one also shows the decline with a note,
which story 50 is the only story to show. The demo goal in task.md names
exactly the third case.

Owner answer: (a), 2026-11-26.

### 2.2 Points found while reading

1. **status.md is stale for C4a.** Its row says "Code reviewed", but
   `933fb399` (PR #383) is on `main`. C6a's close-out sets it to
   "Merged", with the commit, unless the owner fixes it first.
2. **No version comparison exists in the Hub.** AG-47 says commitments
   "are matched by their disclosed description, as the version comparison
   does today". Nothing compares two quote versions
   ([thread_enquiry.ts:3](../../../../crates/roym_web/ui/src/screens/thread_enquiry.ts#L3)
   only finds the newest version; `isQuoteSuperseded` at `:41` checks
   the enquiry version). C6b writes the comparison (D-C6-11). The
   agreements doc's sentence gets a deviation.
3. **The quote form builds at most two commitments.**
   [quote_terms.ts:28](../../../../crates/roym_web/ui/src/screens/quote_terms.ts#L28)
   builds `c1` (the author's `goods-or-service`) and, when the amount is
   above 0, `c2` (the counterparty's `currency`, `:62`). The form fields
   are at [messages.ts:925](../../../../crates/roym_web/ui/src/screens/messages.ts#L925)
   to `:1074`; `readQuoteForm` at `:1104`; `quote.set` at `:1041`. The
   form takes nothing from a listing. Backlog row "The Hub's quote form
   builds only one service and one payment" (target C6).
4. **The quote card totals only money.**
   [quote.ts:148](../../../../crates/roym_web/ui/src/cards/templates/quote.ts#L148)
   to `:164` sums the `currency` commitments. It already draws each
   commitment as "you give" / "X gives" (`:200` to `:243`). C6b replaces
   the money total with the two totals and the difference (AG-23).
5. **No settlement action exists in the Hub.** The Hub calls no
   `payment.*` or `fulfilment.*` method. The settlement cards are display
   only. The node methods exist:
   - `fulfilment.sign` ([fulfilment_ops.rs:79](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L79),
     `sign_at` at `:86`; input `{agreement, commitment_id,
     settlement_note}` at `:36`). Either party; the role comes from the
     commitment's giver.
   - `payment.acknowledge` ([payment_ops.rs:257](../../../../crates/roym_transaction/src/app/payment_ops.rs#L257);
     input at `:56` with `observed_at_secs`, `method`, `reference`,
     `settlement_note`, `supersedes`). It reads the clock inside
     (`:261`); it has no `_at` form.
   - Both return the stored half on a repeat (`AlreadyRecorded`,
     [fulfilment_ops.rs:117](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L117),
     [payment_ops.rs:548](../../../../crates/roym_transaction/src/app/payment_ops.rs#L548)).
   - Only the quote author's node moves a commitment state at once
     (`progress_ops::transition_at`,
     [progress_ops.rs:146](../../../../crates/roym_transaction/src/app/progress_ops.rs#L146),
     called from `payment_ops.rs:325` and `fulfilment_ops.rs:264` only
     when `owner == quote_author_did`). The counterparty's node files its
     own signed half locally, but its `AgreementRow` commitment state
     moves only when it files the quote author's newer snapshot
     ([sync/receipts.rs:277](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L277),
     `apply_newer_snapshot`).
6. **Every value the log needs is held on both nodes.**
   - The agreement row has the plain `terms` (declared values,
     descriptions, givers), the state, each commitment's progress,
     `agreed_at_secs` and both receipt halves
     ([app.rs:170](../../../../crates/roym_transaction/src/app.rs#L170)
     to `:196`).
   - Every snapshot a node holds is a row `snapshot:<agreement>:<seq>`
     in `agreement_progress`, with its envelope
     ([decide/rows.rs:32](../../../../crates/roym_transaction/src/app/decide/rows.rs#L32),
     `SnapshotRow` at `:73`). The counterparty stores each newer
     snapshot the same way
     ([sync/receipts.rs:277](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L277)
     to `:292`). A row has `stored_at_secs`, not `issued_at_secs`; the
     envelope has `issued_at_secs`
     ([envelope.rs:131](../../../../crates/signed_record/src/envelope.rs#L131)).
   - Each settlement half is a `ReceiptHalf {envelope, record_id, issuer,
     issued_at_secs}` ([transaction.rs:193](../../../../crates/roym_core/src/transaction.rs#L193))
     in the `payments` and `fulfilments` rows, by deal role
     ([app.rs:262](../../../../crates/roym_transaction/src/app.rs#L262)
     to `:287`). Its `settlement_note` is in the stored disclosures
     (`disclosures::load`,
     [disclosures.rs:63](../../../../crates/roym_transaction/src/app/disclosures.rs#L63);
     `plain_view` at `:150`).
   - A `not-formed` deal has an agreement row with state `NotFormed`
     ([decide.rs:203](../../../../crates/roym_transaction/src/app/decide.rs#L203)).
7. **The sync passes.** `sync_at`
   ([sync.rs:82](../../../../crates/roym_transaction/src/app/sync.rs#L82))
   runs `sync_direct` and then `decide::recover_deciding_rows` (`:90`).
   `sync_direct` files the cards, writes `sync_state`, confirms own
   reservations (`:161`) and ticks the conversation's agreements (`:162`).
   `transaction.drive` runs `sync_direct` for up to 8 conversations
   ([drive.rs:232](../../../../crates/roym_transaction/src/app/drive.rs#L232)).
   So a derive step at the end of `sync_direct` runs on every Hub redraw
   and every wake.
8. **The data layer has no ordered query.** A query takes a JSON filter
   with `$gt`, `$gte`, `$lt`, `$lte`, a limit and a cursor
   ([data-layer.wit:39](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L39);
   `$lte` in [listing_ops.rs:225](../../../../crates/roym_catalog/src/app/listing_ops.rs#L225)).
   It returns rows in no stated order. The test host's filter knows only
   `$gt` and `$lt`
   ([testing/data_layer.rs:224](../../../../crates/roym_core/src/testing/data_layer.rs#L224)
   to `:251`). So `eventlog.list` filters by `recorded_at_secs` and sorts
   in the service (D-C6-5), and C6a adds `$gte` to the test host.
9. **The test host records queries, not reads by id.** `TestHost::queries`
   ([testing.rs:124](../../../../crates/roym_core/src/testing.rs#L124))
   records each `query` (`testing/data_layer.rs:69`); `get` (`:43`) is
   not recorded. AG-28 needs every read. C6a adds a `reads()` log of
   (collection, id or filter) for `get`, `query` and `query_raw`.
10. **`create` is the unique insert.** `AppDataLayer::create` writes rows
    "together or not at all, and names the first row that already exists"
    (agreements doc `:184`). A log row's id is derived from its event, so
    a second create of the same row is refused, and the derive step needs
    no read of `event_log` to stay exactly-once (AG-27, AG-28).
11. **The method tables.** `transaction` routes 47 methods
    ([api.rs:31](../../../../crates/roym_transaction/src/api.rs#L31) to
    `:87`). The method contract asserts 47
    ([contract.rs:83](../../../../crates/roym_web/tests/method_contract/contract.rs#L83))
    and checks -32013 for every row of the table (`:191` to `:216`). The
    parity list of wire-refused verbs, `scenario_144`
    ([transaction_validation.rs:8](../../../../crates/roym_web/tests/dual_build_parity/transaction_validation.rs#L8)
    to `:58`), lists 44: it already misses `deal.as-listed-status`,
    `quote.from-listing` and `transaction.drive`. C6a adds those three and
    the new ones.
12. **The highest parity scenario is 266**
    ([widget.rs:758](../../../../crates/roym_web/tests/dual_build_parity/widget.rs#L758)).
    C6a uses 267 to 276. `dual_build_parity/transaction.rs` (821) and
    `helpers.rs` (1,266) are on the oversized list; C6a adds a new
    `dual_build_parity/statement.rs` and does not grow either.
13. **No `obligation` service exists.** AG-30's `available` and
    `unavailable` forms need a sibling that M08E builds (task.md section
    5). A sibling call to a service that is not a dependency fails as a
    transport error, which would wrongly read as `unavailable`. So in M08C
    `statement.export` always answers `not-available` and makes no call
    (D-C6-7).
14. **Contacts are in `profile`.** See Q6.
15. **JavaScript numbers are exact only up to 2^53.** A declared value is
    at most 10^15 (AG-23), but 16 of them sum to 1.6 × 10^16. The Hub's
    totals and the statement's totals in the Hub use `BigInt`
    (D-C6-10). `Intl.NumberFormat.format` takes a `BigInt`.
16. **Who settles what in today's settlement rules.** The receiver's half
    settles from `promised` (AG-46: "A `payment-request` is never
    required"). The transition tables allow it (agreements doc `:691`,
    `:699`). So "Received" and "I received everything" need no payment
    request.
17. **The listing editor already fills `deal_terms`** from its own
    as-listed section ([editor.ts:289](../../../../crates/roym_web/ui/src/listings/editor.ts#L289)
    to `:328`; [as_listed_section.ts:12](../../../../crates/roym_web/ui/src/listings/as_listed_section.ts#L12)
    to `:97`), with no saved defaults. C6b fills those inputs from the
    saved deal terms when they are empty.
18. **The test helper `sendQuote` uses today's form classes**
    ([hub-helpers.ts:335](../../../../crates/substrate/tests/e2e/hub-helpers.ts#L335)
    to `:355`): `.quote-request-button`, `.quote-scope-input`,
    `.quote-give-input`, `.quote-currency-input`, `.quote-amount-input`,
    `.quote-payee-input`, `.quote-where-select`, `.submit-quote-button`.
    Stories 40 and 50 use it. C6b keeps these classes on the first rows
    of the new form, so the helper and both stories keep working
    (D-C6-9), and adds `addCommitment` to the helper.
19. **Two backlog rows target C6 but are out of its scope.** "The Hub has
    no action to mark work started" (target "M08C C6") is a booking step,
    and task.md says "Booking steps and payment requests stay out of the
    Hub". C6b moves its target to TBD, with the reason. The Hub action
    panel row (target C6) shrinks to booking steps and payment requests,
    as task.md section 7 says.

## 3. What C6 meets

| Change doc | Criterion | Part | Where it is proved (section 7) |
| --- | --- | --- | --- |
| roym-agreements | AG-23 | The Hub part: before signing and on the agreement panel, each value, the total given, the total received, the difference when they differ; never a block (C6b). The range rule was C1. | Hub `deal_totals.test.ts`, `templates/quote_totals.test.ts`, `agreement_panel.test.ts`; Playwright `roym-deal.spec.ts` case D1 |
| roym-agreements | AG-27 | Every event once, one event per final commitment state; a repeated sync and `eventlog.rebuild` add nothing (C6a) | unit `eventlog/tests.rs`; service `tests/eventlog.rs` on both nodes; parity 267, 268 |
| roym-agreements | AG-28 | Nothing on the deal path reads `event_log` (C6a) | service `tests/eventlog_reads.rs` with the read log |
| roym-agreements | AG-29 | `statement.export` by `occurred_at_secs` of the settling half; given and received; totals per currency; with and without money (C6a); the Hub's export (C6b). On the counterparty node, settled commitments list once the author's snapshot is filed | unit `statement/tests.rs`; service `tests/statement.rs`; two-node `roym_statement_e2e`; parity 270; Playwright D5 and story 50 |
| roym-agreements | AG-30 | The `eeps` section is always present; `not-available` in M08C. The `available` and `unavailable` forms are typed and serialised. The fake-sibling test is M08E (task.md section 5) | unit `statement/tests.rs` |
| roym-agreements | AG-33 | The statement part: `statement.export` shows the receiver's note on the row (C1 built the note, C2 hid it) | service `tests/statement.rs` |
| roym-agreements | AG-46 | `agreement.close` signs one half per listed commitment, skips final ones with `final` (`records: []` on repeated close; repeated claims return stored records) (C6a); the Hub list, which leaves out a commitment whose window has not started unless picked (C6b) | service `tests/close.rs`; parity 271, 272; Hub `close_list.test.ts`; Playwright D2 |
| roym-agreements | AG-47 | The combined summary per side, from disclosures the node holds (C6b) | Hub `version_summary.test.ts`; Playwright D3 |
| roym-agreements | AG-48 | The fill order: listing, then saved deal terms, then built-in text; payee; `INR`; "More terms" closed (C6b); the saved terms on the node (C6a) | Hub `quote_defaults.test.ts`; service `tests/defaults.rs`; parity 275; Playwright D4 |
| roym-agreements | AG-52 | "Suggest changes" starts a message naming the version (C6b) | Hub `suggest_changes.test.ts`; Playwright D3 |
| roym-value-exchange | VX-2 | Any built kind on either side, from the Hub form (C6b) | Hub `quote_form.test.ts`; Playwright D1 |
| roym-value-exchange | VX-7 | Several commitments in one offer from the Hub (C6b) | Hub `quote_form.test.ts`; story 50 |
| roym-value-exchange | VX-45 | Through AG-27 and AG-28 | as above |
| roym-value-exchange | VX-46 | Through AG-29 | as above |
| roym-value-exchange | VX-55 | Through AG-46; an unconfirmed end shows as a normal end in the panel | as above; Hub `agreement_panel.test.ts` |
| roym-value-exchange | VX-56 | Through AG-23 and AG-47 | as above |

Not met here: AG-30's `available` and `unavailable` with a real or fake
`obligation` sibling (M08E). AG-48's fill from in-return terms (M08D,
task.md section 5).

## 4. Behaviour-change audit

### Wire changes (between nodes)

- **None.** No new card type, record type, field or WIT change. "Suggest
  changes" sends an ordinary text message (Q7 (a)). `CARD_TYPES` and
  `RECORD_TYPES` do not change.

### Local method changes (one node)

- **New local-only methods on `transaction`** (D-C6-6):
  `agreement.close`, `eventlog.list`, `eventlog.rebuild`,
  `statement.export`, `deal.defaults-get`, `deal.defaults-set`, and the
  pings `eventlog.ping` and `statement.ping`. 47 to 55 methods. Every one
  answers -32013 off the node (AG-31). `agreement.` and `deal.` are
  routed already ([router.rs:39](../../../../crates/roym_core/src/router.rs#L39),
  `:44`). The router needs two new rows,
  `("eventlog.", TRANSACTION, MethodAuth::Owner)` and
  `("statement.", TRANSACTION, MethodAuth::Owner)`, in the `ROUTES`
  table ([router.rs:24](../../../../crates/roym_core/src/router.rs#L24)).
- **`payment.acknowledge`** gets an `acknowledge_at(host, input, now)`
  form, like `fulfilment.sign`'s `sign_at`. Its behaviour does not change.
- **`transaction.sync` and `transaction.drive`** run the derive step at
  the end of `sync_direct` (D-C6-3). Its result never fails the sync and
  is not in `SyncOutput`.
- **`transaction.import`** runs the derive step over every imported
  agreement after its writes (Q5 (a)), and imports the new `deal_defaults`
  section (Q4 (a)). `TransactionImportOutput` gains `log_entries_made`.
- **`transaction.export`** carries the `deal_defaults` section.
  `SCHEMA_VERSION` 8 to 9 ([app.rs:61](../../../../crates/roym_transaction/src/app.rs#L61)):
  a version 8 bundle is refused, as for every earlier version (no
  migration, the product is not released).
- **New collections:** `event_log` (indexes `recorded_at_secs` numeric,
  `agreement` string), `event_log_state` (no index) and `deal_defaults`
  (no index), in `ensure_collections`
  ([app.rs:357](../../../../crates/roym_transaction/src/app.rs#L357)).
- **`roymctl roym transaction`** gains `agreement close`, `eventlog list`,
  `eventlog rebuild`, `statement`, `defaults get` and `defaults set`
  (D-C6-8).
- **Test host:** `$gte` and `$lte` in the filter; a `reads()` log.

### Hub changes (C6b)

- The quote form: commitment rows for either side, fill order, "More
  terms", the new words (D-C6-9).
- The quote card: two totals and the difference in place of the money
  total; "Agree" on the latest version; "Suggest changes" (D-C6-10,
  D-C6-12).
- The agreement panel under each agreement's newest card (Q2 (a),
  D-C6-13).
- The change summary on the latest version's card (D-C6-11).
- The Statement tab with two downloads (Q3 (a), D-C6-14).
- The listing editor: `INR` default (Q8) and the deal texts filled from
  the saved terms when empty.
- New words: "Offer" where it says "Quote", "Agree" where it says
  "Accept" (D-C6-15).

### Stays the same

- Every signed record, card, state, transition table and reason code.
- `payment.request` and every booking step: still not in the Hub.
- `conversation`, `catalog`, `scheduling`, `directory`, `profile` and
  `web` service code. (`web` serves the new Hub files; its Rust code does
  not change.)
- The event bus events of `transaction`.
- The agree-as-listed path, the driver and `quote.from-listing`.
- Stories 10, 20, 30, 40 and 60 (story 40 keeps passing: it clicks the
  accept button by message id, and the id does not change; D-C6-15).

## 5. Decisions this plan makes

### D-C6-1. The log and statement rules are pure, in `roym_core`

Two new modules, so both builds share them and unit tests need no host:

- `crates/roym_core/src/eventlog.rs` (+ `eventlog/tests.rs`): the entry
  type and `derive`.
- `crates/roym_core/src/statement.rs` (+ `statement/tests.rs`): the
  statement type and `build`.

`transaction` only gathers the inputs, creates the rows and answers the
methods.

### D-C6-2. The log entry

The agreements doc's shape (`:889` to `:910`), with `deny_unknown_fields`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogEvent {
    AgreementFormed,
    AgreementClosed,
    AgreementCancelled,
    AgreementNotFormed,
    CommitmentSettled,
    CommitmentFailed,
    CommitmentEndedUnconfirmed,
    CommitmentCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction { Given, Received }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventLogEntry {
    pub id: String,                 // "<agreement>:<commitment_id or '-'>:<event>"
    pub occurred_at_secs: u64,
    pub recorded_at_secs: u64,
    pub event: LogEvent,
    pub agreement: String,
    pub conversation: String,
    pub counterparty_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<ResourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_kind: Option<CommitmentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_state: Option<CommitmentState>,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_value_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_note: Option<LocalText>,
    pub records: Vec<String>,
}
```

`LocalText`, `ResourceKind` and `CommitmentKind` are the existing types
([deal.rs:37](../../../../crates/roym_core/src/deal.rs#L37), `:88`,
`:97`), and so is `CommitmentState`
([deal/transition.rs:15](../../../../crates/roym_core/src/deal/transition.rs#L15)).
Do not add string copies of them.

### D-C6-3. The derive step

**Input** (built by `transaction`, one per agreement):

```rust
pub struct DeriveInput<'a> {
    pub owner_role: DealRole,            // this node's party in the deal
    pub agreement: &'a str,
    pub conversation: &'a str,
    pub counterparty_did: &'a str,       // the other party, seen from this node
    pub terms: &'a DealTerms,
    pub state: Option<AgreementState>,
    pub commitments: &'a [CommitmentProgress],  // from the agreement row
    pub agreed: Option<FormedFact<'a>>,  // agreed_at_secs + the two receipt half ids
    pub snapshots: &'a [HeldSnapshot],   // every held snapshot: seq, issued_at_secs, record_id, state, commitments
    pub settling: &'a [SettlingHalf],    // per commitment: record ids, the receiver half's issued_at_secs, its note
}

pub fn derive(input: &DeriveInput<'_>) -> Derived;

pub struct Derived {
    pub entries: Vec<EventLogEntry>,     // recorded_at_secs = 0; the service sets it
    pub complete: bool,                  // false when a final state lacks a fact it needs
}
```

**The rules** (agreements doc `:915`):

| Event | When | `occurred_at_secs` | `records` |
| --- | --- | --- | --- |
| `agreement-formed` | `agreed` is set | `quote_author.issued_at_secs` (the countersigned half) | both receipt halves |
| `agreement-closed`, `-cancelled`, `-not-formed` | `state` is that end | `issued_at_secs` of the **lowest-`seq` held snapshot** with that state | that snapshot |
| `commitment-settled` | the commitment is `fulfilled` or `acknowledged` | the settling half's `issued_at_secs` (from snapshot `halves`) | every half of that commitment the node holds |
| `commitment-failed`, `-ended-unconfirmed`, `-cancelled` | the commitment is in that end | `issued_at_secs` of the lowest-`seq` held snapshot that shows it | that snapshot |

- `occurred_at_secs` of `agreement-formed` comes from `quote_author.issued_at_secs`
  (the countersigned half) and nothing else. If the node does not hold it yet,
  the entry is deferred (`complete: false`). `AgreementRow.agreed_at_secs` is not
  used because it can fall back to the local clock or snapshot arrival time.
- For `commitment-settled`, the settling half is taken from the half named in
  the snapshot's `halves` for that commitment and role. Later corrections
  with `supersedes` do not move this time.
- Commitment events carry `direction` (`given` when `owner_role` is the
  commitment's giver), the kinds, `final_state`, the currency and the
  declared value. `commitment-settled` carries the receiver half's
  `settlement_note`, when it has one.
- **A missing fact defers the entry, never guesses it.** If the commitment
  is `fulfilled` but the node does not hold the settling half yet (the
  counterparty's card is late), or no held snapshot shows a state the row
  holds, the entry is left out and `complete` is `false`. The next sync
  derives it. On the counterparty node, `commitment-settled` is derived
  once the author's snapshot is filed (P1 option a). So a time in the log
  always comes from a signed record.
- `not-formed` refusals of a superseded version (agreements doc `:657`)
  have an agreement row for that version's `record_id`; they give one
  `agreement-not-formed` for that row. That is correct: that acceptance
  did not form.

**Where it runs.**

- At the end of `sync_direct`, after the tick
  ([sync.rs:162](../../../../crates/roym_transaction/src/app/sync.rs#L162)):
  `let _ = eventlog_ops::derive_conversation(host, conversation, now).await;`
  The result is ignored, so the log never blocks a deal (VX-45).
- In `transaction.import` (Q5), `eventlog.list` (P7), `eventlog.rebuild` and
  `statement.export` (D-C6-7).

**Cost: a marker per agreement.** Without a marker, every sync would read
all snapshots, halves and disclosures of every agreement of the
conversation. `event_log_state` holds one row per agreement:
`{agreement, seq: u32, complete: bool}`. `derive_conversation` reads the
conversation's agreement rows (it already has the filter) and runs the
derive step only for an agreement whose row `seq` (or `state`, for a
`not-formed` row with no `seq`) differs from the marker, or whose marker
says `complete: false`. It writes the marker after the creates, only when
every create answered "created" or "repeat". On any error or failure, no
marker is written (or `complete: false`), so the next sync retries. A stop
in between repeats the creates, which the data layer refuses as repeats.
Every new event on both nodes comes with a new snapshot `seq` (the writer
writes one per change, `progress_ops.rs:146`; the counterparty files it),
so the marker never hides an event. `eventlog.rebuild` ignores the
markers.

**AG-28 holds.** The derive step writes `event_log` only with `create`. It
reads `agreements`, `agreement_progress`, `payments`, `fulfilments`,
`disclosures` and `event_log_state`, never `event_log`.

**Row ids and `recorded_at_secs`.** The service sets `recorded_at_secs =
now` and creates each entry alone (one `create` per row, because a
multi-row `create` fails as a whole when one row exists). A refused create
is a repeat and is not counted.

### D-C6-4. Times on both nodes

Both nodes give `agreement-formed` and `commitment-settled` the same time,
because both times come from records both nodes hold. The other events use
the earliest snapshot each node holds. When a snapshot card is lost, the
counterparty's earliest held snapshot can be a later one, so its time for
`commitment-failed`, `-ended-unconfirmed`, `-cancelled` and the agreement
ends can be later than the writer's. The statement uses only the two
times that are equal, so both parties' statements still match. `open_at_end`
uses only `agreement-formed` and `commitment-settled` (D-C6-7). This is a
deviation of the agreements doc's sentence "So both parties give one event
the same time" (`:915`), recorded in section 9.

### D-C6-5. `eventlog.list` and `eventlog.rebuild`

```text
eventlog.list    { after?: { recorded_at_secs, id }, limit?: u32 (1..=500, default 100) }
              -> { entries: [EventLogEntry], next?: { recorded_at_secs, id } }
eventlog.rebuild {} -> { made: u32 }
```

- `list` runs the marker-gated derive step over every agreement first (as
  `statement.export` does), so a local settlement shows immediately without
  waiting for a sync. It queries `event_log` with
  `{"recorded_at_secs": {"$gte": after}}` (or no filter), reads every page
  (`paging::query_all`), sorts by `(recorded_at_secs, id)`, drops rows at or
  before the cursor and returns `limit` rows. AG-28 still holds: the derive
  step reads `event_log_state`, never `event_log`. The data layer has no
  ordered query (point 8). A backlog row records the cost for a large log.
- `rebuild` runs the derive step over every agreement row, ignoring the
  markers, and answers the number of rows it created.

### D-C6-6. `agreement.close`

```text
agreement.close { agreement, as: "giver" | "receiver", commitments: [commitment_id] }
             -> { records: [ { commitment_id, record_id, message_id?, state: SendState } ],
                  skipped: [ { commitment_id, reason: "final" } ] }
```

- **Check every id first, sign after.** Before it signs anything, it
  refuses the whole call with `-32602` and `data.code` if an id is not in the
  agreement (`no-such-commitment`), if this node's party is not the
  commitment's `as` side (`not-your-side`), if the list is empty or has a
  repeat (`bad-commitment-list`). An agreement that is `closed` is not an
  error: every listed id (in the agreement and on the caller's side) is
  `skipped` with reason `final`. Refuse `agreement-not-open` only for
  `cancelled`, `not-formed` and an agreement that is not yet formed. So a
  bad list signs nothing.
- **Then one half per id, in the order given,** through the existing
  paths: `fulfilment_ops::sign_at` for a `fulfilment` commitment and
  `payment_ops::acknowledge_at` for a `currency` one, with no note,
  method or reference. This reuses their fences, their idempotency and
  the writer's transition. No new signing code.
- **Repeat rule and final commitments.** A commitment that is already final
  is put in `skipped` with `final` (no record returned). A commitment whose
  half this node already signed and that is not final returns that half in
  `records`: the underlying method answers `AlreadyRecorded` with the
  record. Settling halves of a repeat are not returned because the first
  call made the commitments final; so a repeated close of all commitments
  gives `records: []` and `skipped: [...]`. A repeated claim (giver half)
  returns stored records.
- **Refusal response shape.** Every new refusal in `transaction` uses the
  two-call form `Response::invalid_params(code).with_data(json!({"code": code}))`
  (for example `no-such-commitment`, `not-your-side`, `bad-commitment-list`,
  `agreement-not-open`, `bad-period`, `statement-total-overflow`,
  `bad-deal-defaults`). The code is both the error message and the value of
  `data.code`; the Hub reads `data.code`.
- If one signature fails (for example `NotEnrolled`), the call stops and
  returns that error. The halves already made stay. A retry makes only
  the missing ones. The Hub says so.
- The method does not filter by `window`. The Hub builds the list (AG-46,
  D-C6-13), and the person confirms exactly what is signed.

### D-C6-7. `statement.export`

```text
statement.export { from_secs, to_secs } -> Statement     // [from, to)
```

`Statement` is the agreements doc's shape (`:925` to `:946`), with
`deny_unknown_fields`, `statement_version: 1`, and:

- `counterparty_name` is always absent from the service (Q6 (a)).
- `description` comes from the agreement row's plain terms. The log entry
  holds no description; `build` joins entries with agreement rows.
- `open_at_end`: the commitments of every agreement whose
  `agreement-formed` occurred before `to_secs` and that have no
  `commitment-settled` entry before `to_secs`. A commitment that ended in
  any final state (`failed`, `cancelled`, `ended-unconfirmed`) is not open
  when the node holds any final entry for it, whatever its time.
- `totals`: per `(currency, direction, resource_kind)`, the sum of
  `declared_value_minor` with `checked_add`. An overflow (more than 2^53 - 1,
  or 9,007,199,254,740,991, in one group) refuses the export with
  `statement-total-overflow`. This protects JavaScript and JSON readers from
  silent number corruption.
- `eeps` is `{ "status": "not-available" }` in M08C. `EepSection` has all
  three forms, so M08E fills it without a shape change (point 13).
- Before it builds, it runs the marker-gated derive step over every
  agreement, so a commitment settled seconds ago through a local method on
  the author's node is in the export without waiting for a sync. On the
  counterparty node, a settled commitment appears in the export after the
  author's next snapshot is filed.
- `from_secs >= to_secs` is refused with `-32602`, `bad-period`.

`statement::build(owner_did, from, to, now, entries, agreements) ->
Result<Statement, StatementError>` is the pure part.

### D-C6-8. `roymctl`

In [agreement.rs](../../../../apps/roymctl/src/commands/roym/agreement.rs)
(347 lines) and a new `statement.rs` beside it:

- `roym transaction agreement close --agreement <rec> --as giver|receiver
  --commitment <id> [--commitment <id> …]`
- `roym transaction eventlog list [--after-secs N --after-id ID] [--limit
  N]` and `roym transaction eventlog rebuild`
- `roym transaction statement --from <YYYY-MM-DD> --to <YYYY-MM-DD>`
  (dates in UTC; prints the JSON)
- `roym transaction defaults get` and `defaults set --cancellation …
  --refund … --dispute … --lang … --quote-valid-hours N`

### D-C6-9. The quote form (C6b)

Move the form out of `messages.ts` (1,226 lines) into a folder
`crates/roym_web/ui/src/quote_form/`:

- `form.ts`: `openQuoteForm` (moved from `messages.ts:910`), the rows and
  the "More terms" section.
- `rows.ts`: one commitment row. Fields: who gives ("I give" / "They
  give"), what kind ("Service", "Goods", "Money"), description (not for
  money: money's description is built as today, `"<currency> <amount>"`),
  declared value (for money: the amount), optional window ("When": from
  and to) for a service or goods row, location for the author's service
  row, and for money the payee, tax and fees. Add and remove buttons; 1 to
  16 rows.
- `defaults.ts`: the fill order (AG-48), as a pure function
  `fillQuoteForm(listing, saved, builtIn) -> FormValues`:
  - from the listing (`listing.get` then `listing.verify`, the path
    `as_listed_tap.ts:99` uses): currency, price (one "They give: Money"
    row with the listing's `amount_minor`), payment timing (`after` of
    the rows), location, payee (the listing's `PaymentTerms.payee`), and
    its `deal_terms` texts and `quote_valid_secs`;
  - then the saved deal terms (`deal.defaults-get`): the three texts and
    the validity;
  - then built-in text: today's catalog defaults (`quote-default-*`), a
    validity of 24 hours, and the author's own DID as payee;
  - the currency falls back to `INR`.
  - A field filled by any of these goes into "More terms", which starts
    closed. The scope and every row the author must check stay open.
- `../screens/quote_terms.ts` `buildDealTerms` takes the list of rows and
  builds one commitment per row, in row order (the service assigns `c1` to
  `c16` by position, AG-37). Its money rows keep today's shape. The
  `after` order comes from the payment-timing choice: "before the work"
  puts every money row the counterparty gives before every service row the
  author gives; "after the work" the other way. It gives no `after`
  otherwise. (Today it sets `after` only between `c1` and `c2`.)
- The form keeps `.quote-scope-input`; the first author row keeps
  `.quote-give-input`; the first money row keeps
  `.quote-currency-input`, `.quote-amount-input`, `.quote-payee-input`;
  the form keeps `.quote-form`, `.quote-where-select`, `.quote-size-line`
  and `.submit-quote-button`. So `sendQuote` keeps working (point 18).
- The size line (`quote_size.ts`) is attached as today.
- The "Make an offer" button of the enquiry card replaces "Quote this
  request" (`messages.ts:784`), with the same class.
- A listing has no "goods in return" field until M08D (agreements doc
  `:866`), so the form pre-fills only a money row. The author adds goods
  or service rows for the other side by hand.
- The fresh form has one author service row and one "They give: Money"
  row (keeping the classes of point 18). The service row's declared value
  starts as the money row's amount and follows it until the author types
  in it. If the author does not want to value a goods or service row,
  the field is still required (every commitment has a value, AG-23);
  a hint under it says "What this is worth in <currency>. Used only for
  the totals."
- The form does not offer `service-in-slot` rows: a slot is booked through
  the agree-as-listed path or `roymctl`, as today.

### D-C6-10. Totals and the difference (C6b)

A pure function in `crates/roym_web/ui/src/deals/deal_totals.ts`:

```ts
export function dealTotals(commitments: Commitment[], me: "quote-author" | "counterparty"):
  { give: bigint; receive: bigint; difference: bigint }   // difference = receive - give
```

- It sums `declared_value_minor` with `BigInt` (point 15) over every
  commitment, whatever its kind (AG-23, AG-22: no rule depends on whether
  money is in the deal).
- The quote card shows, for the reader: "You give: …", "You receive: …",
  and, when they differ, "You receive ₹100 more than you give" or "You
  give ₹100 more than you receive". No threshold. It never hides or
  disables "Agree".
- The quote form shows the same block for the author, live as the rows
  change.
- The agreement panel shows it after the deal forms.
- `formatMinor` ([money.ts:80](../../../../crates/roym_web/ui/src/money.ts#L80))
  gains a `bigint` overload.

### D-C6-11. The change summary (C6b)

A pure function in `crates/roym_web/ui/src/deals/version_summary.ts`:

```ts
export function versionSummary(first: DealTerms, latest: DealTerms, me: Role):
  { mine: SideChange; theirs: SideChange }
// SideChange = { added: Commitment[]; removed: Commitment[];
//                changed: { before: Commitment; after: Commitment }[];
//                totalBefore: bigint; totalAfter: bigint }
```

- "Mine" is what the reader gives, "theirs" what the other party gives.
- Two commitments match when they have the same giver, the same
  `resource_kind` and the same description original `text`. A matched
  pair whose value, window, location, payee, tax or fees differ is
  `changed`. An unmatched one is `added` or `removed`. Ids are not used:
  they come from position (AG-37). A description that changed shows as
  one removed and one added, which is a fact the reader should see.
- The Hub gets the versions from `quote.history` (`QuoteVersionItem
  {record_id, terms}`, [quote_ops.rs:131](../../../../crates/roym_transaction/src/app/quote_ops.rs#L131)),
  which holds the plain values from the stored disclosures. It calls it
  only for a card whose `version_count` is above 1.
- The summary is drawn above the latest version's card, for both parties.
  Nothing is stored (AG-47).

### D-C6-12. "Suggest changes" (C6b, Q7 (a))

- On the counterparty's view of the latest version's card of an open
  deal, next to "Agree" and "Decline".
- A tap puts the line "About offer version N: " (catalog id
  `deal-suggest-prefix`, in the writer's Hub language) in the chat's
  compose box, focuses it, and scrolls to it. The person writes and sends
  it as a normal message. Nothing is sent until they tap Send.
- N is the version's position in `quote.history`, from 1. The card shows
  "Version N" on every quote card from now on, so both people read the
  same number.
- The author answers with a new version, through the same "Make an
  offer" form opened on the quote card ("Send a new version"), filled
  from the latest version's terms. No record changes (VX-5).

### D-C6-13. The agreement panel (C6b, Q2 (a))

`crates/roym_web/ui/src/deals/agreement_panel.ts`:

- Drawn once per agreement, below the newest card of that agreement in
  the thread (receipt, progress or settlement card), from
  `agreement.get {quote_record_id}`.
- One line per commitment: who gives, the description in the reader's
  best language (`bestVersion`), the value, the state word, and the
  reason for `failed` and `cancelled`. `ended-unconfirmed` reads as a
  normal end, with AG-8's two sentences, never as an error (VX-55).
- The totals block (D-C6-10).
- **Per-commitment actions** (task.md section 6, question 2), only while
  the agreement is `agreed` and the commitment is not final:
  - giver of a `fulfilment`: "I delivered" → `fulfilment.sign`;
  - giver of a `currency`: "I paid" → `payment.acknowledge`;
  - receiver of either: "Received" → the same method.
  - Each action opens a small form with an optional "How it was handed
    over" note (`settlement_note`, `LocalText` in the Hub language) and a
    confirm button. A sent half shows its send state and a retry, as
    other cards do.
  - A giver who already claimed sees "You said you delivered" and no
    button. A receiver can still settle a claimed commitment.
- **Closing actions** (AG-46): "I gave everything" and "I received
  everything", shown when this person has at least one open commitment
  on that side. A tap shows the list it will sign: each open commitment
  of that side with its value, checked; a commitment whose `window`
  starts after now is listed unchecked, with "Not started yet". The
  person can check it. "Sign" calls `agreement.close` with the checked
  ids. The pure function `closeList(view, side, now)` is in
  `deals/close_list.ts`.
- **Counterparty waiting state.** On the counterparty's node, when this
  node has signed its half for an open commitment but the author's next
  snapshot has not yet arrived to confirm it, the commitment line shows
  "Sent. Waiting for the other person's node." with no button.
- **Form elements in open work.** The settlement note form gets class
  `.settle-dialog`, and the close list gets class `.close-list`. Both join
  `.quote-form` in `OPEN_WORK` (`messages.ts`), so thread redraws do not
  destroy what a person is typing.
- The panel redraws after each action from `agreement.get`, never from
  its own guess (C1 R38, C2 R2).

### D-C6-14. The statement screen (C6b, Q3 (a))

`crates/roym_web/ui/src/screens/statement.ts`, tab key `statement`:

- A period picker: "This month", "Last month", "This year", or two dates.
  Dates are the Hub's local days, turned into `[from, to)` seconds at
  local midnight.
- "Show" calls `statement.export`, then `contacts.list` for the names
  (Q6 (a)), and draws the rows, `open_at_end`, the totals per currency and
  direction, and the EEP line: "Not available on this node" for
  `not-available`, "Could not be read: …" for `unavailable`.
- "Download CSV" and "Download JSON". The JSON is the service's object
  with `counterparty_name` filled. The CSV has one line per row, with the
  columns `agreement, commitment_id, agreed_at, settled_at, direction,
  counterparty_did, counterparty_name, resource_kind, commitment_kind,
  description, currency, declared_value, settlement_note`. Dates are ISO
  8601 with the local timezone offset (matching the local days picker).
  `declared_value` is the decimal amount (`minorToDecimal`), so a
  spreadsheet reads it as a number. `description` uses the original text
  from `LocalText`. Fields are quoted as RFC 4180 says.
- **CSV formula guard.** A cell that starts with `=`, `+`, `-`, `@`, a tab or a
  carriage return gets a leading `'`, so a spreadsheet does not run it as
  a formula. The guard applies to every cell of all text columns (including
  `counterparty_name`), not only description and note. The pure writer is
  `statement_csv.ts`.
- Hub totals for quotes use `BigInt` (D-C6-10), while statement totals stay
  within 2^53 - 1 on the wire (D-C6-7).
- One download helper `download(name, blob)` in `crates/roym_web/ui/src/download.ts`;
  the backup screen's inline code ([backup.ts:95](../../../../crates/roym_web/ui/src/screens/backup.ts#L95)
  to `:101`) moves to it.

### D-C6-15. The new words (C6b)

- English texts change; message ids stay. Ids are stable keys that tests
  and stories use; a rename of `messages-accept-button` would touch every
  test that names it and gives the reader nothing.
- The changed English texts (each needs a new Hindi text for the owner's
  review): `quote-title` "Offer", `quote-request-button` "Make an offer",
  `quote-send-button` "Send offer", `messages-accept-button` "Agree",
  `messages-accept-mt-confirm` "Agree to the original terms",
  `receipt-expiry-label` "Offer expires:", `receipt-quote-to`,
  `receipt-quote-by`, `receipt-quote-record`, `quote-failed`, and the
  lower-case "quote" lines that a person reads (en.ftl lines 38 to 40, 53,
  59, 102, 112, 116, 124, 128 to 131, 195, 307, 315, 460, 485, 486, 522,
  532 to 534, 553, 554, 589, 900, 901, 911, 929; the implementer checks
  each and lists the changed ids in the PR).
- Words in records, method names, error codes and docs for developers
  stay "quote".

### D-C6-16. The saved deal terms (Q4 (a))

```text
deal.defaults-get {} -> DealDefaults | null
deal.defaults-set DealDefaults -> DealDefaults
DealDefaults { cancellation_terms: LocalText, refund_terms: LocalText,
               dispute_path: LocalText, quote_valid_secs: u64 }
```

- One row, id `defaults`, in `deal_defaults`. The texts follow the deal
  text rules (`check_text`, 2,048 bytes each, AG-39's one set of
  languages); `quote_valid_secs` from 300 to 90 days, as `quote.set`. If
  validation of `deal_defaults` fails on any text or `quote_valid_secs`,
  both `deal.defaults-set` and `transaction.import` refuse with code
  `bad-deal-defaults` (`Response::invalid_params("bad-deal-defaults").with_data(json!({"code": "bad-deal-defaults"}))`).
- The quote form's "More terms" has "Save as my defaults".
- The listing editor fills its empty deal texts from them (point 17).
- They never reach the as-listed path: that path reads only the signed
  offering (AG-48).
- Exported and imported as section `deal_defaults`
  (`crates/roym_transaction/src/app/backup/defaults.rs`) with constant
  `SECTION_DEAL_DEFAULTS` in `roym_core::backup`.
- `transaction.import` validates imported `deal_defaults` texts using
  `check_text` (matching the method check).
- `TransactionImportOutput` gains `log_entries_made` (included in method
  shapes and Hub backup types).

## 6. Step-by-step changes

Each step builds, and its tests pass, before the next starts.

### 6.1 C6a — Event log, statement and closing on the node

#### Step 1 — Test host

`crates/roym_core/src/testing.rs`, `testing/data_layer.rs`: add `$gte` and
`$lte` to `matches_filter`, and a `reads: Mutex<Vec<(String, String)>>`
log with a `reads()` getter, filled by `get` and `query` (leaving `query_raw`
`unimplemented!()` as nothing in `roym_transaction` calls it). Unit tests for
both in `testing` (or the nearest test file).

#### Step 2 — The log rules (`roym_core`, pure)

`eventlog.rs`: the types of D-C6-2 and `derive` of D-C6-3. Unit tests
(section 7.1).

#### Step 3 — The statement rules (`roym_core`, pure)

`statement.rs`: the types and `build` of D-C6-7. Unit tests.

#### Step 4 — The collections and the derive step (`roym_transaction`)

- `app.rs`: the three collection constants and their `ensure_coll` lines.
- New `app/eventlog_ops.rs` (+ `eventlog_ops/gather.rs` for building
  `DeriveInput` from rows): `derive_conversation`, `derive_all`,
  `list`, `rebuild`.
- `sync.rs`: one line after `:162`.
- Service tests `tests/eventlog.rs` and `tests/eventlog_reads.rs`.

#### Step 5 — `statement.export`

New `app/statement_ops.rs`. Service test `tests/statement.rs`.

#### Step 6 — `agreement.close`

- `payment_ops.rs`: split `acknowledge` into `acknowledge` and
  `acknowledge_at` (no behaviour change).
- New `app/close_ops.rs`. Service test `tests/close.rs`.

#### Step 7 — Saved deal terms and the backup

- `crates/roym_core/src/backup.rs`: add constant `SECTION_DEAL_DEFAULTS` and
  add to `each_new_section_passes_check_integrity_on_its_own`.
- New `app/defaults_ops.rs` and `app/backup/defaults.rs`.
- `backup.rs`: the section in the export list and the import; validate imported
  defaults with `check_text`; if validation fails, both `deal.defaults-set`
  and the import refuse with code `bad-deal-defaults`; the derive step at the end
  of the import; `SCHEMA_VERSION` 9 with one line in its comment. Keep `backup.rs`
  under 800 lines by putting the new code in `backup/defaults.rs`; if it still
  crosses, move `validate_bundle_header` (`backup.rs:191`) to `backup/header.rs`.
- Update `dual_build_parity/transaction_cards.rs:348` to `:352` assertions for
  `schema_version` 9.
- Method shapes and Hub type for `TransactionImportOutput.log_entries_made`.
- Service test `tests/defaults.rs`; the backup round trip in
  `backup/tests.rs` gains the section (or a new `backup/tests/defaults.rs`).

#### Step 8 — The method table and the router

`api.rs`: the eight new rows. `router.rs`: the two prefixes. Every new
refusal uses the two-call form
`Response::invalid_params(code).with_data(json!({"code": code}))` (or a helper
in `app/refusal.rs`). Method contract tests in
`crates/roym_web/tests/method_contract/`: update count from 47 to 55
(`contract.rs:83`), add shape rows (`shapes_transaction.rs`), and add one
contract test per new refusal checking that the returned error message and
`data.code` both match the code string. `scenario_144`'s list gets the three
missing methods and the eight new ones.

#### Step 9 — Parity

New `crates/roym_web/tests/dual_build_parity/statement.rs`, scenarios 267
to 276 (section 7.2). Run `mise run build:roym` first. Scenario 272 asserts
repeat close gives `records: []` and three `skipped: [final]`, while repeated
claim returns stored records. Update schema version assertions in parity for
version 9.

#### Step 10 — `roymctl`

D-C6-8. Tests beside the existing `agreement` command tests.

#### Step 11 — The two-node test

New `crates/substrate/tests/roym_statement_e2e.rs` (section 7.4), with
`common::roym_flow` helpers. If a helper for a three-commitment deal is
needed, add it to `roym_flow.rs` (433 lines), not a local copy. Move
`setup_barter_fixture`, `BarterFixture`, `exchange_barter_fulfilments` from
`roym_deal_e2e.rs:232` to `roym_flow.rs` as `setup_mixed_fixture`,
`MixedFixture`, `exchange_mixed_fulfilments`, and fix header comment.
`templates.test.ts:527` test string `barter-tokens` stays as an unknown payment
method fixture. Test 7.5 performs two-way sync after `agreement.close` calls
before comparing statements.

#### Step 12 — Sizes, docs and close-out of C6a

Sizes (section 7.9), the Deviations (section 9), the backlog (section 8),
the living docs of C6a, status.md (C6a row; the C4a row, point 1).

### 6.2 C6b — The deal screens in the Hub

#### Step 1 — Pure parts

`deals/deal_totals.ts`, `deals/version_summary.ts`, `deals/close_list.ts`,
`quote_form/defaults.ts`, `statement_csv.ts`, `download.ts` (and the
backup screen moved to it). Each with its unit test.

#### Step 2 — The quote form

`quote_form/form.ts` and `rows.ts`; `quote_terms.ts` takes rows;
`messages.ts` calls `openQuoteForm` from the new place. The form keeps
`.quote-form` in `OPEN_WORK`. The quote form cases of `messages.test.ts`
(`:256`, `:268`, `:300`) move to `quote_form.test.ts`; `quote_terms.test.ts`
is updated.

#### Step 3 — The quote card

The totals block, "Version N", "Suggest changes", the summary. Card
template tests in `templates.test.ts` (905 lines; put new cases in a new
`templates/quote_totals.test.ts`).

#### Step 4 — The agreement panel

`deals/agreement_panel.ts`, wired in the thread redraw
(`screens/thread_redraw.ts`). The panel handles the "Sent. Waiting for
the other person's node." state when a half is signed locally before the
snapshot arrives. Settlement note form (`.settle-dialog`) and close list
(`.close-list`) join `OPEN_WORK`. `agreement_panel.test.ts`, a thread wiring
test (C4 R30), and a Hub test in `thread_redraw.test.ts` style verifying that
an incoming message redraw does not replace an open settle dialog
(`.settle-dialog`) or close list (`.close-list`).

#### Step 5 — The statement screen and the tab

`screens/statement.ts`; `navigation.ts`, `main.ts`, both `.ftl` files.
`statement.test.ts`.

#### Step 6 — Words, Hindi, the listing editor

D-C6-15; Q8; point 17. `catalog.test.ts` and `hindi-screens.test.ts`
pass with the new and changed ids.

#### Step 7 — Playwright and the demo

New `crates/substrate/tests/e2e/tests/roym-deal.spec.ts` (cases D1 to D5,
section 7.6); `hub-helpers.ts` gains `addCommitment`, `openAgreementPanel`,
`settle`; story 50's third case; cases 29 to 31 of `roym-hub.spec.ts`
checked against the new words.

#### Step 8 — Close-out of C6

Sizes (`web` only), living docs, demo.md, the change docs, status.md, the
backlog, `mise run verify` with no `--skip`.

## 7. Tests

### 7.1 Unit tests (`roym_core`)

`eventlog/tests.rs`:

- one case per event kind (8), each checking `occurred_at_secs` and
  `records`;
- `direction` from each side, for a commitment each party gives;
- a settled commitment whose settling half is not held gives no entry and
  `complete: false`; adding the half gives the entry;
- a corrected acknowledgement does not move `occurred_at_secs`;
- a row whose `agreed_at_secs` differs from the half's time uses the half's time;
- the lowest-`seq` snapshot wins when two held snapshots show the same
  end; with the first one missing, the second one's time is used;
- `derive` run twice on the same input gives equal entries (the ids are
  the same, so the second create is a repeat);
- a deal with no currency commitment gives the same kinds of entries
  (AG-22).

`statement/tests.rs`:

- a row exactly at `from_secs` is in, one at `to_secs` is out;
- given and received rows from one deal; totals per currency,
  direction and resource kind;
- failed, cancelled and ended-unconfirmed are not rows and not in
  `open_at_end`; an agreed commitment with no final entry before `to` is
  in `open_at_end`;
- `open_at_end` excludes commitments that have any held final entry;
- a commitment settled after `to` but formed before is in `open_at_end`;
- the note of the settling half is on the row (AG-33);
- the three `EepSection` forms serialise to the agreements doc's JSON;
- `checked_add` overflow (above 2^53 - 1) gives `statement-total-overflow`;
- a struct test that fails when a field is added to `Statement` or
  `EventLogEntry` without a test change (C3c R21).

### 7.2 Parity (both builds agree), `dual_build_parity/statement.rs`

| # | Scenario |
| --- | --- |
| 267 | the writer settles both commitments; `eventlog.list` gives the same entries in both builds without waiting for a sync |
| 268 | a second `transaction.sync` and `eventlog.rebuild` make no new entry (`made: 0`) |
| 269 | `eventlog.list` paging: limit 2 over 5 entries, with the cursor |
| 270 | `statement.export` for a period with given and received rows and totals |
| 271 | `agreement.close` as receiver over three commitments |
| 272 | a repeated `agreement.close` returns records: [] and skipped: [final]; repeated claim returns stored records |
| 273 | `agreement.close` with a foreign id refuses and signs nothing |
| 274 | `statement.export` with `from >= to` refuses `bad-period` |
| 275 | `deal.defaults-set` then `deal.defaults-get`; a bad text is refused |
| 276 | export and import: the defaults survive, and the log is rebuilt (`log_entries_made`) |

### 7.3 Method contract

- Count 55 (`contract.rs:83`); typed input and output for each new row.
- -32013 off the node for each new row (automatic, `:191`), and in
  `scenario_144`'s list.
- One refusal contract test per new refusal (`no-such-commitment`,
  `not-your-side`, `bad-commitment-list`, `agreement-not-open`,
  `bad-period`, `statement-total-overflow`, `bad-deal-defaults`) checking
  that the refusal message equals the code string and `data.code` equals
  the code string.

### 7.4 Service tests (`roym_transaction/src/app/tests/`)

- `eventlog.rs`: one test per event kind on the writer's node, and the
  same on the counterparty's node after it files the snapshot; a crash
  after the creates and before the marker (`crash_on_write` on
  `event_log_state`) gives the same rows after the next sync; a
  `fail_write` on `event_log` does not fail `transaction.sync`; after the
  `fail_write` is removed, the next ordinary sync derives the entry;
  `eventlog.list` runs derive step with no sync.
- `eventlog_reads.rs` (AG-28): a full deal (enquiry, quote, accept,
  countersign, two settlements, close) runs with the read log on; no read
  names `event_log`. Then `eventlog.list` does read it (the test checks
  the log works).
- `statement.rs`: the export on both nodes of one deal gives the same rows
  and dates; counterparty statement verified before and after author snapshot
  is filed; the note (AG-33); a deal with and without money (AG-29);
  `eeps` is `not-available`.
- `close.rs`: one call settles every receivable commitment of a
  three-commitment deal; a repeat returns records: [] and three skipped
  final; repeated claim returns stored records; a closed agreement skips all
  as final; a bad list signs nothing; a failure in the second signature
  leaves the first half and a retry makes the rest.
- `defaults.rs`: set, get, refusals; the backup round trip.

### 7.5 Two-node and the substrate (`crates/substrate/tests/roym_statement_e2e.rs`)

- `both_sides_close_a_three_commitment_deal`: Bob gives a lesson, Alice
  gives goods and money. Alice calls `agreement.close` as receiver and as
  giver; Bob as giver and as receiver. Both nodes perform two-way sync
  (or wait for the driver); both nodes reach `closed`; both statements
  for the period list three rows with the same `settled_at` and opposite
  directions.
- `the_driver_logs_with_no_hub`: the counterparty's log gets its
  `commitment-settled` entries through the wake alone (no `transaction.sync`
  from the test), within the wait the driver tests use.

### 7.6 Hub unit tests (`npm test`) and Playwright (C6b)

Vitest:

- `deal_totals.test.ts`: both sides, equal and unequal totals, `BigInt`
  above 2^53.
- `version_summary.test.ts`: added, removed, changed, the totals, a
  changed description.
- `close_list.test.ts`: a window next week is unchecked; a final one is
  not listed.
- `quote_defaults.test.ts`: each level of the fill order; `INR`; the payee
  falls back to the author.
- `quote_form.test.ts`: rows for either side, add and remove, 16 rows, form
  with only scope and amount builds equal totals with no difference line,
  the built `DealTerms` (with `after`).
- `agreement_panel.test.ts`: the buttons for each (side, kind, state); an
  ended-unconfirmed line reads as a normal end; waiting state shows "Sent.
  Waiting for the other person's node." with no button; a redraw after an action.
- `thread_redraw.test.ts`: a thread redraw from an incoming message does not
  replace an open settle dialog (`.settle-dialog`) or close list
  (`.close-list`).
- `suggest_changes.test.ts`: the compose box gets the line; nothing is
  sent.
- `statement.test.ts` and `statement_csv.test.ts`: the formula guard across
  all text columns including `counterparty_name`, quoting, decimal value,
  local offset date.
- `catalog.test.ts`, `hindi-screens.test.ts`: the new and changed ids.

Playwright, `tests/roym-deal.spec.ts`:

- D1: a quote with three commitments whose totals differ; the counterparty
  sees both totals and the difference, and agrees (AG-23, VX-2).
- D2: "I received everything" leaves out a lesson whose window is next
  week; the person checks it, and it is signed (AG-46).
- D3: a three-version quote; the summary; "Suggest changes" puts the line
  in the compose box (AG-47, AG-52).
- D4: a quote sent from a listing with only the scope typed (AG-48).
- D5: the statement downloads, CSV and JSON (AG-29).

Demo story 50, case 3 (Q10 (a)): the mixed deal, both settle in the Hub,
Alice downloads her statement. The steps wait with `DELIVERY_WAIT_MS`
(C3c R35).

### 7.7 Not tested

- AG-30 with an `obligation` sibling (M08E).
- The time of a large log (section 7.8): measured, not gated.

### 7.8 Size and time risks

- **Files near the cap.** `backup.rs` 788, `decide.rs` 736, `app.rs`
  734, `payment_ops.rs` 733, `backup/sections.rs` 732,
  `as_listed_ops/answer.rs` 744. C6a adds about 10 lines to `app.rs`
  and 5 to `payment_ops.rs`; every other line goes into new files. Do not
  touch `sections.rs`.
- **TypeScript.** `messages.ts` is 1,226 lines and `messages.test.ts`
  1,221. C6b moves the quote form out and adds its new parts in
  `quote_form/` and `deals/`, so `messages.ts` gets shorter.
- **Time of the derive step.** With the marker, a sync costs one read of
  `event_log_state` per agreement of the conversation, and the derive
  only for agreements that moved. C6a records one sync of a conversation
  with 50 closed agreements on the WASM build, as C3c1 recorded the
  driver. Stop rule: over 200 ms, stop and tell the owner.
- **Time of `statement.export` and `eventlog.list`.** Both read the whole
  log (no ordered query). C6a records `statement.export` over 1,000
  agreements on the WASM build. Stop rule: over 2 s, stop and tell the
  owner. A backlog row records the paging.
- **C7 and `messages.ts`.** C7 changes the Hub chat in `messages.ts`. C6b
  must not run beside it (task.md section 4).

### 7.9 Component size

Report all seven components, before and after, from one `target/` folder.
Expected: `transaction` +60 to +120 KB (within +15%, Q9: 2,714,637 bytes
now, limit +407,195); the other six unchanged within a few hundred bytes.
C6b changes no Rust in a component; it reports `web` only.

### 7.10 Hindi text

New messages, for the owner to review before the merge of C6b (English
here; ids prefixed `deal-` and `statement-`): "You give: { $amount }",
"You receive: { $amount }", "You give { $amount } more than you receive",
"You receive { $amount } more than you give", "I give", "They give",
"Service", "Goods", "Money", "When", "Add a line", "Remove", "More
terms", "Save as my defaults", "Version { $n }", "What changed since the
first offer", "Added", "Removed", "Changed", "Suggest changes", "About
offer version { $n }: ", "Send a new version", "I delivered", "I paid",
"Received", "You said you delivered", "How it was handed over", "I gave
everything", "I received everything", "This will sign:", "Not started
yet", "Sign", "Statement", "This month", "Last month", "This year",
"From", "To", "Show", "Download CSV", "Download JSON", "Open at the end
of the period", "Totals", "Not available on this node", "Could not be
read: { $reason }". Plus the changed texts of D-C6-15.

### 7.11 How C6 avoids the kinds of findings in earlier reviews

- **Facts in the plan that are wrong** (C1 P1, C2 P3, C3c P8, C4 P1):
  every line was checked on `edd71c11`; section 2.2 names the facts the
  design rests on, including two the agreements doc gets wrong (points 2
  and 13).
- **Missed callers** (C1 P7, C3b P2, R1, C4 R1): point 11 names both
  method lists and the count; point 18 names the test helper; the backup
  screen download is moved, not copied.
- **Silent failures** (C2 R21, R22, C4 R25): the derive step's error is
  ignored on purpose, and its test checks the sync passes and the next one
  repairs; every Hub action shows its failure and a retry.
- **Errors classed by text** (C1 R3, C3c R24): every new refusal in
  `transaction` uses `Response::invalid_params(code).with_data(json!({"code": code}))`
  where the message equals the code string and `data.code` equals that code string;
  the Hub reads `data.code`. Contract tests check both fields for every new refusal.
- **Hub state that does not match the service** (C1 R38, C2 R2): the
  panel redraws from `agreement.get`; the totals come from the signed
  terms.
- **Tests that prove less than their names** (C3a R3, R4; C3c R2, R3, R21;
  C4 R8): the period edge tests check both sides; the read test runs a
  full deal; the struct tests fail on a new field.
- **Living docs left false** (C1 R6, C3c R23, C4 R29): section 9 names the
  lines.
- **Ratchets and planning ids** (C1 R16, C3c R27, C4 R28): no suppression,
  no planning id in test names, the import clean-up pass on every new
  file.
- **Stories that time out in CI** (C3c R35): `DELIVERY_WAIT_MS`.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C6 action |
| --- | --- |
| "Hub transaction action panel settlement actions and Playwright transaction spec (cases 33-39) are not built" (`:667`) | C6b: the settlement and closing actions are built. The row keeps booking steps and payment requests, target TBD. |
| "The Hub has no action to mark work started" (`:674`) | Target from "M08C C6" to TBD: booking steps stay out of the Hub (task.md C6). |
| "The Hub's quote form builds only one service and one payment" (`:675`) | Resolved (C6b): move to "Recently resolved". |
| "More demo stories" (`:709`) | C6b extends story 50. |
| New: "`eventlog.list` and `statement.export` read the whole log" | C6a (D-C6-5). The data layer has no ordered query. Target TBD. |
| New: "A list of all deals across conversations is not in the Hub" | C6b (Q2). Target TBD. |
| New: "Times of failure and cancel events can differ between the two nodes" | C6a (D-C6-4). The statement does not use them. Target M08G (evidence may). |
| New: "A statement total above 2^53 - 1 is refused" | C6a (D-C6-7). Target TBD. |
| New: "A listing cannot say what goods or services its owner accepts in return" | Already in M08D's scope (task.md section 8); add the row only if M08D's task.md does not name it. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `AGENTS.md`, "Architecture": the Roym paragraph needs no change (it
  names no `transaction` method); check it again at close-out.
- `docs/system-architecture.md`: the `transaction` row (`:1153`) gains the
  event log and the statement; the "Closed" line (`:1264`) and the
  settlement sequence (`:1304` to `:1313`) gain `agreement.close` (C6a).
- `docs/developer-guide.md`: the `transaction` method table (near `:1821`):
  the eight new methods; the `event_log`, `event_log_state` and
  `deal_defaults` collections; the backup section and `SCHEMA_VERSION` 9
  (C6a).
- `docs/roym-integrated-experience-spec.md`: the transaction row (`:103`),
  the settlement steps C17 to C20 (`:416`, `:424`): the Hub actions and the
  one-action close; the words "offer" and "agree" for the Hub (C6b).
- `docs/system-requirements-spec.md`: `[VRT-PAY]` (`:699`): the statement
  (C6a).
- `docs/TERMINOLOGY.md`: "Event log", "Statement", "Saved deal terms"
  (C6a); "Offer" as the Hub's word for a quote (C6b).
- `docs/demo/demo.md`: R4 (`:1303` to `:1331`) says "bob presses
  Accept", with the roles reversed from story 50; fix it, and add story
  50's three cases with an "Automated:" line (C6b).
- `docs/planning/traceability-matrix.md`: `[APP-AGREE]` (`:59`) and
  `[VRT-PAY]` (`:47`) gain the new tests (the matrix uses requirement ids,
  not `AG-*`).
- Change docs (not living docs, but owned here):
  - roym-agreements `## Deviations`, a new block "Written when the M08C C6
    slice was planned": Q4 (a) (the saved terms are a `transaction` row,
    in the backup); Q5 (a) (the log is not in the bundle; the import
    rebuilds it); Q6 (a) (`counterparty_name` from the Hub); D-C6-3 (the
    marker, and an entry waits for a missing fact; counterparty entries wait
    for the author's snapshot); D-C6-4 (the times of non-settle events can
    differ; `open_at_end` uses only formed and settled); D-C6-6
    (`agreement.close`'s exact input and output, checks before signing,
    repeat returns empty records and skipped final while claims return stored
    records); D-C6-7 (`eeps` is always `not-available` in M08C; `bad-period`;
    `statement-total-overflow` at 2^53 - 1 for JavaScript safety; the export
    runs the derive step); D-C6-11 (no version comparison existed; the match
    rule); the new methods `deal.defaults-get` and `deal.defaults-set`, which
    the method table does not list; the test column of AG-23, AG-27 to AG-30,
    AG-33, AG-46 to AG-48, AG-52 (test file names); the "Tasks" checkbox for
    C6.
  - roym-value-exchange `## Deviations`: VX-45's "agreement signed" event
    is `agreement-formed`, and EEP events are M08E's; VX-46 through AG-29;
    the test columns of VX-2, VX-7, VX-45, VX-46, VX-55, VX-56.
  - [task.md](task.md): the C6 result lines, the split, section 1's size
    table.
- No ADR is edited.

## 10. Exit criteria

C6a:

- [ ] The owner answered section 2.1.
- [ ] Every C6a row of section 3 names a test that exists and passes.
- [ ] `transaction` measured and within the Q9 limit.
- [ ] The derive step's time over 50 agreements and the export's time over
  1,000 agreements recorded on the WASM build, within the stop rules.
- [ ] `roym_statement_e2e` passes (both tests).
- [ ] Parity 267 to 276 and the method contract (count 55) pass after
  `mise run build:roym`; `scenario_144` lists every `transaction` method.
- [ ] No signed record, card type or state changed (`CARD_TYPES`,
  `RECORD_TYPES` and the transition tables untouched in the diff).
- [ ] Sizes; Deviations, backlog and living docs of C6a; status.md;
  `mise run verify` with no `--skip`.

C6b:

- [ ] Every C6b row of section 3 names a test that exists and passes.
- [ ] Playwright D1 to D5 and story 50's three cases pass
  (`mise run demo`).
- [ ] Stories 40 and 60 and `roym-hub.spec.ts` pass with the new words.
- [ ] The owner reviewed the Hindi messages.
- [ ] Living docs, demo.md, change docs, status.md; `mise run verify` with
  no `--skip`.

## 11. What C6 does not do

- Booking steps (`scheduling/started`) and payment requests in the Hub.
- `service-in-slot` rows in the Hub's quote form.
- A list of all deals across conversations (Q2).
- Goods or services that a listing accepts in return (M08D).
- EEP commitments and the statement's EEP data (M08E, M08H).
- A signed "not received" (agreements doc open question 15, M08G).
- Showing a deal to a third party in the Hub (backlog row, M08G).
- A second value per commitment, a stored difference, or a stored change
  summary.
- Any WIT change, wire-reachable method, new card type or record change.
- Any change to `conversation` (Q7 (a)).

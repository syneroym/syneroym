# M08C slice C3a — Extension frame, step records and one-shot: implementation plan

*Reader: the developer (or AI) who implements slice C3a, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-20, by Claude Opus 5.5. Plan review round
1 (Claude Sonnet 5.5, [slice-c3a-review.md](slice-c3a-review.md), P1 to
P17) and round 2 (P18 to P20) are folded in. The owner answered Q1 to Q10
on 2026-11-20 (section 2.1). No question is open.**

**Size and PRs.** [task.md](task.md) sizes C3a as M. Checked against the
code, it is L: a registry and a schema checker in `roym_core`, a listing
shape change that reaches `catalog`, `directory` and the Hub, a new signed
record with its card, methods, filing, backup and Hub template, and the
generic field view. This plan splits it in two parts. Each part is one PR
that passes `mise run verify` (Q1):

- **C3a1 — Extension frame (M).** The extension manifests, the fixed
  registry, the schema check before signing (EX-2), the `insight` kind
  (EX-19), the listing names its extension in place of `BookingMode`, the
  one-shot extension (EX-18), and the generic field view in the Hub for a
  commitment's `details`. Branch: `feat/m08c-c3a-deal-extension-frame`
  (this branch; it holds the plan commit).
- **C3a2 — Step records (M).** The `step` record and card (EX-7, EX-8),
  the `step.*` methods, filing, backup, the Hub's step card on the field
  view, step names from the string catalog (the step-name half of ML-28),
  and the first step type, `scheduling/started`. Branch:
  `feat/m08c-c3a2-step-records`, cut from `main` after C3a1 merges.

**Base.** Every file path and line number below was checked on `main` at
`e11fed33` (2026-11-20). Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C3a comes after C2.
[status.md](status.md) says "Merged" for C2a (`581403ca`, PR #370) and C2b
(`535e3893`, PR #371). C3b depends on C3a (both parts), because it moves the
seat code behind the registry's reserve hook and uses step records. C3a
does not touch `conversation`, the forwarding change doc or
`docs/decisions/`, so C7 may run beside it ([task.md](task.md) section 4).
The Engage fix may run beside C3a1 only if it does not touch the listing
editor (C3a1 changes the booking mode select there; see section 4).

**Same branch, earlier commit.** This branch also holds `c0fb9e0a`, the
fix of the agreements doc review findings P61 and P62
([review](../../changes/roym-agreements/review.md), "PR #372 review round
3"). That fix changes `terms_from_listing` so that `c1.extension` is the
extension the listing names. That listing field is the one C3a1 adds
(Q3).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments or test names: no "M08C", "C3a1",
    "EX-2", "D-C3a-4", "Q3" or review ids in `.rs`, `.ts`, `.json`, `.ftl`
    files. The manifest files are code for this rule.
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines; a file in `xtask/oversized-test-files.txt` is
    capped at its recorded count. Section 7.8 lists the files this slice
    touches that are near the cap. Put new code in new files.
  - **Function size.** `clippy::too_many_lines` at 100. Add no suppression.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): section 1 (Z3, Z4 and the component size limit),
  "C3a", "C3b", section 4, section 5 (EX-19), section 7.
- [roym-extensions](../../changes/roym-extensions/change.md): EX-1 to EX-3,
  EX-7, EX-8, EX-18, EX-19, "Extension manifest", "Commitment details",
  "Step records", "Scheduling and one-shot", "Deviations".
- [roym-agreements](../../changes/roym-agreements/change.md): "Resource
  kinds and commitment kinds" (`:422`), "The `details` block" (`:435`),
  "What is hidden" (`:776`), "Card types" (`:1009`), open question 3
  (`:1109`), the `terms_from_listing` mapping table (`:258` to `:279`).
- [roym-multilingual](../../changes/roym-multilingual/change.md): ML-28
  (`:135`), the rule "Commitment kinds and extension fields" (`:220`), the
  catalog rules (`:165` to `:190`), "Deviations" (`:66`).
- The C2 plan ([slice-c2-implementation-plan.md](slice-c2-implementation-plan.md))
  for the layout and for `conceal_hidden` and `reveal_hidden_strict`, which
  C3a2 reuses.
- The C1 and C2 review files. Section 7.11 says how this plan avoids the
  kinds of findings they hold.

### Terms used in this plan

- **Roym extension:** a package that adds work steps or data to a deal. It
  plugs into the deal core and never changes its states (extensions doc,
  "The rule").
- **Manifest:** one data file per extension. It names the extension, its
  kind, its `details` schemas, its step types and its hooks.
- **Registry:** the fixed list of manifests that ship with this build. It
  is compiled into `roym_core`. There is no runtime install.
- **Schema:** a named, versioned list of typed fields, for example
  `scheduling/slot@1`. A commitment's `details.data` and a step's `data`
  are checked against one.
- **Known extension:** an extension whose `(id, version)` is in this
  node's registry. **Unknown extension:** any other. Every node of one
  build knows the same extensions, so "unknown" happens only between two
  builds.
- **Field view:** the Hub's generic drawing of a schema's fields as
  labelled values (task.md Z4).
- **Step record:** a signed record of one work step of an extension inside
  a formed agreement (EX-7). Its card type is `step`.

## 1. What C3a builds, in one paragraph

Today the only extension is a fixed case in code: `deal.rs` knows
`scheduling` version 1 and the schema `scheduling/slot@1`, and refuses
every other extension with `commitment-kind-not-supported`
([deal.rs:349](../../../../crates/roym_core/src/deal.rs#L349) to `:472`).
The same rule is repeated in `SignedTerms::validate_visible`
([deal/sd.rs:264](../../../../crates/roym_core/src/deal/sd.rs#L264)) and in
the presentation checker
([presentation/validate.rs:420](../../../../crates/roym_core/src/presentation/validate.rs#L420)
to `:445`). A listing says how it is booked with `BookingMode` (`slots`,
`order`, `enquiry`,
[listing.rs:53](../../../../crates/roym_core/src/listing.rs#L53)), and no
deal code reads it. C3a1 replaces the fixed case with one registry of
manifests (`scheduling`, `one-shot`), a schema checker, and the EX-2 rule
for an unknown extension. A listing names its extension by `id` and
`version`. The Hub draws a commitment's `details` with labels from the
string catalog. C3a2 adds the `step` record: a party signs one work step
of a commitment (in C3a2 only `scheduling/started`, the giver's "work
started"), the card travels to the other party, and both Hubs show it with
its name in the reader's language. No deal state changes because of a step
(extensions doc, rule 1).

### Why C3a is split (Q1)

- C3a1 changes the shape of the listing record (`catalog`, `directory`,
  the Hub's listing editor and directory screen) and every check of a
  commitment's extension. C3a2 adds a new signed record to `transaction`
  with its whole pipeline: sign, store, send, file, verify, back up, view.
  The two have different reviewers' questions: C3a1 is "is every caller of
  the old rule moved, and does an unknown extension stay safe"; C3a2 is
  "is the record made once, filed safely, and hidden where it must be".
- C3a2 needs C3a1's registry (step types are declared in a manifest), and
  C3a1 needs nothing from C3a2. So the order is fixed, and each PR leaves a
  working system.
- Sizes: C3a1 is about 900 lines of production code and 1,000 of tests;
  C3a2 about 700 and 900 (estimates from the nearest existing code:
  `fulfilment_ops.rs` is 580 lines, `taxonomy.rs` plus its tests about
  500). Together this is larger than C2a, which was split for the same
  reason.

## How this is shown to an end user

C3a1:

- A shop owner opens "Listings" and edits a listing. The choice that says
  how the listing is booked shows the same three options as today:
  "slots", "order" and "enquiry".
- A person opens a quote in the chat. If the quote has extra terms from an
  extension, the card shows them as labelled lines, with the labels in the
  person's language. For a slot quote these are two reference codes: the
  listing's and the slot's. Today only a quote made with `roymctl` has
  them; the Hub's quote form adds none (Q4).
- If the quote names an extension that this person's Roym does not know,
  the card shows its lines as they came and says "not checked on this
  device". The person may still accept, unless they are the one who gives
  that part of the deal. Then the accept button shows "This deal needs a
  part of Roym that this device does not have. Update Roym, then accept."
  (Q9).
- Nothing else looks different. An order is still an order.

C3a2:

- After a slot booking is agreed, the person who gives the service marks
  "work started". In C3a2 this is a `roymctl` command, not a Hub button
  (Q6).
- Both people see a new card in the chat: "Work started", in their own
  language, with who marked it and when.
- The demo story `60-roym-step` shows this: Bob accepts a slot booking in
  his Hub, Alice runs the command, and both Hubs show the card (Q10).

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core extension` runs the registry,
  manifest and schema tests; each manifest in
  `crates/roym_core/extensions/` loads and every schema check has a
  passing and a failing case.
- `cargo nextest run -p syneroym-substrate --test roym_step_e2e` (C3a2)
  runs two nodes: the giver adds `scheduling/started`, and the other
  node's `step.list` shows it with the plain `data` and `verified: true`.
- `roymctl roym step add --agreement <rec_…> --commitment c1 --type
  scheduling/started` (C3a2) prints the signed record id and the send
  state.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for
the recommended answer.

**Owner answers, 2026-11-20.** Q1 (a). Q2 (a), and task.md Z3 is updated
in the same commit (P17). Q3 (a), with the answers to P13, P14 and P15
(D-C3a-9 and section 4). Q4 (a); `roymctl` cannot make a one-shot quote
today, and C3a does not add a flag (P7, Q4 below). Q5 (a), with a size
limit on step `data` (P4, D-C3a-10). Q6 (a); the row key holds the role
(P5, D-C3a-11). Q7 (a) for C3a1 only; for C3a2 the owner does not accept
`none` without an explicit yes, so C3a2's demo is the new question Q10.
Q8 (a), plus a field type `code` for identifiers (P6, D-C3a-2). Q9 (a):
keep the `agreement.accept` refusal for a giver, and record the deviation
(P11; written in the extensions doc in the review commit). Q10 (a): C3a2
adds the demo story `60-roym-step`.

#### Q1. Split C3a into C3a1 (frame) and C3a2 (step records)?

- **(a) Split, as above.** Two PRs, C3a1 first. Recommended: the two parts
  have different risks, and each one alone is about the size of C2b.
- (b) One PR. One review of about 1,600 lines of production code and
  1,900 of tests, across five crates and the Hub.

If (b): sections 6.1 and 6.2 run on one branch, and the exit criteria are
joined.

#### Q2. Format and place of the manifest files

Open question 3 of the agreements doc chose "a TOML manifest per extension,
in the extension's crate, read at build time". Two facts make that
expensive here. First, `toml` is only a dev-dependency of `roym_core`
([Cargo.toml](../../../../crates/roym_core/Cargo.toml)); reading TOML at run
time would add a TOML parser to every Roym component, and the component
size limit applies ([task.md](task.md) section 1). Second, the Hub needs
the same schemas to draw the field view, and it cannot read a file that a
build script writes to `OUT_DIR`. The repository already has the pattern
for a data file that Rust and the Hub share: the base taxonomy is JSON in
`roym_core` (`include_str!` at
[taxonomy.rs:19](../../../../crates/roym_core/src/taxonomy.rs#L19)), and
the Hub imports the same file
([taxonomy.ts:1](../../../../crates/roym_web/ui/src/taxonomy/taxonomy.ts#L1)).
Also, one-shot has no crate, and the scheduling crate is created only in
C3b; `roym_core` cannot depend on an extension crate.

- **(a) JSON files in `crates/roym_core/extensions/<id>.json`, read with
  `include_str!` and `serde_json`, and imported by the Hub.** Exactly the
  taxonomy pattern. No new dependency, no build script, one file that both
  sides read. Recommended. It changes open question 3 in two words ("JSON",
  "in `roym_core`"); the deviation is recorded in the agreements doc.
- (b) TOML files, and a `build.rs` in `roym_core` that converts them to
  JSON in `OUT_DIR`. Keeps TOML. The Hub then needs its own copy (a
  checked-in generated file and a test that the two agree), and the build
  gets a build script.
- (c) TOML parsed at run time. Adds the `toml` crate to every component.

#### Q3. How does a listing name its extension?

Today `BookingTerms.mode: BookingMode`
([listing.rs:59](../../../../crates/roym_core/src/listing.rs#L59) to `:66`).
No deal code reads it: the only readers are `ListingPayload::validate`
(payment may be absent only for `enquiry`, `:315`), the directory's search
index and query filter
([search_ops.rs:123](../../../../crates/roym_directory/src/app/search_ops.rs#L123),
`:296`; [directory.rs:172](../../../../crates/roym_core/src/directory.rs#L172)),
and the Hub's listing editor
([editor.ts:9](../../../../crates/roym_web/ui/src/listings/editor.ts#L9)).

- **(a) A top-level `extension: ExtensionRef | absent` on `ListingPayload`.
  `BookingMode` and `BookingTerms.mode` are removed.** `slots` becomes
  `{ id: "scheduling", version: 1 }`, `order` becomes
  `{ id: "one-shot", version: 1 }`, and `enquiry` becomes no extension (the
  plain enquiry flow, extensions doc "Scheduling and one-shot"). The
  directory's `booking_mode` filter and index column become `extension`
  (the id). `listing.set` refuses an extension this node does not know.
  Recommended: open question 3 says "an offering names its extension by
  `id` and `version`", and `terms_from_listing` (fixed in `c0fb9e0a`) now
  reads this field.
- (b) Keep `BookingMode`, and map it to an extension in one `roym_core`
  function. No listing shape change. Two names for one fact, and a later
  extension needs a new enum value in a signed record.

One cost of (a): the directory can no longer ask for "plain enquiry
listings only". `SearchQuery.extension` filters by an extension id, and
"no extension" is not a value it can ask for (P14). No screen asks for it
today.

#### Q4. Does one-shot need a service, and what does its commitment carry?

- **(a) Manifest only, no service.** One-shot has nothing to reserve and
  no steps in C3a. Its commitment is `goods-or-service` with extension
  `one-shot` version 1 and `details` `{ extension: "one-shot", schema:
  "one-shot/order@1", data: {}, extra: [...] }`. The agreements doc
  requires `details` whenever a commitment names an extension (`:449`), and
  `extra` is where a party adds free terms (EX-3). It settles with a
  payment acknowledgement and a fulfilment receipt, as today. Recommended.
- (b) A one-shot service with its own methods. Nothing in M08C would call
  them.

Who sets `one-shot` on a commitment in C3a: only tests that call
`quote.set` with hand-written terms. `roymctl roym transaction quote`
cannot: `build_quote_commitments`
([transaction.rs:355](../../../../apps/roymctl/src/commands/roym/transaction.rs#L355)
to `:368`) makes a commitment with no extension unless `--slot` is given,
and C3a adds no `--one-shot` flag. A flag would be a new visible command
for a record that no person makes yet. The first real caller is C3c's
`terms_from_listing`, for a listing that names `one-shot`. The Hub's quote form today builds
no extension unless it has a slot, and it never fills a slot either
([messages.ts:1009](../../../../crates/roym_web/ui/src/screens/messages.ts#L1009)
passes no `listingId` or `slotId` to `buildDealTerms`). Filling the quote
form from the listing is C6's AG-48. So in C3a the Hub's quote form does not
change, and a Hub quote has no extension, as today.

#### Q5. Is a step's `data` hidden?

"What is hidden" in the agreements doc (`:780`) says: hidden always are
"the free text, the offering, every declared value, details, window,
location and every payment detail". A step's `data` is the same kind of
content as `details` (for a later extension: a tracking number, a
document name). A step is not part of a presentation in M08C, but M08G
evidence may point at one (EX-9).

- **(a) `data` is one hidden field, like `details`.** The record holds one
  `_sd` digest. The card carries one disclosure. Both nodes store it in the
  `disclosures` collection and back it up. The C2 helpers do all of it:
  `conceal_hidden` and `reveal_hidden_strict`
  ([disclosure.rs:431](../../../../crates/roym_core/src/disclosure.rs#L431),
  `:456`) with a new `STEP_SET`, as the fulfilment receipt does with
  `FULFILMENT_SET` (`:105`). Recommended: one rule for every deal record,
  and no shape change when M08G needs it. `data` has a size limit,
  `MAX_STEP_DATA_BYTES` (8 KiB, the same as a `details` disclosure), and an
  agreement holds at most `MAX_STEPS_PER_AGREEMENT` (64) step rows
  (D-C3a-10, P4).
- (b) `data` visible. Less code (about 80 lines), but the first record of
  the deal family with plain extension data, and a shape change in M08G.

#### Q6. The first step type, and who can make it

C3a2 needs one real step type, or the record has no use and the two-node
test needs a test-only extension in the shipped registry. Today nobody can
mark work as started: `booking.start` was removed in C1a with no stand-in
(backlog row "Nobody can mark work as started",
[deferred-backlog.md:665](../../deferred-backlog.md), target C3b), and the
agreements doc says `in-progress` becomes a scheduling step (`:621`).

- **(a) The scheduling manifest declares `scheduling/started`, made by the
  commitment's giver, with no fields. `step.add` makes it, and `roymctl
  roym step add` calls it. The Hub shows the card, but has no button.**
  Recommended. It closes the backlog row for `roymctl` users, it is the
  step C3b would add anyway, and a Hub button belongs with the deal action
  panel (C6 adds the settlement actions there, task.md question 2). The
  step is a card, not a state: it does not move the commitment to
  `in-progress` or change the progress snapshot.
- (b) As (a), plus a "Work started" button on a slot agreement in the Hub.
  The Hub has no screen for a slot agreement's actions today, so this is a
  new panel.
- (c) No real step type. The step tests use a manifest that only tests
  load. The two-node test then needs a test build of the registry.

#### Q7. Demo

The Mandatory Demo Close-out asks for a story when a viewer can see
something new. Hub quotes have no extension (Q4), so the field view shows
nothing in today's stories. A step can be made only with `roymctl` (Q6).

- **(a) C3a1: `- none: no visible change; the listing editor keeps its
  three choices and a Hub quote carries no extension`. C3a2: `- none: a step
  is made only from roymctl`.** The step card is covered by the Hub unit
  tests and the two-node test. Recommended. C3b (slot booking through the
  extension) and C6 (the deal action panel) are the slices whose stories
  can show the field view and a step card.
- (b) C3a2 adds a story that runs `roymctl` from the test to make the step,
  then shows the card in the Hub.

**Answer:** (a) for C3a1. For C3a2 the owner does not accept `none`
without an explicit yes: AGENTS.md ("Mandatory Demo Close-out") counts a
new command as something a viewer can see, and C3a2 adds `roymctl roym
step add`. The first version of this plan also said that a demo story
drives only the Hub. That is wrong: stories 10, 20 and 30 have a shell part
that runs `roymctl` (`demo/lib.sh:49`, `rc1` and `rc2`), and a browser
part can run a shell part with `runScript` (`demo/steps.ts:15`). So option
(b) is possible. Q10 asks which one C3a2 does.

#### Q8. Which field types are built now?

The extensions doc lists ten field types: text, long text, number, money,
date-time, duration, choice, location, file pointer, record pointer.

- **(a) The seven scalar types: `text`, `long-text`, `number`, `money`,
  `date-time`, `duration`, `choice`.** Each is a JSON string or integer
  with a limit, and each has a simple Hub view. `location`, `file-pointer`
  and `record-pointer` need their own rules (an address and its
  disclosure, a blob reference, a record lookup) and no M08C manifest uses
  them. A manifest that names one fails to load. A backlog row records
  them. Recommended.
- (b) All ten now.

**Answer:** (a), plus an eighth type, `code`, for identifiers such as a
listing id or a slot id (P6). A `code` value is not free text: it has no
language and is drawn as plain data. The scheduling fields `listing_id`
and `slot_id` use it (D-C3a-1, D-C3a-2).

#### Q9. Can a giver accept a deal part whose extension its node does not know? (answered)

EX-2 says a party without the extension "may still sign". D-C3a-6 makes
`agreement.accept` refuse with `extension-unknown` when this node gives a
commitment whose extension it does not know (P11).

- **(a) Keep the refusal for a giver, and record the deviation.** A
  receiver may still accept. A giver must run its extension's hooks and
  steps, and its node cannot. Recommended.
- (b) Follow EX-2 to the letter: any party may accept. The giver's node
  then holds an agreement whose work it cannot record.

**Answer:** (a). The deviation is in the extensions doc.

#### Q10. The demo story of C3a2 (answered)

- **(a) A new story `60-roym-step`: a browser part
  `demo/60-roym-step.spec.ts` and a shell part `demo/60-roym-step.sh`.**
  Alice publishes a listing with one slot in her Hub (the listing editor
  and its slot list, `listings.ts:612`, already do this). The shell part
  runs `rc1 roym transaction quote … --listing <id> --slot <id>` to send
  Bob a slot quote. Bob accepts it in his Hub, as in story 50. The shell
  part runs `rc1 roym step add --agreement <rec_…> --commitment c1 --type
  scheduling/started`. Both Hubs then show the "Work started" card.
  Recommended: it shows the new command and the new card, which is what
  the rule asks for, and it reuses the parts that exist. Cost: about 150
  lines of story, and one more story in `npm test` (about 1 minute).
- (b) `- none: a step is made only from roymctl; C3b's story shows the step
  card`, with the owner's explicit yes. C3b's plan then must carry a story
  that shows the step card.

**Answer:** (a). One more cost, found in review round 2: `roymctl` has no
command that reads a listing's slots, and `quote --slot` needs the slot id
and the window (`transaction.rs:341` to `:364`). The Hub makes the slot
from local date and time fields. So the story uses this order:

1. The browser part makes Alice's listing and one slot in her Hub, with a
   fixed window that it computes (for example tomorrow, 10:00 to 11:00 in
   the browser's time zone). It reads the listing id from the Hub.
2. It sets `DEMO_STEP_LISTING_ID`, `DEMO_STEP_SLOT_START` and
   `DEMO_STEP_SLOT_END` in `process.env`, then calls
   `runScript('60-roym-step-quote.sh')`. `execFileSync` passes
   `process.env` to the script. The script derives nothing: it passes the
   window to `quote --slot` as it is.
3. Bob accepts the quote in his Hub, as in story 50. The browser part reads
   the agreement record id with `agreement.list`, as story 50's helper does
   (`demo/50-roym-deal.spec.ts:25` to `:38`; move it to `steps.ts` or
   `hub-helpers.ts` so both stories use one helper), and sets
   `DEMO_STEP_AGREEMENT_ID`.
4. `runScript('60-roym-step-start.sh')` runs `rc1 roym step add`.
5. Both Hubs show the "Work started" card.

`runScript` takes no arguments, so the two shell steps are two files. This
adds a few dozen lines to the estimate above.

### 2.2 Points found while reading

1. **task.md's facts are older than the code.** Section 2 says
   `SCHEMA_VERSION` of `transaction` is 4; it is 6
   ([app.rs:54](../../../../crates/roym_transaction/src/app.rs#L54)). The
   card table has 8 types, not 7
   ([card.rs:11](../../../../crates/roym_core/src/card.rs#L11) to `:20`;
   the doc comment at `:8` says "eight"). No action: task.md section 2 is a
   record of its own date.
2. **No deal code reads `BookingMode`.** The quote's extension is chosen by
   whoever builds the terms. So Q3 changes a listing field and the
   directory filter, and no deal rule.
3. **The Hub never draws `details` or `extension`.** `renderQuote`
   ([quote.ts:99](../../../../crates/roym_web/ui/src/cards/templates/quote.ts#L99))
   draws only the description, the giver line and the declared value of
   each commitment (`:198` to `:237`). The field view is new UI, not a
   rewrite of a template.
4. **The Hub's quote form never makes a slot quote.** `buildDealTerms`
   ([quote_terms.ts:52](../../../../crates/roym_web/ui/src/screens/quote_terms.ts#L52)
   to `:59`) can build scheduling `details`, but `readQuoteForm`
   ([messages.ts:947](../../../../crates/roym_web/ui/src/screens/messages.ts#L947),
   the call at `:1009`) never passes `listingId`, `slotId` or `schedule`.
   Slot quotes come only from `roymctl` and tests. This plan keeps
   `buildDealTerms`'s slot branch and moves its two constants to the
   registry module (D-C3a-12).
5. **The scheduling rule is repeated in three places.**
   `validate_commitment_kind` and `validate_scheduling_slot_details`
   ([deal.rs:349](../../../../crates/roym_core/src/deal.rs#L349),
   `:415`), `SignedTerms::validate_visible` (`deal/sd.rs:264`), and the
   presentation checker (`presentation/validate.rs:420` to `:445`). C3a1
   makes all three call one registry function (D-C3a-5).
6. **A counterparty refuses a quote with an unknown extension today.**
   `file_quote_card` runs `plain_terms.validate()` and refuses the card on
   error ([sync.rs:526](../../../../crates/roym_transaction/src/app/sync.rs#L526)).
   EX-2 says such a quote is shown "not checked on this device", and the
   person may still sign. This is a behaviour change (section 4).
7. **Listings with `mode: enquiry` and no payment are used in tests.** The
   trust harness builds such listings
   (`crates/roym_web/tests/dual_build_parity/trust_harness.rs:187` to
   `:192`, `:544` to `:549`). They rely on the payment exemption at
   `listing.rs:315`. Under Q3 (a) they become listings with no extension,
   and the same exemption holds (D-C3a-9).
8. **`full_listing_params` is shared by about 78 call sites.** It has
   `"mode": "slots"` (`crates/roym_web/tests/dual_build_parity/fixtures.rs:97`).
   One change there moves them all; the method shape
   `listing_verify_good.json` (`:7` to `:12`) changes with it.
9. **No demo story makes an extension.** Story 50 creates no listing and
   no slot (`demo/50-roym-deal.spec.ts`); story 40 publishes a listing with
   no booking block (`hub-helpers.ts:115` to `:144`). So no story changes
   in C3a1. (The first version of this point said that stories drive only
   the Hub. Stories 10, 20 and 30 also have a shell part that runs
   `roymctl`; see Q7 and Q10.)
10. **The backup re-verify match has no `deal-decline` arm.** On import,
    `transaction.import` re-verifies each card by kind
    ([backup.rs:735](../../../../crates/roym_transaction/src/app/backup.rs#L735)
    to `:748`); `deal-decline` falls to `_ => false`. C3a2 adds a `step`
    arm to the same match. It also checks with a test whether a
    `deal-decline` card is marked not verified after an export and import.
    If it is, C3a2 adds the arm (one line, `decline::verify`), with that
    test. If the test shows another path covers it, the plan records that.
11. **The file-length gate checks `.rs` files only**
    (`xtask/src/file_lengths.rs:398`). `messages.ts` is 1,069 lines. C3a1
    adds one `else if` branch to its accept error handler
    (`messages.ts:686` to `:690`, for `extension-unknown`, Q9); all other
    new Hub code goes in new files.
12. **Two backlog rows name the wrong slice.** "Widget labels and extension
    step names do not come from the catalog yet" says target C4
    ([deferred-backlog.md:574](../../deferred-backlog.md)); task.md moved
    step names to C3a. "Nobody can mark work as started" says C3b (`:665`);
    under Q6 (a), C3a2 closes it for `roymctl`. Section 8 says how each row
    changes.

## 3. What C3a meets

| Change doc | Criterion | Part | Where it is proved (section 7) |
| --- | --- | --- | --- |
| roym-extensions | EX-1 | C3a1: `details` is checked against the registry's schema, not a fixed case. (C1a built the shape.) | unit `extension::schema` tests; service `quote_set_refuses_details_that_fail_the_schema` |
| roym-extensions | EX-2 | C3a1: the author's node refuses an unknown extension; the counterparty files it, the Hub marks it "not checked on this device", and `agreement.accept` works unless this node gives that commitment; the writer's check binds; a presentation lists it in `not_checked` | service `counterparty_files_a_quote_with_an_unknown_extension`, `accept_refuses_when_this_node_gives_an_unknown_extension`; Hub `fields.test.ts`; unit presentation case |
| roym-extensions | EX-3 | C3a1: `extra` is drawn by the field view (C1a built the shape and limits) | Hub `fields.test.ts` |
| roym-extensions | EX-7 | C3a2: the `step` record; either party verifies it (`step.verify`) | unit `step` tests; two-node `roym_step_e2e` |
| roym-extensions | EX-8 | C3a2: the `step` card; the Hub draws a known step with its catalog name and fields, and an unknown one as labelled text | Hub `templates.test.ts` (step); two-node |
| roym-extensions | EX-18 | C3a1: the one-shot extension; an agreement with a one-shot commitment settles with a payment acknowledgement and a fulfilment receipt. The "agree as listed" half is C3c's | service `one_shot_commitment_settles_like_today` |
| roym-extensions | EX-19 | C3a1: `kind = "insight"` and `reads` in the manifest format only; no insight manifest ships; a commitment or listing cannot name one | unit `insight_manifest_loads_and_cannot_be_named` |
| roym-multilingual | ML-28 | C3a1: `details` field labels; C3a2: step names. Both from the string catalog, English fallback | Hub `catalog.test.ts` (manifest ids), `fields.test.ts`, `templates.test.ts`; `hindi-screens.test.ts` |

Not met here: EX-4 (one test with scheduling and one-shot through the same
states) is C3b's, because the scheduling extension is moved there. EX-9 is
M08G. Criteria of the agreements doc: none. `terms_from_listing` uses the
listing's `extension` field (C3c builds the function).

## 4. Behaviour-change audit

### Wire changes

- **Listing record (C3a1).** `ListingPayload` gains `extension: { id,
  version }` (absent for the plain enquiry flow). `BookingTerms` loses
  `mode`. This changes the signed `listing` record that strangers verify,
  and the `listing.set` input. `catalog`'s `SCHEMA_VERSION` goes from 3 to
  4 ([app.rs:23](../../../../crates/roym_catalog/src/app.rs#L23)). That
  bump gates only the catalog backup import. A stored or held listing of
  the old shape still parses, and has no extension (D-C3a-9, "An old
  stored listing"). No migration, by the owner's pre-release rule.
- **Directory search (C3a1).** `SearchQuery.booking_mode` becomes
  `extension: Option<String>` (an extension id)
  ([directory.rs:172](../../../../crates/roym_core/src/directory.rs#L172)).
  `directory.search` is wire-reachable, so this is a change on the wire
  between nodes. The index column `booking_mode` becomes `extension`
  (`search_ops.rs:58`, `:123`, `:140`, `:171`, `:296`).
  `DIRECTORY_SCHEMA_VERSION` goes from 5 to 6
  ([directory.rs:19](../../../../crates/roym_core/src/directory.rs#L19)).
  A held publication of the old listing shape parses and is indexed with
  no extension, as in the catalog. No migration; development nodes start
  empty. `rebuild_search_index` (`search_ops.rs:539`) needs no change. The
  query value is checked as an id (D-C3a-9, P14). An old node that gets
  `extension` in a query ignores it (no `deny_unknown_fields` on
  `SearchQuery`), so it answers without that filter.
- **Quote records (C3a1).** A `goods-or-service` commitment may name
  `one-shot` version 1, with `details` of schema `one-shot/order@1`.
  Before, any extension on `goods-or-service` was refused.
- **New error codes (C3a1).** `extension-unknown` (`quote.set`,
  `listing.set`, `agreement.accept` when this node gives the commitment),
  `details-invalid` (a field fails its schema), and the listing code
  `booking-required` (D-C3a-9). `slot-details`, `details-mismatch` and
  `extension-shape` stay, with the same meaning; `extension-shape` now
  also covers `extra` of every extension (P2).
- **New error codes (C3a2).** `step-type-unknown`, `not-your-step`,
  `step-data-too-large`, `too-many-steps` (D-C3a-11).
- **New record and card (C3a2).** Record type `step` version 1 in
  `RECORD_TYPES`; card type `step` version 1 in `CARD_TYPES` (Rust and Hub).
  The card carries one disclosure (Q5).
- **New methods (C3a2).** `step.add`, `step.list`, `step.verify`,
  `step.ping` on `transaction`, all local-only through
  `admit::require_internal`
  ([app.rs:275](../../../../crates/roym_transaction/src/app.rs#L275)).
- **Backup bundle (C3a2).** A new section `steps`. Its disclosures go in
  the existing `disclosures` section.

### Behaviour changes

- **A quote with an unknown extension is filed by the counterparty**
  (C3a1), not refused (section 2.2, point 6). The Hub marks its `details`
  "not checked on this device".
- **A presentation that names an unknown extension is verified** (C3a1).
  The commitment's details are listed in `not_checked`, as a missing
  window already is
  ([validate.rs:425](../../../../crates/roym_core/src/presentation/validate.rs#L425)).
  Before, `validate_visible` refused the whole presentation.
- **`listing.set` refuses an unknown extension** (C3a1). Before, any of
  three modes was accepted.
- **A listing with no extension may leave out `payment`** (C3a1). This is
  today's `enquiry` rule. One case widens: a listing with no `booking`
  block at all needed `payment` before (`booking_is_enquiry` is false when
  `booking` is absent, `listing.rs:315`), and needs none now. Recorded as a
  deviation in the extensions doc.
- **The Hub's quote card shows `details`** (C3a1), with the field view.
- **A `step` card is filed and shown** (C3a2). It does not change any
  agreement or commitment state.

### Stays the same

- Every deal state, transition, reason code and settlement rule.
- The seat code. It stays in `transaction` until C3b, and C3a does not
  change it: every site still matches `ResourceKind::ServiceInSlot` and
  reads `details.data.slot_id`. For scheduling, the only extension with a
  reserve hook, the result is the same as asking the registry. The first
  version of this plan moved one of these sites to `has_reserve_hook`;
  that would give two answers to one question in one slice (P12). C3b
  moves all of them behind the reserve hook. The sites, for C3b's plan:
  - `reserve_counterparty_slots`
    ([agreement_ops.rs:267](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L267));
  - `reserve_author_holds` (`decide.rs:427` to `:436`);
  - the release at expiry (`agreement_ops.rs:513`);
  - the `fulfil:` kind check on import (`backup/sections.rs:373`);
  - `validate_and_apply_slot` (`quote_ops.rs:150`, and `:427` reads the
    slot from `catalog`).
  `Manifest::has_reserve_hook` is still built in C3a1, because rule 6 of
  D-C3a-5 and the loader use it.
- The hidden fields of the quote and of every commitment. `details` stays
  one hidden field.
- The Hub's quote form, enquiry form and the chat. Story 40 and story 50
  pass unchanged.
- The listing editor's visible choices and their messages
  (`listing-mode-slots`, `listing-mode-order`, `listing-mode-enquiry`,
  [en.ftl:447](../../../../crates/roym_web/ui/src/i18n/en.ftl#L447) to
  `:449`). Only the value sent changes.
- `availability.*` in `catalog`, for any listing, as today.
- `conversation`, `profile`, `web`'s Rust code, the WIT interfaces.

## 5. Decisions this plan makes

**D-C3a-1. The manifest shape** (Q2 (a)). One JSON file per extension:

```json
{
  "id": "scheduling",
  "version": 1,
  "kind": "workflow",
  "resource_kinds": ["service-in-slot"],
  "hooks": ["reserve", "confirm", "release"],
  "details": [
    {
      "schema": "scheduling/slot@1",
      "fields": [
        { "name": "listing_id", "type": "code", "max_len": 128, "required": true },
        { "name": "slot_id", "type": "code", "max_len": 128, "required": true }
      ],
      "checks": ["slot-matches-window"]
    }
  ],
  "steps": [
    { "type": "scheduling/started", "by": "giver", "fields": [] }
  ]
}
```

```json
{
  "id": "one-shot",
  "version": 1,
  "kind": "workflow",
  "resource_kinds": ["goods-or-service"],
  "hooks": [],
  "details": [ { "schema": "one-shot/order@1", "fields": [] } ],
  "steps": []
}
```

- `kind`: `workflow` or `insight`. An `insight` manifest has `reads` (a
  list of codes, for example `event-log`) and no `resource_kinds`,
  `hooks`, `details` or `steps` (EX-19). A `workflow` manifest has no
  `reads`.
- `resource_kinds`: the commitment kinds that may name this extension. This
  replaces the hand-written cases in `validate_commitment_kind`.
- `hooks`: `reserve`, `confirm`, `release`, all three or none. The
  agreements doc requires a reserve hook for `service-in-slot` (`:426`); the
  loader checks that.
- `checks`: names of built-in checks in `roym_core` that a field list
  cannot say. C3a has one: `slot-matches-window`, today's
  `validate_scheduling_slot_details` (`deal.rs:415`). A manifest that names
  an unknown check fails to load.
- `steps[].by`: `giver`, `receiver` or `either`, relative to the step's
  commitment. A step type happens at most once per role: with `either`,
  each party may record it once (D-C3a-11).
- `widgets`: not in C3a. C4 adds it. The loader refuses unknown keys
  (`deny_unknown_fields`), so a manifest cannot carry a part that no code
  reads.
- `version` is a number, as `ExtensionRef.version: u32` already is
  (`deal.rs:115`). The extensions doc's sketch has a string; this is
  recorded with Q2.

**D-C3a-2. Field types** (Q8 (a)). Each field has `name` (`[a-z0-9_]{1,32}`),
`type`, `required` (default false) and the type's limits:

| Type | JSON value | Limits | Hub view |
| --- | --- | --- | --- |
| `text` | string | `max_len` bytes, required, at most 512; not empty when required | free text in the record's language (see below) |
| `long-text` | string | `max_len` bytes, required, at most 2048 | as `text`, line breaks kept |
| `code` | string | `max_len` bytes, required, at most 128; only `[A-Za-z0-9._:-]`; not empty | the value as plain data, with no language |
| `number` | integer (i64) | optional `min`, `max`; `min` at most `max` | the number in the reader's format |
| `money` | integer minor units, at least 0 | in the deal's currency | `formatMoney` with the deal's currency |
| `date-time` | integer seconds since 1970 | | the date and time in the reader's format |
| `duration` | integer seconds, at least 0 | | hours and minutes |
| `choice` | string | one of `choices`: not empty, each a code `[a-z0-9-]{1,32}`, no repeats | the choice's catalog label |

Every number is an integer: the agreements doc's `details` block says
"integers only" (`:443`). `data` must be a JSON object. A key not in the
field list is refused (`details-invalid`). `Registry::parse` checks every
limit in this table (D-C3a-4), so a manifest cannot defeat the card size
rules with, for example, `max_len: 100000`.

**Free text in `details`.** A `text` or `long-text` value and each `extra`
label and value are free text inside a quote. The quote has one `lang`
tag, and it covers every free-text field of the record (roym-multilingual,
"Language-tagged free text"). So the Hub draws them with the quote's
original `lang` as their `lang` attribute, never with a translation's tag.
The Hub's machine-translation action covers the quote's `scope` and terms
texts today, not `details`; C3a does not add `details` to it. A backlog
row records this (section 8). `code`, number and choice values are not
free text.

**D-C3a-3. Message ids are derived, not written in the manifest.** As in
the taxonomy (`taxonomy-<kind>-<code>`, `taxonomy.ts:69` to `:90`), the
Hub derives ids from codes. The exact function, the same in Rust (for the
load check) and in the Hub:

- Let `short` be the schema name without its `<extension id>/` prefix and
  its `@<n>` suffix. For `scheduling/slot@1`, `short` is `slot`.
- A field label: `ext-<extension id>-<short>-<field>`.
- A choice: `ext-<extension id>-<short>-<field>-<choice>`.
- A step name: `ext-<extension id>-step-<step>`, where `step` is the step
  type without its `<extension id>/` prefix.
- In every part, `_` becomes `-`. Example: `ext-scheduling-slot-listing-id`,
  `ext-scheduling-step-started`.

The schema version is not part of the id. So `slot@1` and `slot@2` share
the label of a field with the same name. The rule for a manifest author: a
new schema version that changes what a field means gives it a new name.

Derived ids can collide (extension `a-b` with schema short name `c`, and
extension `a` with short name `b-c`, both give `ext-a-b-c-…`).
`Registry::parse` builds every derived id of every manifest and refuses a
set in which one id comes from two places.

A catalog test checks that `en.ftl` has exactly the ids the manifests
imply, and `hi.ftl` the same (the pattern of `catalog.test.ts:226` to
`:234`). The messages go in a new section `## Extensions` of `en.ftl` and
`hi.ftl`. No new catalog file and no loader change.

**D-C3a-4. The registry module.** `crates/roym_core/src/extension.rs`
(with `extension/schema.rs` and `extension/tests.rs`):

```rust
pub const MANIFESTS: &[&str] = &[
    include_str!("../extensions/scheduling.json"),
    include_str!("../extensions/one-shot.json"),
];

pub struct Registry { manifests: Vec<Manifest> }

/// The manifests that ship with this build, parsed once.
#[expect(clippy::expect_used, reason = "the shipped manifests are compiled in; \
    every_shipped_manifest_loads proves they parse")]
pub fn registry() -> &'static Registry; // std::sync::LazyLock

impl Registry {
    pub fn parse(sources: &[&str]) -> Result<Self, ManifestError>;
    /// A `workflow` manifest only. The deal and listing paths use this,
    /// so an `insight` id is unknown to them.
    pub fn workflow(&self, ext: &ExtensionRef) -> Option<&Manifest>;
    pub fn details_schema(&self, ext: &ExtensionRef, schema: &str) -> Option<&Schema>;
    pub fn step_type(&self, ext: &ExtensionRef, step_type: &str) -> Option<&StepType>;
}

impl Manifest {
    pub fn allows(&self, kind: ResourceKind) -> bool;
    pub fn has_reserve_hook(&self) -> bool;
}

impl Schema {
    /// Checks `data` against the field list. A `money` value is minor
    /// units of the deal's currency, so the check needs no currency.
    pub fn check(&self, data: &serde_json::Value) -> Result<(), SchemaError>;
}
```

`registry()` uses `expect` because a shipped manifest that does not parse
is a bug in this build, and there is nothing a caller could do with the
error. The workspace lint `expect_used` is `warn` (`Cargo.toml:241`), and
`mise run verify` runs clippy with `-D warnings`, so the function carries
`#[expect(clippy::expect_used, reason = …)]`. This is the pattern of
`data_keystore/src/key_store.rs:44` and `identity/src/keys.rs:255`, which
use `allow`; `expect` is better because it fails when it is no longer
needed. `xtask check-lint-suppressions` counts only `too_many_lines`
suppressions; step 2 checks that it accepts this one. The unit test
`every_shipped_manifest_loads` is what makes the `expect` safe.

`Registry::parse` checks:

- ids are `[a-z0-9-]{1,32}` (`MAX_EXTENSION_ID_LEN`, `deal.rs:24`); no
  `(id, version)` twice; version at least 1;
- each schema name starts with `<id>/` and ends with `@<n>`; each step type
  starts with `<id>/` and is at most 64 bytes;
- field names are unique in a list;
- types are the eight of D-C3a-2, and each field has the limits its type
  needs: `max_len` present and within the type's ceiling for `text`,
  `long-text` and `code`; `min` at most `max` for `number`; `choices` not
  empty, codes, no repeats; `max_len`, `min`, `max` and `choices` only on
  the types that use them;
- `hooks` is all three or empty; `service-in-slot` only with hooks;
  `checks` names a known check;
- `kind` is `workflow` or `insight`, with the keys of D-C3a-1 for each;
- every derived message id of D-C3a-3 is unique across the registry.

`ManifestError` names the file's id and the rule.

**D-C3a-5. One set of functions checks a commitment's extension.** In
`extension.rs` (and `extension/check.rs` if `extension.rs` grows past a
few hundred lines):

```rust
pub enum ExtensionCheck {
    /// No extension on this commitment.
    None,
    /// Known extension; shape, kind, schema and named checks all pass.
    Checked,
    /// The extension is not a workflow extension in this node's registry.
    /// Only `check_details_shape` ran.
    NotChecked,
}

/// Runs first for every commitment that names an extension, known or
/// unknown: `extension.is_some() == details.is_some()`,
/// `details.extension == extension.id`, the id and version form, the
/// schema name form (`<id>/<name>@<n>`), and the `extra` limits
/// (`MAX_DETAILS_EXTRA` 16 items, label 64 bytes, value 512 bytes,
/// `deal.rs:32` to `:34`). Code `extension-shape`, or `details-mismatch`
/// for the first two rules, as today.
pub fn check_details_shape(c: &Commitment) -> Result<(), DealError>;

/// A full commitment (its `details` and `window` are plain).
pub fn check_commitment_extension(
    registry: &Registry,
    c: &Commitment,
) -> Result<ExtensionCheck, DealError>;

/// Only the kind rules, for `SignedTerms::validate_visible`, which sees
/// a commitment whose `details` and `window` are still hidden digests.
pub fn check_extension_kind(
    registry: &Registry,
    resource_kind: ResourceKind,
    kind: CommitmentKind,
    ext: Option<&ExtensionRef>,
) -> Result<ExtensionCheck, DealError>;

pub enum PartialCheck {
    /// Everything this extension checks was visible and passed.
    Checked,
    /// A part was hidden: push `details_matches_extension` to `not_checked`.
    Partial,
    /// Unknown extension: push `details_matches_extension` to `not_checked`.
    NotChecked,
}

/// For the presentation checker. `details` is `None` when hidden;
/// `window` is `None` when hidden and `Some(None)` when the commitment has
/// none.
pub fn check_partial_details(
    registry: &Registry,
    ext: &ExtensionRef,
    details: Option<&Details>,
    window: Option<Option<&TimeWindow>>,
) -> Result<PartialCheck, DealError>;
```

Rules of `check_commitment_extension`, in order. Rules 1 and 2 are
today's `validate_commitment_kind` refusals (`deal.rs:349` to `:390`),
kept with the same codes (P18):

1. The pair of resource kind and commitment kind is valid: `service-in-slot`
   or `goods-or-service` with kind `fulfilment`, `currency` with kind
   `currency`. Any other pair is `kind-mismatch` (`deal.rs:384` to `:387`).
2. No extension: a `service-in-slot` commitment is refused with
   `commitment-kind-not-supported` (`deal.rs:356` to `:361`), because a
   slot needs an extension with a reserve hook. Otherwise the commitment
   must have no `details` (`details-mismatch`, as today), and the result
   is `None`.
3. `check_details_shape` (it includes the extension id shape, today's
   check at `sd.rs:265` to `:269`).
4. `currency` names no extension (`commitment-kind-not-supported`).
5. `registry.workflow(ext)` is `None`: the result is `NotChecked`. An
   `insight` manifest is not a workflow manifest, so naming one gives
   `NotChecked` here and `extension-unknown` from every caller that
   refuses `NotChecked` (P9).
6. The manifest lists the resource kind (`commitment-kind-not-supported`
   otherwise, the code used today). For `service-in-slot`, the manifest
   must have a reserve hook (`commitment-kind-not-supported`).
7. `details.schema` is one of the manifest's schemas (`details-mismatch`).
8. `Schema::check` (`details-invalid`).
9. Each named check. `slot-matches-window` keeps its code `slot-details`
   and needs the window (a missing window is `slot-details`, as today).

`check_extension_kind` runs the rules that need neither `details` nor
`window`: 1, 2 without its `details` part, the extension id shape of
rule 3, 4, 5 and 6. It returns `NotChecked` for an unknown extension.
`validate_visible` keeps its other checks (`sd.rs:259` to `:285`).

`check_partial_details` runs the same rules on what is visible. With
hidden `details`, it returns `Partial`. With visible `details`, it runs
rules 3, 7 and 8 for a known extension (the schema check needs no window).
A named check that needs a hidden window gives `Partial`. A window that is
visible gives the full check. An unknown extension with visible `details`
runs rule 3 and returns `NotChecked`. Errors map to
`PresentationError::FieldInvalid` at the same field paths as today
(`presentation/validate.rs:394` to `:451`). The entries that stay in
`not_checked`: `details_matches_extension` for hidden `details`, for a
hidden window when a named check needs it, and for an unknown extension.
The presentation checker reaches `check_partial_details` only after the
commitment passed `validate_visible`, so rules 1, 2 and 4 to 6 have
already run.

**`DealTerms::validate` keeps its protection (P1).** `validate()` is not
only called by `quote.set` and `file_quote_card`. It is the last step of
`reveal_terms_strict` (`deal/sd.rs:384`), which every load of a held quote,
agreement receipt and settlement record goes through: `load_quote`
(`disclosures.rs:100`), so every accept, settle, fulfilment and
presentation call; the backup import (`backup/disclosures.rs:138`);
`quote_ops.rs:599`; `sync.rs:519`. So `validate()` calls
`check_commitment_extension(registry(), c)` for every commitment and keeps
the `Err`. A known extension is checked as fully as today; an unknown one
passes with the shape rules only. No caller of `validate()` changes.
`DealTerms::check_extensions(&self, &Registry) -> Result<Vec<ExtensionCheck>,
DealError>` is only for the callers that must know which commitments are
`NotChecked`: `quote.set`, `agreement.accept`, and the Hub's data.
`SignedTerms::validate_visible` calls `check_extension_kind`.

`validate_commitment_kind`, `validate_extension_and_details`,
`validate_scheduling_details` and `validate_scheduling_slot_details`
(`deal.rs:349` to `:474`) are replaced. The `extra` loop moves from the
scheduling function into `check_details_shape`, so `extra` of a one-shot
commitment and of an unknown extension has the same limits (P2). The
8 KiB limit of one `details` disclosure (`MAX_DETAILS_DISCLOSURE_BYTES`,
`disclosure.rs:21`) is not a `deal.rs` rule: `sd.rs` enforces it when it
conceals and reveals (`:174`, `:329`), and `quote_ops.rs:265` before
signing. It stays there, for known and unknown extensions alike.
`SCHEDULING_EXTENSION` and `SCHEDULING_SLOT_SCHEMA` (`deal.rs:25`, `:26`)
move to `extension.rs`.

**D-C3a-6. Who refuses an unknown extension.**

- `quote.set` (the author's node, the writer): every commitment must be
  `None` or `Checked`; else `extension-unknown`. The writer's check binds
  (EX-2), and the writer must know what it signs. The call goes in
  `validate_and_conceal_terms`
  ([quote_ops.rs:478](../../../../crates/roym_transaction/src/app/quote_ops.rs#L478)).
- `file_quote_card` (the counterparty's node): `NotChecked` is accepted;
  an `Err` refuses the card, as today (`sync.rs:526`).
- `agreement.accept` (the counterparty's node): refuses with
  `extension-unknown` only when this node is the giver of a `NotChecked`
  commitment. A giver must be able to run its extension's hooks and steps;
  a receiver only receives. This is a deviation from EX-2's "may still
  sign", accepted by the owner (Q9) and recorded in the extensions doc. In `accept_at`
  ([agreement_ops.rs:136](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L136)),
  before `reserve_counterparty_slots`.
- The writer's decision on the counterparty's half: no new check. The
  writer checked the same signed terms at `quote.set`.
- `listing.set` (catalog): `registry().workflow(ext)` must be `Some`; else
  `extension-unknown`. An `insight` id is refused the same way (P9).
- `ListingPayload::validate`, run by every verifier of a listing,
  including a stranger's directory: shape only. A newer node's listing
  stays searchable.

**D-C3a-7. The Hub has the same registry.** `crates/roym_web/ui/src/extensions/registry.ts`
imports the JSON files (`import scheduling from
"../../../../roym_core/extensions/scheduling.json"`), as `taxonomy.ts:1`
does. It exports `manifestFor(ext)`, `detailsSchema(ext, schema)`,
`stepType(ext, type)` and the id functions of D-C3a-3. The Hub decides
"not checked on this device" by the same lookup as the node: both read one
file of one build, so they cannot disagree. A Vitest test checks that the
Hub's list of files equals the Rust `MANIFESTS` list (by reading
`extension.rs`, as `card.rs:150` to `:189` reads `registry.ts`).

**D-C3a-8. The field view** (task.md Z4). `crates/roym_web/ui/src/cards/fields.ts`:

```ts
export interface FieldViewInput {
  ext: { id: string; version: number };
  schema?: FieldList;            // undefined: unknown extension
  data: Record<string, unknown>;
  extra?: { label: string; value: string }[];
  currency?: string;             // for money fields
  lang?: string;                 // the record's original `lang`, for free text
  idFor: (field: string) => string; // D-C3a-3
}
export function fieldView(input: FieldViewInput): HTMLElement;
```

- Known: one line per field in manifest order, `labelled(parent,
  t(idFor(name)), value)` (`dom.ts:55`), the value drawn by type (D-C3a-2).
  A field absent from `data` is left out. A `text` or `long-text` value
  gets `lang` as its `lang` attribute; a `code` value gets none.
- Unknown: one line per key of `data` in key order, the key as its label
  and the value as JSON text, both marked as data from another node; then
  the line `t("ext-not-checked")` ("Not checked on this device").
- Then `extra`, each `labelled(label, value)`; both are free text from the
  other party and get `lang` as their `lang` attribute.
- `quote.ts` passes the quote's original `lang` (not the language of an
  author translation that the reader picked). It draws the field view once,
  outside the `recordText` version switch, because `details` has no
  translations.
- `quote.ts` calls `fieldView` for each commitment with `details`, after
  the declared value (`:237`). C3a2's step template calls it for a step's
  `data`. C4's widgets will use it too.

**D-C3a-9. The listing change** (Q3 (a)). In `listing.rs`:
`ListingPayload.extension: Option<ExtensionRef>`; `BookingMode` and
`BookingTerms.mode` removed; `PaymentRequired`'s message reads "payment
terms are required when the listing names an extension"; `validate` checks
the extension's shape (`listing::valid_token`, `MAX_EXTENSION_ID_LEN`,
version at least 1).

**The rule between `booking` and the extension (P15).** Before this
change, a `slots` listing always had a `booking` block, because the mode
lived inside it. The new rule, in `ListingPayload::validate`:

- extension `scheduling` requires the `booking` block (lead time,
  cancellation window and per-booking maximum are slot rules). A new
  `ListingError::BookingRequired`, message "booking terms are required
  when the listing names the scheduling extension". This is a shape rule
  of a known id, so a stranger's directory checks it too. So
  `ListingPayload::validate` compares the id with the constant
  `extension::SCHEDULING_EXTENSION`, and does not call `registry()`: a
  `pub const &str` links no registry code, and every verifier of a listing,
  `directory` included, must check this rule without the registry
  (D-C3a-6, section 7.9). This is the one match on an extension id in
  `listing.rs`, and exit criterion 2 names it (P20);
- `one-shot` and no extension may have a `booking` block or not;
- a `booking` block with no extension is allowed (today's `enquiry` with
  lead time).

`catalog`'s `SetListingInput` gains `extension` (`listing_ops.rs:95`,
copied at `:437`) and runs D-C3a-6's check.

**The Hub's listing editor.** It keeps the select and its three labels,
and maps `slots`, `order`, `enquiry` to the three values of Q3 (a) in
`editor.ts` (`bookingBlock()`, `:159` to `:166`, and the params at `:298`)
and `listings.ts` (`:41` to `:45`, `:341`, `:473` to `:481`). The select
moves out of the booking block into the listing's main fields, because it
is no longer part of `booking`; its label `listing-label-mode` stays. The
default stays "enquiry" (no extension). When the person picks "slots", the
editor switches the booking block on, and its "on" box cannot be cleared
while "slots" is picked. So the Hub never sends a listing that
`BookingRequired` refuses.

**An old stored listing (P13).** `ListingPayload` and `BookingTerms` have
no `deny_unknown_fields`, so a stored or held listing with `booking.mode`
still parses after this change. The old key is ignored, and the listing
has no extension: a former `slots` listing silently stops naming
scheduling, and it still verifies (the signed bytes do not change). This
is the owner's pre-release rule (no migration; development nodes start
empty), not a refusal. The catalog's `SCHEMA_VERSION` bump gates only the
catalog backup import (`roym_catalog/src/app/backup.rs:63`); nothing
refuses an old listing at start-up. A unit test
`old_listing_with_booking_mode_has_no_extension` fixes this behaviour, so
it is known and not a surprise. `deny_unknown_fields` is not added: it
would make an older node refuse a newer node's listing.

**The directory query (P14).** `SearchQuery.extension: Option<String>` is
an extension id on a wire-reachable method. The old field was an enum, so
its value was always one of three words. `directory.search` checks the
value with `listing::valid_token(value, MAX_EXTENSION_ID_LEN)` and answers
`invalid_params` otherwise (test
`search_refuses_an_extension_filter_that_is_not_an_id`). The query does
not check the value against the registry: a directory may index listings
of extensions it does not know. "No extension" cannot be asked for (Q3).

**D-C3a-10. The step record** (C3a2, Q5 (a), Q6 (a)). In
`crates/roym_core/src/step.rs`:

```rust
pub const STEP_VERSION: u32 = 1;

#[serde(deny_unknown_fields)]
pub struct StepPayload {
    pub agreement: String,         // the agreed quote version, "rec_…"
    pub conversation: String,
    pub commitment_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub role: SettlementRole,      // the signer's role in this commitment
    pub extension: ExtensionRef,
    pub step_type: String,         // "<extension id>/<name>"
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,           // exactly 1: `data`
}

pub struct StepHidden { pub data: serde_json::Value } // a JSON object

pub fn verify_step(envelope: &str, now_secs: u64) -> RecordVerdict<StepPayload>;
```

`STEP_SET` (one field, `data`) goes beside `FULFILMENT_SET` in
`disclosure.rs`. `StepPayload::validate` checks the same things as
`FulfilmentReceiptPayload::validate`
([fulfilment.rs:70](../../../../crates/roym_core/src/fulfilment.rs#L70)),
plus `step_type` starts with `extension.id + "/"` and is at most 64 bytes.
The extensions doc's "optional pointer to an earlier step" is left out: no
step type in M08C needs it, and each step type happens at most once per
role of a commitment (D-C3a-11). A deviation in the extensions doc records
it.

**Limits (P4).** Two new constants in `step.rs`:

- `MAX_STEP_DATA_BYTES = 8 * 1024`, the same as
  `MAX_DETAILS_DISCLOSURE_BYTES`. It bounds the serialized `data` value
  (the disclosure's plain JSON). `conceal_hidden` does not limit a single
  field, so the limit is checked in `step.add`, in the filing (D-C3a-12)
  and in the import (D-C3a-13). For an unknown extension it is the only
  bound on `data`.
- `MAX_STEPS_PER_AGREEMENT = 64`: the step rows one agreement may hold, of
  both parties together. It is checked in `step.add` and in the filing, by
  counting the `steps` rows of that agreement (index `agreement`). Without
  it, a counterparty could file 2,000 step cards with made-up step types
  of an unknown extension, and use up the conversation's card cap
  (`MAX_CARDS_PER_CONVERSATION`, `transaction.rs:85`). A repeat of a stored
  step (same record id) is not counted again.

If the owner wants other values, the deviation and the backlog row name
them.

**D-C3a-11. Rules of `step.add`** (C3a2), in a new
`crates/roym_transaction/src/app/step_ops.rs`. Input `{ agreement,
commitment_id, step_type, data }` (`data` default `{}`). Order:

1. `resolve_principal_and_owner` (`app.rs:284`).
2. The agreement row exists, both halves are held, and the agreement is
   not terminal: the same checks as `check_fulfilment_preconditions`
   ([fulfilment_ops.rs:340](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L340)).
   Extract the shared part into one function in a new `app/parties.rs`
   (`agreement_party(&AgreementRow, owner) -> Result<DealRole, Response>`)
   and call it from both, so there is no copy.
3. The commitment exists, names an extension, and is not final
   (`is_final`, `deal/transition.rs:47`, on the row's `commitments`).
4. The step type is in the registry for that extension
   (`step-type-unknown` otherwise). The signer's role matches `by`
   (`not-your-step` otherwise). `data` is at most `MAX_STEP_DATA_BYTES`
   (`step-data-too-large`) and passes the step's field list
   (`details-invalid`). The agreement holds fewer than
   `MAX_STEPS_PER_AGREEMENT` step rows (`too-many-steps`).
5. Made once per role: `create` the row
   `step:<agreement>:<commitment_id>:<step_type>:<role>` (`role` is
   `giver` or `receiver` of that commitment) in a new `steps` collection,
   holding the envelope, record id and
   send state, after the record is signed and its disclosure is stored and
   before the card is sent (the rule "A signed record is made once",
   agreements doc `:729`). If the row exists, return the stored record
   with state `already-recorded` (the same shape as
   `already_recorded_fulfilment_response`).
6. Send the card with `send_card_and_file` and `OutgoingCard`
   (`app.rs:549`, `:477`), `card_type: "step"`, the one disclosure.

`step.list { agreement }` returns every step row of the agreement, oldest
first, with plain `data` from the stored disclosure, `issuer`,
`issued_at_secs`, `verified`, and `checked` (false for an unknown
extension). `step.verify` takes `{ envelope, disclosures? }` and returns a
`DisclosedVerdict<StepPayload>`, as `fulfilment.verify` does
(`fulfilment_ops.rs:579`).

**D-C3a-12. Filing a `step` card** (C3a2), in a new
`crates/roym_transaction/src/app/sync/steps.rs`, one more arm in
`dispatch_known_card` ([sync.rs:288](../../../../crates/roym_transaction/src/app/sync.rs#L288)):
verify the envelope (`verify_step`); `reveal_hidden_strict` with
`STEP_SET`; an agreement this node does not hold yet, or whose pair is not
complete, is deferred with `defer_or_refuse`
([receipts.rs:23](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L23)),
as `validate_payment_request_agr` does
([settle_receipts.rs:37](../../../../crates/roym_transaction/src/app/sync/settle_receipts.rs#L37));
the parties and conversation match the row; the commitment exists and its
extension equals the step's; the issuer is the DID that `role` names, and
`role` is the giver or receiver of that commitment; `data` is at most
`MAX_STEP_DATA_BYTES`; the agreement holds fewer than
`MAX_STEPS_PER_AGREEMENT` step rows; for a known extension, the step type
and `by` and the field list pass; an unknown one is filed with
`checked: false`. The row key of D-C3a-11 (with the role) makes a repeat a
repeat (same record id) and refuses a different record for the same key
("a second step of this type by this role"). With `by: either`, the
other party's step of the same type has a different key and is filed. The
disclosure is stored with `disclosures::store` (`disclosures.rs:33`)
before the row.

**D-C3a-13. Backup** (C3a2). Export writes the `steps` collection as a new
section `SECTION_STEPS` beside `SECTION_FULFILMENTS`
(`backup/sections.rs:10`). Import checks every row: the key's agreement and
commitment are in the same bundle, the envelope verifies, the issuer
matches the role, the key's role equals the payload's `role`, the
disclosure is in the bundle and passes `reveal_hidden_strict`, its `data`
is at most `MAX_STEP_DATA_BYTES`, and no agreement has more than
`MAX_STEPS_PER_AGREEMENT` rows (the C1 review rule "the import checks
every ledger row", agreements doc `:1323`).

Two `match` statements by card kind need a `step` arm (P16):

- the card re-verify match (`backup.rs:735` to `:748`; and see section
  2.2, point 10);
- `rebuild_card_data` (`backup/disclosures.rs:263`), which rebuilds an
  imported card's plain `data` from its disclosures. It has its own
  `match` (five arms, then `_ => {}`). Without a `step` arm an imported
  step card has `data: None`, and nothing reports an error. Test
  `imported_step_card_has_its_plain_data`.

**D-C3a-14. The Hub's step card** (C3a2).
`crates/roym_web/ui/src/cards/templates/step.ts`: title
`t(stepNameId(ext, type))` for a known step, else the step type as data
and the "not checked" line; the signer ("you" or the other person's name,
as the fulfilment card does) and the time; then `fieldView` of `data`. It
is added to `CARD_TYPES` in `registry.ts`, to the switch in `render.ts`
(`:28` to `:51`), to `samples.json`, and to the e2e gallery test
(`roym-hub.spec.ts:199` to `:222`, "eight known types" becomes nine).

**D-C3a-15. roymctl.** C3a1: no new command or flag. One import changes:
`build_quote_commitments` reads `SCHEDULING_EXTENSION` and
`SCHEDULING_SLOT_SCHEMA` from `syneroym_roym_core::deal`
([transaction.rs:359](../../../../apps/roymctl/src/commands/roym/transaction.rs#L359)
to `:362`); after step 3 they live in `syneroym_roym_core::extension`
(P7). `roymctl` gets no `--one-shot` flag (Q4). C3a2: `roymctl roym step add
--agreement --commitment --type [--data <json>]` and `roymctl roym step
list --agreement`, in a new `apps/roymctl/src/commands/roym/step.rs`,
following `FulfilmentCommands`
(`apps/roymctl/src/commands/roym/agreement.rs:125`).

## 6. Step-by-step changes

Each step builds, and its tests pass, before the next starts.

### 6.1 C3a1

#### Step 1 — Commit this plan

Done with this commit.

#### Step 2 — Manifests and the registry (`roym_core`, pure, beside today's code)

- Add `crates/roym_core/extensions/scheduling.json` and `one-shot.json`
  (D-C3a-1). Add `extension.rs`, `extension/schema.rs` (field types and
  `Schema::check`), `extension/tests.rs`. Export from `lib.rs`.
- Nothing calls it yet. Tests: section 7.1 "registry" and "schema".

#### Step 3 — One check for every caller (`roym_core`)

- Add `check_details_shape`, `check_commitment_extension`,
  `check_extension_kind`, `check_partial_details` and
  `DealTerms::check_extensions` (D-C3a-5). `DealTerms::validate` calls
  `check_commitment_extension(registry(), c)` for each commitment, so
  every caller of `validate` and of `reveal_terms_strict` keeps today's
  protection with no change (P1). `SignedTerms::validate_visible` calls
  `check_extension_kind`; the presentation checker's
  `validate_commitment_details_partial` calls `check_partial_details` and
  keeps its field paths and `not_checked` entries (P3).
- Move `SCHEDULING_EXTENSION` and `SCHEDULING_SLOT_SCHEMA` to
  `extension.rs` and fix every import: `roym_transaction` tests
  (`tests/disclosure/cards.rs:10`), `presentation/validate.rs`, and
  `apps/roymctl/src/commands/roym/transaction.rs:359` to `:362` (P7).
  `git grep -n 'SCHEDULING_EXTENSION\|SCHEDULING_SLOT_SCHEMA'` must show no
  path through `deal::` after this step.
- Add `extension-unknown` and `details-invalid` to `DealError` and its
  `code()` (`deal.rs:192`, `:242`).
- `deal/tests.rs`: today's scheduling cases pass unchanged; add the
  one-shot, unknown-extension, `extra` limit and `reveal_terms_strict`
  cases of section 7.1.

#### Step 4 — `transaction` callers (EX-2)

- `quote.set` refuses `NotChecked` (D-C3a-6); `file_quote_card` accepts
  it; `agreement.accept` refuses a `NotChecked` commitment this node
  gives. The seat code is not changed (section 4, "Stays the same", P12).
- Service tests (section 7.2). The unknown-extension quote reaches the
  counterparty in a test as a hand-signed card, the way the C1 tests file a
  hand-signed half (agreements doc "AG-12's test", `:1254`).

#### Step 5 — The listing names its extension (Q3)

- `listing.rs` (D-C3a-9: the extension field, `BookingRequired`, the
  payment rule), `listing/tests.rs` (`:112` to `:117`, `:244` to `:255`;
  new cases of section 7.1), `record/tests/wire_words.rs` (`:21`, `:287`
  to `:292`, `:362`: the `every!` row for `BookingMode` goes; add none,
  because the new field holds no enum).
- `catalog`: `SetListingInput.extension`, the `listing.set` check,
  `SCHEMA_VERSION` 3 to 4.
- `directory`: `SearchQuery.extension` with its id check (P14), the index
  column, the query clause, its schema version, `search_ops/tests.rs:59`,
  `:73`, and the new test of section 7.2.
- `roym_federated_query/src/tests/helpers.rs:124`, `:200` (they set
  `booking: None`; unchanged unless the field moves).
- Fixtures: `fixtures.rs:97` (`"mode": "slots"` becomes `"extension": {
  "id": "scheduling", "version": 1 }` at the listing's top level),
  `trust_harness.rs:187` to `:192` and `:544` to `:549` (drop `mode`),
  `method_shapes/listing_verify_good.json`. Regenerate the shapes with
  `UPDATE_METHOD_SHAPES=1` (`method_contract/shapes.rs:92`), and review the
  diff by hand.
- Hub: `editor.ts`, `listings.ts`, `editor.test.ts` (`:24` to `:33`,
  `:166` to `:167`), `listings.test.ts:72` to `:78` (same five options,
  same Hindi labels). New editor cases: picking "slots" switches the
  booking block on, and its "on" box cannot be cleared while "slots" is
  picked (D-C3a-9, P15).
- e2e: no change. `tests/roym-hub.spec.ts:344` to `:345` only switches the
  booking and service blocks on, so that their default fields are valid.
  It never sets or checks the mode (the checks at `:351` to `:362` look
  only at the numbers). With the default "enquiry", the select's move does
  not touch it (P15).

#### Step 6 — Hub registry and field view

- `extensions/registry.ts`, its test against `extension.rs` (D-C3a-7).
- `cards/fields.ts` and `cards/fields.test.ts` (D-C3a-8). Call it from
  `quote.ts`.
- `quote_terms.ts:4` and `:5` import the two constants from the registry
  module instead of defining them.
- `en.ftl` and `hi.ftl`: `## Extensions` with
  `ext-scheduling-slot-listing-id`, `ext-scheduling-slot-slot-id`,
  `ext-not-checked`, `ext-details-heading` (the heading over the lines);
  and `messages-accept-extension-unknown` beside
  `messages-accept-failed`, shown by the new branch in `messages.ts`'s
  accept error handler (`:686` to `:690`) when the error is
  `extension-unknown` (Q9). A case in `screens/messages_translate.test.ts`
  (which already drives `agreement.accept`) checks the branch.
  `catalog.test.ts`: the manifest id test (D-C3a-3).
  `hindi-screens.test.ts`: a quote card with scheduling `details`, in
  Hindi, and one with an unknown extension whose keys carry the `Zq`
  marker.

#### Step 7 — Sizes, docs and close-out of C3a1

- `mise run build:roym`, component sizes before and after (section 7.9).
- Deviations, backlog rows, living docs (sections 8 and 9). The
  extensions doc's Tasks: tick "Manifest format and the extension registry
  inside Roym" and "Commitment `details` in the deal core, and schema
  checks", and "Add one-shot" in the scheduling line.
- `demo:` in the extensions change doc's front matter is not used (that
  doc has none); the C3a1 row in status.md records `- none: <reason>`
  (Q7).
- `mise run verify` with no `--skip`.

### 6.2 C3a2

#### Step 1 — The record (`roym_core`, pure)

- `step.rs`, `step/tests.rs` (D-C3a-10). `RECORD_STEP` and `("step", 1)` in
  `RECORD_TYPES` (`record.rs:18`); `("step", 1)` in `CARD_TYPES`
  (`card.rs:11`) and the Hub's `registry.ts` in the same commit, because
  `the_ui_card_registry_matches_this_crate` (`card.rs:150`) compares them.
  A `wire_words.rs` sample for `step`.
- The scheduling manifest's `scheduling/started` step (already in
  D-C3a-1's file; C3a1 ships it with no caller, so the loader test covers
  it).

#### Step 2 — `step.add`, `step.list`, `step.verify`

- `step_ops.rs`, `app/parties.rs` (D-C3a-11), the `steps` collection in
  `ensure_collections` (`app.rs:330`) with indexes `conversation` and
  `agreement`, four rows in `api.rs` (`:29` to `:77`).
- `method_contract` rows and `method_shapes/step_*.json`; a parity
  scenario in `crates/roym_web/tests/dual_build_parity/step.rs`.

#### Step 3 — Filing and backup

- `sync/steps.rs` and the `dispatch_known_card` arm (D-C3a-12). The live
  paths build a card's plain `data` with `disclosures::plain_view`
  (`disclosures.rs:143`; called at `app.rs:496` when this node sends, and
  at `sync.rs:564` and `:682` when it files). The step card uses the same
  function.
- Backup section and import checks (D-C3a-13): the `step` arm in both the
  re-verify match (`backup.rs:735`) and `rebuild_card_data`
  (`backup/disclosures.rs:263`), which runs only on import; the
  `deal-decline` check of section 2.2, point 10.

#### Step 4 — Hub step card and step names

- `cards/templates/step.ts`, `render.ts`, `samples.json`,
  `templates.test.ts` (a `describe` for `step`: known, unknown, the
  signer line), `hindi-screens.test.ts`, the e2e gallery count.
- `en.ftl`, `hi.ftl`: `ext-scheduling-step-started` ("Work started").

#### Step 5 — roymctl and two-node

- `apps/roymctl/src/commands/roym/step.rs`, `cli_args.rs` cases.
- `crates/substrate/tests/roym_step_e2e.rs` on `common::SubstrateTestContext`
  and `common/roym_flow.rs`, and the step in `roym_restore_e2e.rs`
  (section 7.4).
- The demo story of Q10 (a): `demo/60-roym-step.spec.ts`,
  `demo/60-roym-step-quote.sh` and `demo/60-roym-step-start.sh`, in the
  order of Q10's answer. The C3a2 row in status.md lists these files as
  its demo. Run with `mise run demo`.

#### Step 6 — Sizes, docs and close-out of C3a2

- As C3a1 step 7. Tick "`step` record and card, the generic step view in
  the Hub" in the extensions doc's Tasks.

## 7. Tests

### 7.1 Unit tests (`roym_core`)

Registry and manifests (`extension/tests.rs`):

- `every_shipped_manifest_loads`: `Registry::parse(MANIFESTS)` is `Ok`, and
  holds `scheduling` 1 and `one-shot` 1. `registry()` returns the same.
- One failing case per load rule of D-C3a-4: a bad id, a repeated
  `(id, version)`, version 0, a schema name without the id prefix or
  `@n`, a step type without the prefix, a repeated field name, a `text`
  field with no `max_len` and one with `max_len: 513`, a `code` field
  with `max_len: 129`, `min` above `max`, an empty `choices`, a repeated
  choice, a choice that is not a code, `max_len` on a `number` field,
  hooks that are not all three or none, `service-in-slot` with no hooks,
  an unknown check, an unknown field type (`location`, Q8), an unknown
  key, and two manifests whose derived message ids collide (D-C3a-3).
- `message_ids_are_derived_as_written`: `scheduling/slot@1` field
  `listing_id` gives `ext-scheduling-slot-listing-id`; step
  `scheduling/started` gives `ext-scheduling-step-started`.
- `insight_manifest_loads_and_cannot_be_named` (EX-19): a test-only
  insight manifest with `reads` loads; `Registry::workflow` does not return
  it; a commitment that names it is `NotChecked`, and `quote.set` and a
  listing refuse it with `extension-unknown` (P9); a workflow manifest
  with `reads` fails to load.

Schema (`extension/schema/tests.rs`): for each of the eight types, one
passing and one failing value; a `code` with a space; `required`; a key not
in the list; a non-object `data`; a float in an integer field.

The check (`deal/tests.rs`):

- Today's scheduling cases, unchanged: a good slot, a slot id that does not
  match the window (`slot-details`), a missing window.
- `one_shot_goods_commitment_is_checked`; one-shot on `service-in-slot` or
  on `currency` is refused (`commitment-kind-not-supported`); one-shot with
  scheduling's schema is refused (`details-mismatch`); one-shot with a
  field in `data` is refused (`details-invalid`).
- `unknown_extension_is_not_checked`: `ext: future 1` with any `details`
  gives `NotChecked`; on `currency`, an error; with
  `details.extension != ext.id`, `details-mismatch`.
- `extra` limits (P2): a one-shot commitment with 17 `extra` items, one
  with a 65-byte label, one with a 513-byte value; and the same three for
  an unknown extension. Each gives `extension-shape`.
- `reveal_terms_strict` keeps the check (P1):
  `reveal_strict_refuses_a_slot_id_that_does_not_match_its_window`, and
  `reveal_strict_reveals_a_quote_with_an_unknown_extension`.
- `validate_visible`: an unknown extension passes; a known one on a
  resource kind it does not list is refused.
- The kind rules (P18), each for `validate` and for `validate_visible`:
  `service-in-slot` with no extension (`commitment-kind-not-supported`),
  `goods-or-service` with kind `currency` (`kind-mismatch`), and
  `currency` with kind `fulfilment` (`kind-mismatch`). Today's case at
  `deal/tests.rs:181` keeps passing.
- The presentation checker (P3; `presentation/tests/validation.rs:112` to
  `:149` keep passing unchanged): scheduling with a good window (checked);
  a wrong `slot_id` with a visible window (`FieldInvalid`); a hidden
  window (`details_matches_extension` in `not_checked`); hidden `details`
  (the same); an unknown extension with visible `details`
  (`details_matches_extension` in `not_checked`, presentation verified);
  a one-shot commitment whose visible `details` fail its schema
  (`FieldInvalid`).

Listing (`listing/tests.rs`): the extension's shape; `payment` may be
absent only with no extension; `scheduling` with no `booking` is
`booking-required`; `one-shot` with no `booking` and `booking` with no
extension both pass (P15); `old_listing_with_booking_mode_has_no_extension`
(P13); the `wire_words` sample.

Step (C3a2, `step/tests.rs`): `validate`'s cases (as the fulfilment
receipt's); `verify_step` with a good, a tampered and a wrong-type
envelope; `conceal_hidden` and `reveal_hidden_strict` with `STEP_SET`; a
`data` of `MAX_STEP_DATA_BYTES` passes the size check and one byte more
fails.

### 7.2 In-crate service tests (`crates/roym_transaction/src/app/tests/`)

New file `tests/extension.rs` (C3a1):

- `quote_set_refuses_an_unknown_extension` (`extension-unknown`).
- `quote_set_refuses_details_that_fail_the_schema` (`details-invalid`).
- `counterparty_files_a_quote_with_an_unknown_extension`: a hand-signed
  quote card with `ext: future 1` on the author's commitment is filed,
  `verified: true`, and `agreement.accept` signs.
- `accept_refuses_when_this_node_gives_an_unknown_extension`.
- `one_shot_commitment_settles_like_today` (EX-18): quote, accept,
  payment acknowledgement halves, fulfilment receipt halves, the snapshot
  shows both commitments settled.

New file `tests/step.rs` (C3a2): add and list; a repeat returns
`already-recorded` with the same record id; `by: giver` refuses the
receiver (`not-your-step`); an unknown step type; a commitment with no
extension; a final commitment; an agreement not formed; a filed card from
the other party; a card for an agreement not yet held is deferred, then
filed after the receipt; a second record for the same key is refused; a
card whose issuer does not match its role is refused; export then import
keeps the step and its disclosure; an import row whose disclosure is
missing is refused. Limits and keys (P4, P5, P16): a step whose `data` is
one byte over `MAX_STEP_DATA_BYTES` is refused at `step.add`, at filing
and at import; the 65th step row of one agreement is refused at filing;
with a test-only manifest whose step is `by: either`, both parties record
it once each; an import row whose key's role differs from the payload's
`role` is refused; `imported_step_card_has_its_plain_data`.

Directory (`crates/roym_directory/src/app/search_ops/tests.rs`, C3a1):
`search_refuses_an_extension_filter_that_is_not_an_id` (P14), and a search
with `extension: "scheduling"` finds only the listings that name it.

### 7.3 Parity (both builds agree)

`mise run build:roym` first. C3a1: the listing scenarios through the
shared `full_listing_params` (no new scenario); one quote scenario with a
one-shot commitment. C3a2: `dual_build_parity/step.rs` with add, list and
verify, seeded salts as in C2; and `dual_build_parity/bundles.rs`, whose
method and section lists (`:99` to `:100` name the fulfilment methods) get
the `step.*` methods and the `steps` section (P16).

### 7.4 Two-node (`crates/substrate/tests`)

C3a2, `roym_step_e2e.rs`: a slot agreement between two nodes (the
existing slot quote helper at `common/roym_flow.rs:195`); the giver's `step.add`; the
other node's `transaction.sync`, then `step.list` shows the step with plain
`data`, `verified: true`, `checked: true`; `step.verify` on the other node
with the stored disclosure. Ports from `alloc_ports`, no literal.
`roym_restore_e2e.rs` (which already restores a fulfilment, `:341` to
`:370`) adds one step before the export and checks that it is back after
the import (P16).

### 7.5 Hub unit tests (`npm test`)

- `extensions/registry.test.ts`: the file list equals Rust's; the id
  functions.
- `cards/fields.test.ts`: each field type's view; a missing field is left
  out; an unknown extension draws its keys and the "not checked" line;
  `extra`; a `text` value and each `extra` line carry the quote's `lang`
  attribute, and a `code` value carries none (P6).
- `templates.test.ts`: a quote with scheduling `details` shows the two
  labelled lines; C3a2: the `step` describe.
- `catalog.test.ts`: the manifest ids (D-C3a-3).
- `hindi-screens.test.ts`: the cases of step 6 and C3a2 step 4.
- `editor.test.ts`, `listings.test.ts`: the select's values.

### 7.6 Playwright

- C3a1: no change and no new spec (step 5, P15).
- C3a2: the gallery count (`tests/roym-hub.spec.ts:199` to `:222`), and
  the demo story `60-roym-step` (Q10).
- Stories 40 and 50 pass unchanged (they create no extension).

### 7.7 Not tested

- A Hub action that makes a step: none exists (Q6).
- A real version skew between two builds. The unknown-extension cases use
  a hand-signed card and a test-only manifest instead.

### 7.8 Size and time risks

Files near the 800-line cap that this slice must not grow:
`sync.rs` (717: one match arm of about 12 lines; the filing goes in
`sync/steps.rs`), `app.rs` (688: one collection name and one
`ensure_coll` call), `quote_ops.rs` (749: one call in
`validate_and_conceal_terms`), `backup.rs` (766: one match arm; the
section code goes in `backup/sections.rs`, 707, or a new
`backup/steps.rs`), `agreement_ops.rs` (561), `settle_receipts.rs` (663:
not touched). `deal.rs` (600) shrinks when the scheduling functions move
out. `listing.rs` (584) and `search_ops.rs` (569) change by a few lines.

On the Hub, `messages.ts` (1,069) gains one `else if` branch; `quote.ts`
(379) gains one call.

### 7.9 Component size

Each part reports every component's size, before and after, from one
build folder ([task.md](task.md) section 1). Expected: C3a1 adds the
registry and schema code to `transaction` (new record code, limit +15%)
and to `catalog` (`listing.set` checks the extension against the
registry; limit +40 KB, and `serde_json` is already linked there).
`directory` runs only the shape check of `ListingPayload::validate`, so it
should not link the registry; the size report shows whether it does. C3a2
adds the step record to `transaction` only. Over a limit: stop and tell
the owner.

### 7.10 Hindi text

New messages for the owner to review before each merge:
C3a1: `ext-details-heading`, `ext-not-checked`,
`ext-scheduling-slot-listing-id`, `ext-scheduling-slot-slot-id`,
`messages-accept-extension-unknown`.
C3a2: `ext-scheduling-step-started`. Each has a Devanagari letter
(`catalog.test.ts:119` to `:131`).

### 7.11 How C3a avoids the kinds of findings in the C1 and C2 reviews

- **Copies of one shape** (C2 R20, R27): one registry read by both Rust
  and the Hub; one `check_commitment_extension` for the three old places;
  one `agreement_party` for step and fulfilment.
- **Silent failures** (C2 R21: `if let Ok(Some(..))`): every new
  `get_row` matches `Ok(None)` and `Err` apart, as `sign_at` does
  (`fulfilment_ops.rs:93` to `:97`).
- **Who signed is not checked** (C2 R24, R26, R29): D-C3a-12 ties the
  issuer to the role and the role to the commitment.
- **Import trusts the bundle** (C1 R13, C2 R31): D-C3a-13 checks every
  `steps` row.
- **Made once broken by a resend** (C1 R2, R28): D-C3a-11 step 5 stores
  before sending and reuses the row.
- **Recorded shapes left stale** (C2 R17): section 6 names every fixture
  and shape file found (section 2.2, points 7 and 8).
- **Tests that prove less than their names** (C1 R5, C2 R25): section 3
  names the part each test proves; test names are checked with
  `git grep 'fn <name>'` before a change doc names them.
- **Living docs left false** (C1 R6, C2 R40): section 9 names the lines.
- **Status ahead of facts**: "Merged" only after the merge.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C3a action |
| --- | --- |
| "Widget labels and extension step names do not come from the catalog yet" (`:574`, target C4) | C3a2 closes the step-name half; the row keeps widget labels, target C4. |
| "Nobody can mark work as started" (`:665`, target C3b) | C3a2 resolves it for `roymctl` (Q6 (a)). The resolved row says "work started is a card, not a state: the commitment does not move to `in-progress`". The row moves to "Recently resolved", and a new row "The Hub has no action to mark work started" goes under the Hub action panel row (`:659`), target C6 or TBD. |
| "`RECORD_TYPES` is not read by any verifier" (`:661`) | Not targeted. C3a2 adds `step` to the table; note it in the row. |
| New: "Extension field types `location`, `file-pointer` and `record-pointer` are not built" (Q8) | C3a1 adds it, target: the first extension that needs one. |
| New: "A step cannot point at an earlier step, and each step type happens once per role of a commitment" (D-C3a-10, P5) | C3a2 adds it, with the two limits `MAX_STEP_DATA_BYTES` (8 KiB) and `MAX_STEPS_PER_AGREEMENT` (64) (P4). Target: the first extension that needs a repeated step. |
| New: "Free text in `details` is not offered to machine translation" (D-C3a-2, P6) | C3a1 adds it. The Hub draws `text`, `long-text` and `extra` with the quote's `lang`, but its translate action covers only the quote's own texts. Target: TBD. |
| New: "The directory cannot search for listings with no extension" (Q3, P14) | C3a1 adds it. Target: TBD, when a screen needs it. |
| "Roym extensions are only those that ship with Roym" (`:540`) | Unchanged; the registry is that fixed list. |

The seat code that does not ask the registry for a reserve hook (section 4, P12) gets no row: C3b's task already moves it, and section 4 lists its five sites for C3b's plan.

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `docs/system-requirements-spec.md:727`: `BookingTerms` "modeled by
  `BookingMode` (`Slots`, `Order`, `Enquiry`)" becomes "the listing names
  its extension (scheduling, one-shot, or none for an enquiry)" (C3a1).
- `docs/developer-guide.md:2530`: the query field `booking_mode` becomes
  `extension` (C3a1); a new short section "Roym extensions" (where the
  manifests are, how to add one, the field types, the message ids) (C3a1);
  `:2553`: the card types list gets `step`, and the `step.*` methods
  (C3a2).
- `docs/system-architecture.md:1210` (the quote's commitments name an
  extension: add "checked against the registry, or marked not checked");
  `:1207` to `:1216` and `:3123` ("eight card types" becomes nine, with
  `step`) (C3a2); a paragraph on the registry where Roym's records are
  described (C3a1).
- `docs/roym-integrated-experience-spec.md:739` to `:750` ("eight card
  types", C3a2).
- `docs/TERMINOLOGY.md`: Roym extension, extension manifest, registry,
  step record (check which exist first).
- `docs/planning/traceability-matrix.md`: EX-1 to EX-3, EX-7, EX-8, EX-18,
  EX-19, ML-28 rows.
- Change docs (not living docs, but owned here):
  - roym-extensions `## Deviations`: Q2 (JSON in `roym_core`), D-C3a-1
    (`version` is a number; `resource_kinds`, `hooks` and `checks` keys),
    D-C3a-2 (the field type `code`, Q8), EX-19 (manifest field only;
    written in the plan commit), Q9 (a giver cannot accept an unknown
    extension; written in the review commit), the listing's payment and
    booking rules (section 4, D-C3a-9), D-C3a-10 (no pointer to an earlier
    step; once per role; the two step limits), Q6 (the first step type is
    in C3a2, not C3b).
  - [task.md](task.md): Z3 names JSON in `roym_core/extensions/`, and the
    C3b line says the scheduling manifest stays there (P17). Written in
    the review commit.
  - roym-agreements `## Deviations`: open question 3's file format and
    place (Q2).
  - roym-multilingual `## Deviations` (`:66`): extension step names come in
    M08C C3a2, widget labels in C4. Written in this plan's commit, because
    task.md already moved it.

## 10. Exit criteria

C3a1:

- [x] Every C3a1 row of section 3 names a test that exists and passes.
  (Each named test exists; all passed in CI on `62fdff99`.)
- [x] No code outside `crates/roym_core/src/extension.rs` (and its
  submodules) names `scheduling/slot@1` or matches on an extension id,
  except the `slot-matches-window` check, the seat code sites of section 4
  (until C3b), the users of the moved constants
  (`apps/roymctl/src/commands/roym/transaction.rs` and the Hub's
  `quote_terms.ts`, which imports them from `extensions/registry.ts`), and
  the `booking-required` rule in `listing.rs` (it uses the constant
  `SCHEDULING_EXTENSION`, because a stranger's directory checks it without
  the registry; D-C3a-9). Each of these uses the constant, never a string
  literal.
  Check with `git grep -n 'scheduling/slot@1\|"scheduling"'`.
  (On `629f8f28`, outside tests, only `extensions/scheduling.json` and the
  two constants in the Hub's `extensions/registry.ts` match.)
- [x] `DealTerms::validate` still refuses a scheduling quote whose
  `slot_id` does not match its window, on every path through
  `reveal_terms_strict` (P1). (`reveal_strict_refuses_a_slot_id_that_does_not_match_its_window`.)
- [x] Stories 40 and 50 and the Hub cases pass unchanged. (CI "Playwright
  E2E" on `62fdff99`; Hub: 49 files, 599 passed.)
- [x] Parity passes in both harnesses after `mise run build:roym`. (CI on
  `62fdff99`; locally 264 parity tests ran after the review fixes.)
- [x] Component sizes recorded and within limits ([task.md](task.md),
  section 1).
- [x] Deviations, backlog rows and living docs done.
- [ ] Owner reviewed the five Hindi messages. Open: six messages (see the
  roym-extensions Deviations), waiting for the owner.
- [x] `mise run verify` passes with no `--skip`. Replaced by CI, by owner
  decision (2026-11-20): every CI job passed on `62fdff99`
  ([run](https://github.com/syneroym/syneroym/actions/runs/44770697189)),
  and the commits after it change only docs.

C3a2:

- [ ] Every C3a2 row of section 3 names a test that exists and passes.
- [ ] The two-node step test passes with no hardcoded port.
- [ ] Export and import keep steps, with their plain `data`; the
  `deal-decline` re-verify question of section 2.2, point 10 is answered
  with a test.
- [ ] The demo story `60-roym-step` (Q10) passes in `mise run demo` and in
  `npm test`, and the C3a2 row in status.md lists its three files.
- [ ] Component sizes recorded and within limits.
- [ ] Owner reviewed the Hindi message.
- [ ] `mise run verify` passes with no `--skip`.

## 11. What C3a does not do

- Move slots, seat holds or the booking track out of `transaction`, or add
  the scheduling service (C3b).
- Reserve, confirm and release as extension methods, with expiry (C3b,
  task.md Z5).
- A Hub button to make a step, or any deal action panel (Q6; C6).
- Fill the Hub's quote form from the listing's extension (C6, AG-48).
- `terms_from_listing` and "Agree as listed" (C3c).
- Widgets (C4), events to an extension service and deal grants (moved out
  of M08C).
- Third-party extensions or a runtime registry.
- The field types `location`, `file-pointer`, `record-pointer` (Q8).
- Any wire-reachable `transaction` method.

# M08C slice C3a — review

*Reader: the developer who fixes the C3a plan or code, and the project
owner. This file holds every review finding for slice C3a (parts C3a1 and
C3a2). Plan review findings are P1, P2, …; code review findings are R1, R2,
…. Ids are never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code), 2026-11-20

Checked [slice-c3a-implementation-plan.md](slice-c3a-implementation-plan.md)
on branch `feat/m08c-c3a-deal-extension-frame` at `e359686a` (the plan
commit, on top of `main` at `e11fed33`). `git pull` ran and the branch was
already up to date. The checkout is the main one, not a worktree. The plan
was written by Claude Opus 5.5; this round runs on Claude Sonnet 5.5, so the
reviewer is a different model from the author.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan; task.md and status.md of
M08C; both change docs named in the prompt (roym-extensions: the criteria,
"Design", "Deviations"; roym-multilingual: ML-28, the rule "Commitment kinds
and extension fields", "Deviations"); the agreements doc where the plan
cites it ("The `details` block", open question 3, the state table); the C1
and C2 review files (kinds of findings before). task.md's C3a section names
the same two change docs as the prompt, so no change doc is missing from the
list.

**Summary: 17 findings: 0 blocker, 4 major, 13 minor.**

### What was checked and found correct

- **Line counts and file names.** `sync.rs` 717, `app.rs` 688,
  `quote_ops.rs` 749, `backup.rs` 766, `agreement_ops.rs` 561, `deal.rs` 600,
  `listing.rs` 584: all match the plan's table (section 7.8). I spot-checked
  19 `file:line` links of the plan (`disclosure.rs:105/:431/:456`,
  `fulfilment.rs:70`, `app.rs:284/:477/:549`, `record.rs:18`,
  `receipts.rs:23`, `disclosures.rs:33`, `fulfilment_ops.rs:579`,
  `transition.rs:47`, `listing.rs:315`, `render.ts:28`, `quote.ts:99`,
  `labelled` in `dom.ts:55`): all hit the named item.
- **Section 2.2 facts.** Point 1 (`SCHEMA_VERSION` 6, eight card types),
  point 2 (no deal code reads `BookingMode`; the only readers are
  `ListingPayload::validate`, the directory index and filter, and the Hub
  editor), point 4 (the Hub's quote form never passes `listingId` or
  `slotId`, `messages.ts:1009`), point 5 (three copies of the scheduling
  rule), point 6 (`file_quote_card` refuses on `validate()`, `sync.rs:526`),
  point 10 (`backup.rs:735` has no `deal-decline` arm): all true on this
  branch.
- **Q2 facts.** `toml` is a dev-dependency of `roym_core` only
  (`Cargo.toml:24`). The taxonomy pattern (`include_str!` at
  `taxonomy.rs:19`, the Hub import at `taxonomy.ts:1`) is real, and the
  relative path from `ui/src/extensions/` has the same depth as from
  `ui/src/taxonomy/`.
- **The `BookingMode` caller list.** `git grep` finds the same files as the
  plan: `listing.rs`, `listing/tests.rs`, `wire_words.rs`, `directory.rs`,
  `search_ops.rs` and its tests, `listing_ops.rs`, `editor.ts`, and the
  fixtures `fixtures.rs:96`, `trust_harness.rs:187/:544`,
  `listing_verify_good.json`. No fixture is missing, and no `.rs`, `.ts` or
  `.json` file under `apps/` or `test-components/` names a booking mode.
- **`extension-unknown` is a new code.** `DealError` has no such code now.
- **The wire list is right.** `directory.search` is wire-reachable
  (`SearchQuery` is serde, no `deny_unknown_fields`); an old node that gets
  `extension` ignores it.
- **Component size plan.** `directory` links only the shape check, as the
  plan says (checked in `search_ops.rs`, `publication_ops.rs`).

### View on the owner questions

I do not answer for the owner. My view on each one:

- **Q1 (split).** (a). The two parts have different callers and different
  review questions. I would not merge them.
- **Q2 (JSON in `roym_core`).** (a). It is the taxonomy pattern, with no new
  dependency and no build script. Two consequences the plan does not list:
  task.md's Z3 row says "TOML in each extension's crate" (P17), and the
  loader rules are not complete (P10).
- **Q3 (listing names its extension).** (a). It removes the two names for one
  fact. It needs the answers of P13, P14 and P15 first (what an old listing
  becomes, how the query string is checked, and the rule between `booking`
  and the extension). One cost is real: the directory can no longer ask for
  "plain enquiry listings only". This is acceptable; say it in the doc.
- **Q4 (one-shot needs no service).** (a). Nothing in M08C calls a service
  method. But the plan says `roymctl` can make a one-shot quote, and it
  cannot today (P7).
- **Q5 (step `data` is hidden).** (a). One rule for every deal record is
  right. It needs a size limit for `data` (P4).
- **Q6 (`scheduling/started`, `roymctl` only).** (a). It closes the backlog
  row for `roymctl` users and needs no new Hub screen. Two notes: the step
  does not change the progress snapshot (the plan says so; the backlog row
  text must say "a card, not a state" so nobody reads it as `in-progress`),
  and the row key needs the role (P5).
- **Q7 (demo).** (a) for C3a1 is right: nothing a viewer sees changes. For
  C3a2 I would not accept `none` without an explicit owner "yes". AGENTS.md
  ("Mandatory Demo Close-out") lists "a new command" as something a viewer
  can see, and C3a2 adds `roymctl roym step add`. If the owner accepts (a),
  the plan should say that it is an exception to that rule, and C3b's plan
  must carry the story that shows the step card.
- **Q8 (seven scalar field types).** (a). `location`, `file-pointer` and
  `record-pointer` have their own rules and no M08C manifest needs them. The
  plan should also split identifier text from free text (P6).
- **Unrecorded decision, new Q9.** D-C3a-6 makes `agreement.accept` refuse a
  quote when this node gives a commitment whose extension it does not know.
  EX-2 says the person "may still sign". The rule is sensible (a giver must
  run its extension's steps), but it is a product choice that the owner has
  not been asked about and the deviation list does not record (P11). My view:
  keep the rule, ask the owner, and record the deviation.

### Findings

- [x] **P1 — major.** *Where:* D-C3a-5 (the last paragraph), step 3, and
  the callers of `DealTerms::validate`.
  *Problem:* The plan says `validate` "keeps every other rule and no longer
  refuses an unknown extension", and adds `check_extensions` as the new check.
  It does not say whether `validate` still runs the registry check for a
  *known* extension. `validate()` is not called in three places only. It is
  the last step of `reveal_terms_strict` (`deal/sd.rs:384`), which every
  load of a held quote, agreement receipt and settlement record goes through
  (`load_quote` at `disclosures.rs:100`, which every accept, settle,
  fulfilment and presentation call uses; the backup import at
  `backup/disclosures.rs:138`; `quote_ops.rs:599`; `sync.rs:519`). Besides
  that it is called directly at `quote_ops.rs:482` and `sync.rs:526`.
  *Failure case:* the implementer reads "validate no longer checks the
  extension" and calls `check_extensions` only at the three places the plan
  names (`quote.set`, `file_quote_card`, `accept`). Then `reveal_terms_strict`
  stops checking `scheduling/slot@1` details. A quote restored from a backup,
  or an agreement receipt revealed on the counterparty, is accepted with a
  `slot_id` that does not match the window. Today's protection is lost on
  every path that is not named.
  *Fix:* State the contract in D-C3a-5. Recommended: `validate()` runs the
  registry check for every commitment through `registry()` (a known extension
  is checked, an unknown one passes the shape rules only), so every existing
  caller keeps today's protection and none needs a change. `check_extensions`
  is then only for the callers that need to know *which* commitments were
  `NotChecked` (`quote.set`, `accept`, and the Hub's data). Add a unit test
  that `reveal_terms_strict` refuses a scheduling quote whose `slot_id` does
  not match its window, and that it reveals one with an unknown extension.
  *Status:* Fixed as proposed. D-C3a-5 states the contract: `validate()` calls `check_commitment_extension(registry(), c)` for every commitment, so every caller of `validate` and `reveal_terms_strict` keeps today's protection; `check_extensions` is only for callers that need the `NotChecked` list. Two `reveal_terms_strict` tests in 7.1; a new exit criterion.

- [x] **P2 — major.** *Where:* D-C3a-5 (the rule order and the `NotChecked`
  rule), D-C3a-4, `deal.rs:456` to `:474`.
  *Problem:* Today the limits of `details.extra` (16 items; label 64 bytes;
  value 512 bytes, `MAX_DETAILS_EXTRA*`, `deal.rs:32` to `:34`) are checked
  only inside `validate_scheduling_slot_details`. The plan moves that
  function into a named check (`slot-matches-window`) and lists no generic
  rule for `extra`. So `extra` of a one-shot commitment and of an unknown
  extension would have no count or length limit. The plan also says an
  unknown extension is checked for "the `details` size limit". That limit
  (`MAX_DETAILS_DISCLOSURE_BYTES`, 8 KiB) is not enforced in `deal.rs` at all;
  it is enforced only in `sd.rs` (conceal and reveal) and `quote_ops.rs:265`.
  *Failure case:* a one-shot quote carries 500 `extra` entries of 10 bytes.
  `quote.set` accepts it (5 KB is under 8 KiB), the Hub's field view draws 500
  lines, and the agreements doc rule "0 to 16; label 64 bytes, value 512
  bytes" (`:441`) is broken for every extension except scheduling.
  *Fix:* Add one generic function, `check_details_shape(details, ext)`, run
  first for every commitment with an extension (known or unknown): `extra`
  count and lengths, `details.extension == extension.id`, and the schema-name
  shape. Move the `extra` loop out of the scheduling function into it. Either
  drop "the details size limit" from the unknown rule (sd.rs enforces it), or
  say where it is checked in `deal.rs` and with which constant. Tests: a
  one-shot commitment with 17 `extra` items, one with a 65-byte label, one
  with a 513-byte value, and the same three for an unknown extension. Each
  gets `extension-shape`.
  *Status:* Fixed as proposed. New `check_details_shape` runs first for every commitment with an extension; the `extra` loop moves into it. "The details size limit" is dropped from the unknown rule: D-C3a-5 says it stays in `sd.rs` and `quote_ops.rs:265`. Six `extra` tests in 7.1.

- [x] **P3 — major.** *Where:* D-C3a-5 (`check_visible_extension`),
  section 2.2 point 5, step 3, `presentation/validate.rs:395` to `:451`.
  *Problem:* The plan says the presentation checker calls
  `check_visible_extension(registry, resource_kind, kind, ext)`. That
  function has no `details` and no `window` parameter. But the presentation
  checker does more than check the kind. In `validate_commitment_details_partial`
  it reads `details` (which may be absent), reads `window` (which may be
  hidden or `null`), calls `validate_scheduling_slot_details`, and pushes the
  names `details_matches_extension` into `not_checked`. A function that sees
  neither `details` nor `window` cannot replace it. Also, with the new
  registry a *known* extension whose `details` are shown can now be
  schema-checked in a presentation even when the window is hidden (only
  `slot-matches-window` needs the window).
  *Failure case:* the implementer follows the signature, makes the
  presentation checker call the kind check only, and deletes the scheduling
  branch. A presentation of a scheduling commitment with a wrong `slot_id`
  is now `verified`. The existing cases in
  `presentation/tests/validation.rs:112` to `:149` fail, or worse are
  deleted.
  *Fix:* Add a third function to D-C3a-5, for example
  `check_partial_details(registry, ext, details: Option<&Details>, window:
  Option<Option<&TimeWindow>>, currency) -> Result<PartialCheck, DealError>`
  where the result is `Checked`, `NotChecked` (unknown extension) or
  `Partial` (a part is hidden: push `details_matches_extension`). Say which
  of today's `not_checked` entries stay (`details_matches_extension` for a
  hidden window or hidden details; the unknown extension case). Keep the
  mapping of errors to `PresentationError::FieldInvalid`. Test list in 7.1:
  scheduling with a good window, a wrong `slot_id`, a hidden window, hidden
  details; an unknown extension with visible details.
  *Status:* Fixed as proposed, with one change: `SignedTerms::validate_visible` keeps a kind-only function, renamed `check_extension_kind`, and the presentation checker gets `check_partial_details` with the three results. D-C3a-5 lists the `not_checked` entries that stay; 7.1 lists the six presentation cases.

- [x] **P4 — major.** *Where:* D-C3a-10, D-C3a-11, D-C3a-12, D-C3a-13.
  *Problem:* A step's `data` has no size limit. `step_type` is capped (64
  bytes), but `data` is a free JSON object, and for an unknown extension the
  field list does not exist to bound it. The row key is
  `step:<agreement>:<commitment_id>:<step_type>`, so a party can add one row
  per distinct `step_type` string for an unknown extension. The only limit
  is `MAX_CARDS_PER_CONVERSATION` (2,000, `transaction.rs:85`) and the card
  size.
  *Failure case:* a counterparty with an agreement sends 2,000 step cards
  with step types `x/a`, `x/b`, … for an extension this node does not know.
  Each is filed as `checked: false`. The conversation's card cap is used up
  and every later card (a payment request, a decline) is refused with the
  cap message. A step with a 90 KiB `data` also passes the filing check.
  *Fix:* (1) Define `MAX_STEP_DATA_BYTES` (use the same 8 KiB as `details`)
  and check it in `step.add`, in the filing, and in the import check of
  D-C3a-13 (`conceal_hidden` does not limit a single field). (2) Cap the
  steps per agreement (for example 64) in `step.add` and in the filing, and
  count only rows of that agreement. (3) Tests: a step with 8 KiB + 1 byte
  of `data` is refused at add, at filing and at import; the 65th step of an
  agreement is refused at filing. (4) Add both limits to the deviation and
  backlog text if the owner wants different values.
  *Status:* Fixed as proposed: `MAX_STEP_DATA_BYTES` (8 KiB) and `MAX_STEPS_PER_AGREEMENT` (64) in D-C3a-10, checked at add, filing and import (D-C3a-11 to -13), new codes `step-data-too-large` and `too-many-steps`, tests in 7.1 and 7.2, and both values in the backlog row.

- [x] **P5 — minor.** *Where:* D-C3a-11 step 5, D-C3a-12 (the last sentence).
  *Problem:* The row key does not contain the role of the signer. The
  manifest format allows `by: either` and `by: receiver` (D-C3a-1). With a
  key of agreement, commitment and step type only, the first step filed for
  a type blocks the other party's step of the same type: "refuses a
  different record for the same key". `scheduling/started` is `giver` only,
  so nothing ships that hits this, but the format promises it works.
  *Failure case:* a later extension declares `{ "type": "docs/received", "by":
  "either" }`. Alice adds it; Bob's own step is refused as "a second step of
  this type".
  *Fix:* Put the role in the key: `step:<agreement>:<commitment_id>:<type>:<role>`
  and state that "once" means once per role. Update D-C3a-13's import check
  (the key's role equals the payload's `role`) and the backlog row text
  ("each step type happens once per role of a commitment").
  *Status:* Fixed as proposed: the key is `step:<agreement>:<commitment_id>:<type>:<role>`, "once" means once per role (D-C3a-1, -11, -12), the import checks the key's role, and the backlog row text says "once per role". Test with a test-only `by: either` step.

- [x] **P6 — minor.** *Where:* D-C3a-2 (the `text` row and `choice`), D-C3a-8
  (`extra`), D-C3a-1 (`listing_id`, `slot_id` typed `text`), section "How
  this is shown to an end user".
  *Problem:* Two gaps in how text from the other party is drawn.
  (a) A `text` or `long-text` field and every `extra` value are free text
  inside a quote that has one `lang` tag (roym-multilingual, "Language-tagged
  free text": one tag covers every free-text field of the record). The plan
  draws them "as data" with the language unknown, and outside
  `recordText`, so they get no `lang` attribute and no machine-translation
  action. A term hidden in `extra` that a person cannot read is accepted
  after the person read a translation of the other fields.
  (b) `listing_id` and `slot_id` are identifiers, not free text, but the only
  text type is `text`. The end-user section says the card shows "the chosen
  slot", but the card would show two lines of raw ids (`Listing id: lst_…`,
  `Slot id: slot_…`). That helps nobody.
  *Fix:* (a) Draw `extra` and `text` fields through the same record-text
  path as the quote's other free text (the quote's `lang`), or record in the
  backlog that they are outside the translate hook. (b) Add a field type
  `code` (identifier, not shown through language rules, drawn as plain data)
  and use it for the two scheduling fields, or give a field `hidden_in_view:
  true` so the generic view skips ids; and correct the end-user bullet to say
  what the card really shows.
  *Status:* Fixed. (a) Not routed through `recordText`: `details` has no author translations, so drawing it inside the version switch would give `extra` a translation's tag. The field view is drawn once, with the quote's original `lang` on `text`, `long-text` and `extra` (D-C3a-2, -8); machine translation of `details` is a new backlog row. (b) The owner chose a field type `code` (Q8 answer); the scheduling fields use it, and the end-user bullet now says the card shows two reference codes.

- [x] **P7 — minor.** *Where:* Q4 (the sentence "Who sets `one-shot`"),
  D-C3a-15, step 3, `apps/roymctl/src/commands/roym/transaction.rs:358` to
  `:365`.
  *Problem:* (a) `roymctl roym transaction quote` builds the scheduling
  extension from `syneroym_roym_core::deal::SCHEDULING_EXTENSION` and
  `SCHEDULING_SLOT_SCHEMA`. Step 3 moves both constants to `extension.rs` and
  lists only the `roym_transaction` tests (`cards.rs:10`). `roymctl` stops
  compiling. D-C3a-15 says C3a1 has "none" for `roymctl`. (b) Q4 says a
  one-shot quote comes "from `roymctl` and tests", but `build_quote_commitments`
  makes a one-shot commitment only as a `goods-or-service` with no
  extension; there is no flag for `one-shot`.
  *Fix:* Add `apps/roymctl` to step 3's import fixes. Either add a
  `--one-shot` flag to the quote command (one test in `cli_args.rs`) or
  change Q4's sentence to "tests, and `roymctl` through a raw `quote.set`
  JSON". Exit criterion 2 ("no code outside `extension.rs` names
  `scheduling/slot@1`") must also name `roymctl` and the Hub's
  `quote_terms.ts`.
  *Status:* Fixed. (a) Step 3 and D-C3a-15 name the `roymctl` import fix, with a `git grep` check. (b) No `--one-shot` flag: Q4 now says only tests make a one-shot quote in C3a, and C3c's `terms_from_listing` is the first real caller; a flag would be a new visible command for a record no person makes yet. Exit criterion 2 names `roymctl` and `quote_terms.ts`.

- [x] **P8 — minor.** *Where:* D-C3a-4 (`registry()` and its comment).
  *Problem:* The comment says the `expect` "cannot fire in a shipped build".
  The workspace lints `expect_used = "warn"` (`Cargo.toml:241`), and
  `roym_core` allows `expect` only under `cfg(test)` or the `test-host`
  feature (`lib.rs:3`). `mise run verify` runs clippy with `-D warnings`. A
  plain `expect` in `registry()` fails that gate. Nothing in `roym_core`
  parses embedded JSON at run time today (the Rust side of the taxonomy only
  parses in tests), so there is no example to copy.
  *Fix:* Say how: `#[expect(clippy::expect_used, reason = "…")]` on that one
  function (the suppression ratchet in `xtask/src/lint_suppressions.rs`
  covers `too_many_lines` only; check that it does not reject this), or
  return `Result` from `registry()`. Keep the unit test
  `every_shipped_manifest_loads`.
  *Status:* Fixed: `#[expect(clippy::expect_used, reason = …)]` on `registry()` only, after the pattern of `key_store.rs:44` and `keys.rs:255` (they use `allow`); step 2 checks that `check-lint-suppressions` accepts it. The comment no longer claims the `expect` cannot fire.

- [x] **P9 — minor.** *Where:* D-C3a-5 (rule 2), D-C3a-6 (`listing.set`),
  7.1 `insight_manifest_loads_and_cannot_be_named`.
  *Problem:* The test says that a commitment naming a loaded `insight`
  manifest is refused with `extension-unknown`. But the rules say a *known*
  extension that does not list the resource kind gives
  `commitment-kind-not-supported`, and an `insight` manifest has no
  `resource_kinds`, so by the rules the answer is
  `commitment-kind-not-supported`. For a listing the plan says
  `extension-unknown`. The two paths give two codes for one fact, and the
  plan does not choose.
  *Fix:* Choose one. Simplest: `Registry::get` for the deal and listing
  paths returns only `workflow` manifests (add `Registry::workflow(ext)`), so
  an `insight` id is unknown everywhere and gives `extension-unknown`. Say so
  in D-C3a-5 and keep the test as written.
  *Status:* Fixed as proposed: `Registry::workflow(ext)` replaces `get` on the deal and listing paths, so an `insight` id is `NotChecked` and gives `extension-unknown` everywhere. The test is kept and now also checks `workflow`.

- [x] **P10 — minor.** *Where:* D-C3a-4 (`Schema::check`, "`Registry::parse`
  checks"), D-C3a-2, D-C3a-3.
  *Problem:* Four loose ends in the schema part.
  (a) `Schema::check(&self, data)` has no `currency`, but its doc comment
  says "`currency` is the deal's, for `money` fields". A `money` value is an
  integer of at least 0, so no currency is needed. Drop the sentence, or add
  the parameter and say what it checks.
  (b) The loader checks list has no rule for the field limits of D-C3a-2:
  `max_len` required for `text` and `long-text` and at most 512 and 2048;
  `min` at most `max`; `choices` not empty, each a code `[a-z0-9-]`, no
  duplicates; `required` only where it means something. A manifest with
  `max_len: 100000` would load and defeat the card size rules.
  (c) D-C3a-3 does not say how the id is made from `scheduling/slot@1`. The
  example `ext-scheduling-slot-listing-id` drops the `scheduling/` prefix and
  the `@1`. Is the version part of the id? If not, `slot@1` and `slot@2`
  share labels for fields that may change meaning.
  (d) The derived ids can collide: extension `a-b` with schema `c`, and
  extension `a` with schema `b-c`, give the same prefix `ext-a-b-c-`.
  *Fix:* Add the four rules to `Registry::parse` and its failing tests; write
  the exact id function (and decide on the version) in D-C3a-3; make `parse`
  refuse a manifest set whose derived ids are not unique.
  *Status:* Fixed. (a) The `currency` sentence is replaced: a `money` value needs no currency. (b) Every limit is a load rule (D-C3a-2 table, D-C3a-4 list), with a failing test each. (c) D-C3a-3 writes the exact id function; the version is not part of the id, with the author rule that a changed meaning needs a new field name. (d) `parse` refuses a registry with a repeated derived id; test in 7.1.

- [x] **P11 — minor.** *Where:* D-C3a-6 (`agreement.accept`), section 9
  (deviations), section "How this is shown to an end user" (C3a1, third
  bullet), `roym-extensions` EX-2.
  *Problem:* EX-2 says a party without the extension "may still sign". The
  plan makes `agreement.accept` refuse with `extension-unknown` when this
  node is the giver of an unknown-extension commitment. The deviation list
  in section 9 does not record this, the end-user bullet says "The person may
  still accept" without the exception, and no owner question covers it. The
  rule is reasonable (a giver must run the extension's hooks and steps), but
  it changes a criterion of the change doc.
  *Fix:* Add it as owner question Q9, with the recommendation to keep the
  rule. Add it to the roym-extensions `## Deviations` list in section 9. Add
  the exception to the end-user bullet ("unless this person is the one who
  gives that part of the deal; then Roym asks them to update first").
  *Status:* Fixed: owner question Q9 (answered (a): keep the refusal), the deviation is written in the extensions doc in this commit, and the end-user bullet names the exception. One addition: the Hub shows a plain message for this refusal (`messages-accept-extension-unknown`, one branch in `messages.ts`), so the bullet is true.

- [x] **P12 — minor.** *Where:* "Stays the same" (section 4), step 4.
  *Problem:* The plan moves one seat-code test to the registry:
  `reserve_counterparty_slots` (`agreement_ops.rs:267`) asks
  `has_reserve_hook`. Three more places in `transaction` still match
  `ResourceKind::ServiceInSlot` and read `details.data.slot_id`:
  `reserve_author_holds` (`decide.rs:418` to `:431`), the release at expiry
  (`agreement_ops.rs:512` to `:515`), and the `fulfil:` kind check on import
  (`backup/sections.rs:373`); and `validate_and_apply_slot`
  (`quote_ops.rs:151`) reads the slot from `catalog`. After step 4 the
  counterparty side asks the registry and the author side does not: two
  answers to one question in one slice.
  *Fix:* Leave all of them as they are until C3b (the result is the same for
  scheduling, and the diff is smaller), and drop the change at `:267`. Or
  change all four and add a test per site. Either way list the four sites in
  section 4 so the C3b plan finds them.
  *Status:* Fixed with the first option: the change at `agreement_ops.rs:267` is dropped, the seat code is not changed in C3a, and section 4 lists the five sites (the reviewer's four plus `quote_ops.rs:427`) for C3b's plan.

- [x] **P13 — minor.** *Where:* Section 4 ("Wire changes": "stored listings
  of version 3 are not read"), `DIRECTORY_SCHEMA_VERSION` 5 to 6.
  *Problem:* The statement is not what the code does. `ListingPayload` and
  `BookingTerms` have no `deny_unknown_fields` (`listing.rs`), so a stored or
  held listing with `booking.mode` still parses: the old key is ignored and
  the listing becomes a listing with no extension. A former `slots` listing
  silently stops naming scheduling, and it still verifies (the envelope bytes
  are not changed). The version bump gates only the catalog backup import
  (`roym_catalog/src/app/backup.rs:63`); I found no start-up refusal.
  *Failure case:* a developer node with an old `slots` listing is upgraded.
  The listing stays `active`, now plain, and its `availability.*` rows are
  unused.
  *Fix:* State the real behaviour (no migration, per the owner's rule, so
  development nodes start empty). Add one test that an old listing with
  `booking.mode` parses as "no extension", so the behaviour is known and not
  a surprise. Do not claim "not read".
  *Status:* Fixed as proposed: section 4 and D-C3a-9 state the real behaviour (an old listing parses with no extension; the bump gates only the catalog backup import), and the test `old_listing_with_booking_mode_has_no_extension` is added. `deny_unknown_fields` is not added, because an older node would then refuse a newer node's listing.

- [x] **P14 — minor.** *Where:* Section 4 ("Directory search"),
  `search_ops.rs:296`, `directory.rs:172`.
  *Problem:* `SearchQuery.extension` is a plain string on a wire-reachable
  method. The old field was an enum, so the value was always one of three
  words. Categories go through `category_tokens` / `normalize_category`
  (`search_ops.rs:284`). The plan does not say what happens to a long or odd
  string. Also there is no value that means "plain enquiry listings only".
  *Fix:* Check the query value with `listing::valid_token(.., MAX_EXTENSION_ID_LEN)`
  and answer `invalid_params` otherwise; add a test. State in the
  directory doc line that "no extension" cannot be asked for.
  *Status:* Fixed as proposed: `directory.search` checks the value with `valid_token(.., MAX_EXTENSION_ID_LEN)` and answers `invalid_params`; test `search_refuses_an_extension_filter_that_is_not_an_id`; "no extension" cannot be asked for, said in Q3 and in a backlog row. The value is not checked against the registry, so a directory can index listings of extensions it does not know.

- [x] **P15 — minor.** *Where:* D-C3a-9, step 5 (the e2e line), `roym-hub.spec.ts:344`.
  *Problem:* Two points. (a) Before this change, `slots` lived inside the
  `booking` block, so a slot listing always had lead time, cancellation
  window and per-booking maximum. After the select moves out, a listing can
  name `scheduling` with no `booking` block, or have a `booking` block with
  no extension. The plan states no rule. (b) The plan says the e2e line "turns
  the booking block on to set the mode". The test only enables the block, so
  the block's fields are valid; it never sets or asserts the mode (the
  assertions at `:351` to `:362` check only that numbers are integers). The
  line needs no change when the select moves out.
  *Fix:* (a) Choose a rule and test it: recommended, extension `scheduling`
  requires `booking`; `booking` without an extension is allowed. Say so in
  `ListingPayload::validate`, the Hub editor (disable "slots" while the block
  is off, or switch it on) and the tests. (b) Remove the e2e change from step
  5 and 7.6, or say why it is needed.
  *Status:* Fixed as proposed. (a) `scheduling` requires `booking` (new `booking-required`); `one-shot` or no extension may have either; in the Hub, picking "slots" switches the booking block on and locks it. (b) The e2e change is removed from step 5 and 7.6.

- [x] **P16 — minor.** *Where:* D-C3a-13, 6.2 step 3, 7.3 (parity).
  *Problem:* (a) The plan says `transaction.thread` builds a step card's
  plain `data` "as for a fulfilment card (`disclosures::rebuild_card_data`,
  called at `backup.rs:750`)". `rebuild_card_data`
  (`backup/disclosures.rs:263`) is used only on import. The live path uses
  `plain_view` (`app.rs:496`, `sync.rs:564`, `sync.rs:682`). (b)
  `rebuild_card_data` has its own `match` by card kind (five arms, then
  `_ => {}`), separate from the `match` at `backup.rs:735`. The plan names
  only the second one. A `step` arm missing there leaves an imported step
  card with `data: None` and no error. (c) The bundle parity file
  (`dual_build_parity/bundles.rs`) and `roym_restore_e2e.rs` list the
  fulfilment methods and sections; a new `steps` section must appear in
  both.
  *Fix:* Correct the sentence in 6.2 step 3, add the `step` arm to both
  matches, and add one test that an imported step card has its plain `data`.
  Add `bundles.rs` and `roym_restore_e2e.rs` to the C3a2 test list.
  *Status:* Fixed as proposed: 6.2 step 3 names `plain_view` for the live paths; D-C3a-13 adds the `step` arm to both `match` statements, with the test `imported_step_card_has_its_plain_data`; `bundles.rs` and `roym_restore_e2e.rs` are in the C3a2 tests (7.3, 7.4).

- [x] **P17 — minor.** *Where:* Section 9 ("Change docs ... not living
  docs"), Q2, task.md section 1 (Z3).
  *Problem:* Q2 changes the manifest format and place. task.md's shared-part
  row Z3 still says "A TOML manifest in each extension's crate, read at build
  time", and the "Extension manifest" sketch in the roym-extensions doc shows
  TOML and a string `version`. Section 9 lists deviations for the change docs
  but not the Z3 row of task.md, and C3b's text says "scheduling crate".
  *Fix:* Add "update task.md Z3 and the C3b line about the manifest" to
  section 9 and to step 7 of 6.1. The roym-extensions deviation already
  covers the sketch.
  *Status:* Fixed: task.md Z3 and the C3b line are updated in this commit (owner answer to Q2), and section 9 says so.

### Round 1 close-out — Claude Opus 5.5, 2026-11-20

All 17 findings are fixed in the commit after `45e38bfb`. P3, P6, P7,
P8 and P12 differ from the proposal in a detail; each status says how. The
owner answered Q1 to Q9. One new point came from checking P7 and Q7: the
plan said demo stories drive only the Hub, but stories 10, 20 and 30 run
`roymctl` from a shell part (`demo/lib.sh:49`, `demo/steps.ts:15`). So the
C3a2 demo is a new owner question, Q10, with a story as the
recommendation.

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code), 2026-11-20

This round checks the fix of round 1 (commit `afac7bd4`, on top of
`45e38bfb`) against the plan and the code. The plan author is Claude Opus
5.5; the reviewer is Claude Sonnet 5.5. I read the whole plan again (1,677
lines), the new text of `task.md` (Z3, C3b) and the extensions doc deviation,
and I checked the new `file:line` links and claims the fix added.

**Summary: P1 to P17 are fixed and verified. 3 new findings: 0 blocker, 1
major, 2 minor. 3 open.**

### Verification of P1 to P17

- **P1 verified.** D-C3a-5 now states the contract: `validate()` calls
  `check_commitment_extension(registry(), c)` for every commitment.
  `check_extensions` is only for callers that need the `NotChecked` list.
  Two `reveal_terms_strict` tests are in 7.1 and a new exit criterion
  exists. The caller list matches the code. (But see P18: the rule list that
  `validate()` now runs has a gap.)
- **P2 verified.** `check_details_shape` runs first and owns the `extra`
  limits; the 8 KiB rule is said to stay in `sd.rs:174`/`:329` and
  `quote_ops.rs:265`. Six `extra` tests are listed.
- **P3 verified.** `check_extension_kind` (kind only, for `validate_visible`)
  and `check_partial_details` (three results) are separate. The `not_checked`
  entries that stay are named, and the six presentation cases are in 7.1.
  The existing cases at `presentation/tests/validation.rs:112` to `:149`
  are kept.
- **P4 verified.** `MAX_STEP_DATA_BYTES` and `MAX_STEPS_PER_AGREEMENT` are
  checked at add, filing and import, with codes and tests. For a known
  extension the rows are bounded by commitments, types and roles; only an
  unknown extension can reach the cap, which is the case that mattered.
- **P5 verified.** The key has the role (`SettlementRole` is `Giver` or
  `Receiver`, `deal.rs:106`); import checks the key's role; the test-only
  `by: either` step is listed.
- **P6 verified.** `code` is the eighth type (the owner chose it); the view
  uses the quote's original `lang` for `text`, `long-text` and `extra`;
  machine translation of `details` is a backlog row.
- **P7 verified.** `roymctl` is named in step 3 and in D-C3a-15; there is no
  `--one-shot` flag, and Q4 says so. Exit criterion 2 names `roymctl` and
  `quote_terms.ts`. The lines `transaction.rs:359` to `:362` are right.
- **P8 verified.** `#[expect(clippy::expect_used, reason = …)]` on
  `registry()`. `xtask/src/lint_suppressions.rs` handles `too_many_lines`
  only, so it will not reject this; step 2 still says to run the check.
  The cited precedents `key_store.rs:44` and `keys.rs:255` exist and use
  `allow` (the plan says so).
- **P9 verified.** `Registry::workflow` is the lookup for deal and listing
  paths, so an `insight` id is unknown everywhere.
- **P10 verified.** All four loose ends are rules in D-C3a-2/-3/-4 with
  tests; the id function is written out and the version is not part of the
  id.
- **P11 verified.** Q9 is answered (a); the deviation is in the extensions
  doc (`git diff` shows one added line there); the end-user bullet has the
  exception; the Hub message `messages-accept-extension-unknown` has its
  branch. The branch place `messages.ts:686` to `:690` is the right one.
- **P12 verified.** The change at `agreement_ops.rs:267` is dropped and
  section 4 lists the five sites. I checked each line: `decide.rs:427`,
  `agreement_ops.rs:513`, `backup/sections.rs:373`, `quote_ops.rs:150` and
  `:427` are right.
- **P13 verified.** Section 4 and D-C3a-9 say what really happens, and
  `old_listing_with_booking_mode_has_no_extension` is a listed test. The
  catalog gate at `backup.rs:63` is right.
- **P14 verified.** The id check and its test are named.
- **P15 verified.** `scheduling` requires `booking` (see P20 for one detail);
  the e2e change is removed from step 5 and 7.6.
- **P16 verified.** 6.2 step 3 names `plain_view`; both `match` statements
  get a `step` arm; `bundles.rs` and `roym_restore_e2e.rs` are in 7.3/7.4.
- **P17 verified.** `task.md` Z3 and the C3b line say JSON in
  `roym_core/extensions/`.

New facts the fix added, spot-checked and correct: `demo/lib.sh:49`
(`rc1`, `rc2`), `demo/steps.ts:15` (`runScript`), `en.ftl:447` to `:449`,
`catalog.test.ts:226` to `:234`, `transition.rs:47` (`is_final`),
`app.rs:284`, catalog `SCHEMA_VERSION` 3 at `app.rs:23`, `deal.rs:106`.

### View on Q10 (the C3a2 demo; still open for the owner)

I do not answer for the owner. My view: (a), the story `60-roym-step`. The
rule in AGENTS.md ("Mandatory Demo Close-out") counts a new command as
visible, and `roymctl roym step add` is one. Story number 60 is free (the
folder ends at `50-roym-deal`). One cost the plan does not list: `roymctl`
has no command that reads a listing's slots, and its `quote --slot` needs
the slot id and the window (`transaction.rs:341` to `:364`). The slot id is
derived from the listing id and the window (`listing::derive_slot_id`), and
the Hub creates the slot from local date and time fields. So the browser
part must write the listing id and the window to a file for the shell part,
or the story must use a fixed window. This adds a few dozen lines to the
"about 150" estimate. It is not a reason to choose (b).

### Findings

- [ ] **P18 — major.** *Where:* D-C3a-5 ("Rules of
  `check_commitment_extension`, in order", rules 1, 3 and 5), the sentence
  that `validate_commitment_kind` is replaced, `deal.rs:349` to `:390`,
  `deal/sd.rs:264`.
  *Problem:* The new rule list drops two refusals that
  `validate_commitment_kind` makes today, and rule 1 contradicts rule 5.
  (a) Rule 1 says: no extension means the result is `None`. But today a
  `service-in-slot` commitment with no extension is refused
  (`deal.rs:356` to `:361`, "service-in-slot requires scheduling extension
  v1"). Rule 5 says "`service-in-slot` needs an extension", but rule 5 is
  never reached when the extension is absent, because rule 1 returns first.
  (b) The pairs of resource kind and commitment kind that give
  `kind-mismatch` (`deal.rs:384` to `:387`: `service-in-slot` or
  `goods-or-service` with kind `currency`, and `currency` with kind
  `fulfilment`) are in no rule. The test at `deal/tests.rs:181` would
  catch (b) when the code is written, but nothing in the plan says where
  the check lives, and `check_extension_kind` (the replacement for
  `validate_commitment_kind` in `validate_visible`) takes the same inputs.
  *Failure case:* the implementer follows the rules as written. A hand-built
  quote with a `service-in-slot` commitment, no extension and no `details`
  passes `validate()` on the counterparty's node, and a `goods-or-service`
  commitment with kind `currency` is accepted. Nothing fails until a later
  step reads `slot_id` ("missing slot_id" at `agreement_ops.rs:267`), or a
  settlement rule meets a commitment of the wrong kind.
  *Fix:* Add a first rule: the resource kind and kind pair must be valid
  (`kind-mismatch`), and make rule 1 "no extension: refused if the resource
  kind needs one (`service-in-slot`: `commitment-kind-not-supported`);
  otherwise no `details`, result `None`". Say that `check_extension_kind`
  runs the same two rules, and keep the extension id shape check that
  `validate_visible` makes at `sd.rs:265` to `:269`. Add three cases to
  7.1: `service-in-slot` with no extension, `goods-or-service` with kind
  `currency`, and `currency` with kind `fulfilment`, each for `validate` and
  for `validate_visible`.
  *Status:* Open

- [ ] **P19 — minor.** *Where:* Statuses of P1 to P17 in this file.
  *Problem:* session-strategy.md ("The review file") allows the statuses
  `Open`, `Fixed (<sha>)`, `Rejected: <reason>` and `Deferred: <link>`. The
  fix wrote "Fixed as proposed", "Fixed with the first option" and similar,
  with no sha. The fix commit is `afac7bd4`. The boxes are ticked, so the
  file is readable, but a later reader cannot find the commit from the
  status.
  *Fix:* Start each status of P1 to P17 with `Fixed (afac7bd4)` and keep the
  explanation after it. P3, P6, P7, P8 and P12 already say how they differ;
  keep that.
  *Status:* Open

- [ ] **P20 — minor.** *Where:* D-C3a-9 ("The rule between `booking` and
  the extension"), exit criterion 2 of C3a1, section 7.9 (directory size).
  *Problem:* `ListingPayload::validate` (in `listing.rs`) now has to know
  that the id `scheduling` needs a `booking` block. That is a match on an
  extension id outside `extension.rs`, and exit criterion 2 says no such
  code exists, except a list that does not name `listing.rs`. The check
  `git grep -n 'scheduling/slot@1\|"scheduling"'` in the same criterion
  would then find `listing.rs` if the id is written as a string. Using
  `registry()` instead would link the registry into every verifier of a
  listing, including `directory`, which D-C3a-6 and 7.9 say must stay
  free of it.
  *Fix:* Say that `listing.rs` uses the constant `SCHEDULING_EXTENSION` from
  `extension.rs` (a constant, no registry lookup), and add `listing.rs`
  to the exceptions of exit criterion 2 with that reason. State the reason
  in the plan: the rule is a shape rule that a stranger's directory must
  check without the registry.
  *Status:* Open

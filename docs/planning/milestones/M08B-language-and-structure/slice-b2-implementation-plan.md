# M08B slice B2 — Taxonomy, units, payment methods and attributes: implementation plan

*Reader: the developer (or AI) who implements slice B2. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan review round 1 done, 2026-11-07. Owner questions answered
2026-11-07. P-items folded in.**

**Size and PR.** M. One PR on `feat/m08b-b2-taxonomy`, with the commits of
section 6 in that order. The branch starts from `main` at `c44e055c` (B1b
is merged, [PR #351](https://github.com/syneroym/syneroym/pull/351)) and
already holds the plan commit. B2 adds the **base taxonomy**: one data file
in `roym_core` with category codes (with a parent), unit codes,
payment-method codes and attribute definitions. The Hub reads the same file
when it is built, shows a label for each code in English or Hindi, and
gets pickers for categories, units and payment methods in place of free
text boxes. The service checks that a unit and a payment method are
well-formed codes, as it already does for categories. B2 adds no JSON-RPC
method, no WIT file, no record field and no record version.

**Base.** Every file path and line number below was checked on `main` at
`c44e055c`. Links are relative to this folder.

**Parallel work.** B4a (substrate only) may run beside B2. It touches no
file that B2 touches ([task.md](task.md) section 4). B3a must wait for
B2: both change `ListingPayload` and the listing editor.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code or comments: no "M08B", "B2", "ML-7", "N-2" or
    "D-B2-3" in `.rs`, `.ts`, `.ftl`, `.json` files or test names. B2 also
    removes one such comment that is already there (section 2.2 point 9).
  - Comments never cite size rules.
  - The import clean-up pass over the Rust files you edit, the deferred
    backlog, the living-docs close-out and the demo close-out.
  - A `roym_*` crate changes, so run `mise run build:roym` before you trust
    a parity run (task.md section 2, "Records").
  - Every new Hub string goes through `t()` and the build check (B1b).
- [task.md](task.md): section 1 (Y1, Y2), section 2 ("Hub", "Records"), the
  "Demo" paragraph, "B2", section 4, section 6 (owner answers 3 and 5),
  section 7 (the N-2 row).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "B. Language-neutral structure", "Rules for every layer" (rule 3:
  "Structure before text"), "A. UI localisation" (the Hub mechanism),
  criteria ML-2, ML-7, ML-8, ML-9 and ML-28, "Deviations". Every B2
  decision and deviation is written there, and only there (section 3).
- Two more change docs, read for context only:
  - [demo-stories](../../changes/demo-stories/change.md): task.md's "Demo"
    paragraph names it. B2 extends story 40 under its rule "Link between
    slice plans, the demo and AGENTS.md". B2 meets no criterion of that doc
    and writes nothing there.
  - [roym-shop](../../changes/roym-shop/change.md): its SH-5 defines what
    an attribute is (a code, a type, choice codes, labels per language) and
    says the shop "adds attribute definitions" to the taxonomy file. B2
    builds the file format and a first small set (Q8). SH-5 stays a
    criterion of the shop change doc, met by M08D. B2 writes nothing there.
- [slice-b1b-implementation-plan.md](slice-b1b-implementation-plan.md) (the
  Hub rules: D-B1b-1 to D-B1b-15) and the three review files. Section 7.8
  lists how B2 avoids their kinds of findings.

### Terms used in this plan

- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **Code:** a short token of `[a-z0-9-]`, 1 to N bytes, that a record
  stores instead of a word: `plumbing`, `kg`, `upi`. A code is never
  translated. Only its label is.
- **Base taxonomy:** the code lists that the Roym project keeps in the
  repository (owner answer 5): categories, units, payment methods and
  attributes. File: `crates/roym_core/taxonomy/base.json` (new).
- **Own code:** a well-formed code that is not in the base taxonomy. A
  group may use one at once; the Hub shows it as the raw code (ML-7).
- **Label:** the word for a code in one language, for example
  `plumbing` → "Plumbing" in English and "प्लंबिंग" in Hindi.
- **Category group:** a category with no parent. Its children are the
  categories whose `parent` is its code.
- **String catalog:** as in B1a and B1b: the Fluent files of Hub text,
  `src/i18n/en.ftl` and `src/i18n/hi.ftl`.
- **Taxonomy label file:** a new Fluent file per language with only
  taxonomy labels, `src/i18n/taxonomy/<tag>.ftl` (D-B2-4).

## 1. What B2 builds, in one paragraph

B2 adds `crates/roym_core/taxonomy/base.json` with about 95 category codes
in about 14 groups, 17 unit codes, 6 payment-method codes and 6 attribute
definitions. A new Rust module `roym_core::taxonomy` reads and checks the
file, and a test fails when a code is badly formed, a parent is missing or
an attribute names an unknown category. The labels are Fluent messages in
two new files, `src/i18n/taxonomy/en.ftl` and `hi.ftl`, which the Hub loads
with its string catalogs, so English is the fallback and Hindi loads only
when it is chosen. The Hub imports the JSON file when it is built. A new
category picker replaces the four free-text category boxes (listing editor,
request form, directory search, SynOrg settings). A unit list and
payment-method check boxes replace the two free-text boxes in the listing
editor. Every place that shows a category or a payment method shows its
label. The service now refuses a unit or a payment method that is not a
well-formed code. A listing's categories stay any well-formed code, as
the change doc says.

## How this is shown to an end user

- Alice opens Listings to offer gardening work. In the category box she
  types a few letters, in Hindi or in English. The Hub shows the matching
  categories, and she picks "बागवानी" (gardening).
- For a product she picks the unit ("किलो") from a list, and she ticks the
  payment methods she takes ("नकद", "यूपीआई"). She no longer types them.
- Bob uses the Hub in English. He searches the directory for "Gardening"
  and finds Alice's listing. He sees its category as "Gardening"; Alice
  sees the same listing's category as "बागवानी".
- A category that a group made itself, and that Roym does not know yet,
  shows as its code.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core taxonomy` checks
  `base.json`. Break a parent code in the file and run it again: the test
  names the code.
- `mise run test:roym-ui` runs the picker tests, the label tests and the
  Hindi sweep (which now draws the pickers in Hindi).
- `mise run demo` runs story `demo/40-roym-languages.spec.ts`, which now
  also picks a category by its Hindi label.

**Demo:** story 40 gains one step: Alice, with her Hub in Hindi, opens
Listings, types part of a Hindi category label and picks it; the chosen
category shows its Hindi label (Q10).

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. Where do the labels live? (blocks Step 4)**

task.md Y2 says the data file in `roym_core` holds "labels per language".
ML-28 says taxonomy labels "come from the string catalog", and the change
doc says "Labels in a new language come with that language's string
catalog". The Hub already has a Fluent mechanism with English fallback,
lazy loading of Hindi, catalog tests, the Hindi sweep and an owner review
flow ([i18n.ts:16](../../../../crates/roym_web/ui/src/i18n/i18n.ts#L16)
to `:20`, `:89` to `:100`).

- (a) **Fluent files beside the string catalogs**, one per language:
  `src/i18n/taxonomy/en.ftl` and `hi.ftl`, with ids built from the codes
  (`taxonomy-category-plumbing = Plumbing`). `base.json` holds only codes
  and structure. The Hub adds each language's taxonomy file to the same
  Fluent bundle as its string catalog. ML-2, ML-7 fallback and ML-28 come
  from the B1a mechanism with no new code path. A new language is two new
  files. *Recommended.*
- (b) **Labels inside `base.json`** (`"labels": { "en": "…", "hi": "…" }`).
  One file holds all of a code, but the Hub needs its own fallback and its
  own lazy loading, or every Hindi label goes into the main script (B1b
  kept the main script free of Devanagari).
- (c) **Labels inside `en.ftl` and `hi.ftl`.** No new files, but the
  catalog tests that need literal ids (`findMessageIds`) and "no unused
  message" would need an exception for about 250 ids.

Owner answer: (a), 2026-11-07. Labels live in Fluent files beside the
string catalogs. `base.json` holds only codes and structure. Step 9 also
updates task.md Y2 and the "B. Language-neutral structure" sentence in the
change doc so they no longer say "labels per language" in the data file
(P7).

**Q2. How does the Hub read the codes? (blocks Step 4)**

task.md: "a build step or a `web` method; the plan decides". The Hub is
built from this repository and served by the same node's `web` service
from that build
([roym.toml:30](../../../../crates/roym_core/app/roym.toml#L30)), so the
Hub and the Rust code always come from the same commit.

- (a) **Vite imports `base.json` straight from `roym_core`** when it
  builds (`import base from "../../../../roym_core/taxonomy/base.json"`).
  One copy of the file, no build step, no RPC. `tsconfig.json` already has
  `resolveJsonModule` and includes imported files that are outside `src`
  ([tsconfig.json](../../../../crates/roym_web/ui/tsconfig.json)). The
  dev server needs `server.fs.allow` for the folder (D-B2-5).
  *Recommended.*
- (b) **A local-only `web` method `taxonomy.get`.** A third-party client
  could read it, but it adds a method, a contract row, an RPC before every
  picker draws and a failure path, for data that the bundle already has.
  It can be added later without changing the file.
- (c) **A build step that copies the file into `src/`.** Two copies that
  can drift; nothing gained over (a).

Owner answer: (a), 2026-11-07. Vite imports `base.json` straight from
`roym_core`.

**Q3. Which fields get the code rule in the service? (blocks Step 3)**

task.md: "Units and payment methods become codes in the listing.
`ListingPayload::validate` still accepts any well-formed token." Today a
unit and a payment method are any string up to 32 bytes, even an empty one
([listing.rs:320](../../../../crates/roym_core/src/listing.rs#L320) to
`:334`). The same method names appear in three more places: a quote's
`terms.payment_methods`
([transaction.rs:441](../../../../crates/roym_core/src/transaction.rs#L441)
to `:447`, non-empty and at most 32 bytes), a payment acknowledgement's
`method` ([payment.rs:137](../../../../crates/roym_core/src/payment.rs#L137)),
which must equal one of the agreed methods
([payment.rs:159](../../../../crates/roym_core/src/payment.rs#L159)), and a
SynOrg's own `categories`
([directory.rs:131](../../../../crates/roym_core/src/directory.rs#L131) to
`:135`, non-empty and at most 64 bytes). A SynOrg category that is not a
code can never match a listing: credential issue folds it with
`normalize_category` ([credential_ops.rs:83](../../../../crates/roym_directory/src/app/credential_ops.rs#L83)),
so `Home Repair` becomes `homerepair`.

- (a) **The code rule (`listing::valid_token`) for all five:** listing
  `product.unit`, listing `payment.methods`, quote `terms.payment_methods`,
  payment `method`, SynOrg settings `categories`. One rule for every field
  that names a code ("Structure before text"). Every fixture in the
  repository already uses codes (section 2.2 point 6). *Recommended.*
- (b) **Listing `unit` and `methods` only**, as task.md says. A quote can
  then name `UPI` while the listing it came from names `upi`, and a payment
  that names `upi` does not match.
- (c) **No Rust change.** The Hub sends codes, but `roymctl` and other
  clients may still send any text.

Owner answer: (a), 2026-11-07. The code rule applies to all five fields:
listing `product.unit`, listing `payment.methods`, quote
`terms.payment_methods`, payment `method`, and SynOrg settings
`categories`.

**Q4. Can a person use an own code in the Hub? (blocks Step 5)**

The change doc says "The Hub category picker offers only taxonomy codes".
Owner answer 5 says "A group may use its own codes at once (they show as
raw codes)". With only taxonomy codes in the picker, an own code can be
set only with `roymctl` or the API, and a Hub user can never search for
one.

- (a) **Taxonomy codes, plus one "use your own code" choice.** When the
  text in the picker's box is a well-formed code that is not in the
  taxonomy, the picker shows one more choice: "Use the code “<code>”". The
  chosen chip shows the raw code. *Recommended:* it makes owner answer 5
  work in the Hub at no cost to the normal path.
- (b) **Only taxonomy codes**, as the change doc says.

Owner answer: (a), 2026-11-07. The picker offers taxonomy codes plus one
"use your own code" choice. See D-B2-6 for the updated spec (P8:
own-code offer only when not already chosen, `codes()` returns a set).

**Q5. Can a category group be picked? (blocks Step 5)**

The directory search joins the query's categories with AND, and matches
each one exactly
([search_ops.rs:284](../../../../crates/roym_directory/src/app/search_ops.rs#L284)
to `:286`). A listing tagged `plumbing` is not found by a search for its
group `home-repair`. The shop change (M08D, SH-11) plans "category code
(with sub-categories)".

- (a) **Only categories with a parent can be picked.** A group is a
  heading in the picker. A listing, a request and a search then use the
  same level, so an exact match finds what the person means. Search by
  group comes with M08D (backlog row, section 8). *Recommended.*
- (b) **Any code can be picked.** A search by a group then finds only
  listings that name the group itself.
- (c) **Any code, and the Hub expands a group into its children for a
  search.** Needs an OR in `directory.search`, a wire change that belongs
  to M08D.

Owner answer: (a), 2026-11-07. Only categories with a parent can be
picked. A group is a heading. See D-B2-5 and D-B2-3: `validate` checks
that a group comes before its children and that every group has at least
one child (P9).

**Q6. Which forms get the category picker? (blocks Step 7)**

The Hub takes categories as comma-separated text in four forms: the
listing editor
([listings.ts:231](../../../../crates/roym_web/ui/src/screens/listings.ts#L231)),
the request form
([messages.ts:317](../../../../crates/roym_web/ui/src/screens/messages.ts#L317)),
the directory search
([directory.ts:215](../../../../crates/roym_web/ui/src/screens/directory.ts#L215)),
and the SynOrg settings
([synorg.ts:64](../../../../crates/roym_web/ui/src/screens/synorg.ts#L64)).

- (a) **All four, and a label at every place that shows a category**
  (section 2.2 point 3). One picker component serves all four.
  *Recommended:* a Hindi user meets the same categories everywhere, and
  ML-8 needs at least the listing editor and the search.
- (b) **The listing editor and the directory search only.** The request
  form and the SynOrg settings keep free text, so a Hindi user still types
  English codes there.

Owner answer: (a), 2026-11-07. All four forms get the picker. See P1:
Step 8 also updates `roym-hub.spec.ts:984` (the request-form e2e step).
See P2: the picker gains a `clear()` method for the request form reset.

**Q7. In which languages does the picker search the labels? (blocks
Step 5)**

The change doc says the picker "searches those labels in any language the
taxonomy has". With Q1 (a), English is always loaded and the UI language
is loaded when it is chosen. Other languages are not loaded.

- (a) **The UI language, English and the code itself.** No extra loads.
  With two languages this already covers every language for a Hindi user;
  an English user cannot type a Hindi label. *Recommended:* simple, and
  the gap is small.
- (b) **Every language that has a taxonomy file.** The picker loads every
  other taxonomy file the first time it draws (one more script file per
  language).

Owner answer: (a), 2026-11-07. The picker searches the UI language,
English and the code itself.

**Q8. How much of the attribute part does B2 build? (blocks Step 2 and
Step 4)**

ML-28 asks for "taxonomy attribute names and choice values" with labels.
No Hub screen shows an attribute until the shop (M08D) gives offerings
attribute values.

- (a) **The format, a check, and 6 attributes that cover the four types of
  SH-5** (choice, number with an optional unit, yes or no, text), with
  labels in both languages, and Hub label functions with unit tests.
  *Recommended:* M08D then adds attributes to a format that is already
  checked, and ML-28 has a test.
- (b) **The format and its check only, no attributes.** ML-28 has nothing
  to test until M08D.
- (c) **Nothing about attributes in B2.** ML-28 moves to M08D.

Owner answer: (a), 2026-11-07. The format, a check, and 6 attributes that
cover the four types of SH-5, with labels in both languages and Hub label
functions with unit tests.

**Q9. Does the Hub show a listing's unit to a reader? (blocks Step 6)**

ML-9: "The Hub shows its label in the reader's language." The Hub shows
the unit only in the listing editor. No screen shows another person's
product block: a directory hit carries title, summary and categories only
([search.ts:10](../../../../crates/roym_web/ui/src/directory/search.ts#L10)
to `:32`).

- (a) **The editor list and the payment-method label in the payment card
  now; a reader's view of another person's product with M08D (shop
  browse).** Recorded as a deviation. *Recommended.*
- (b) **Add unit and price to the directory hit.** Changes the output of
  `directory.find`, a wire change for one line of text.

Owner answer: (a), 2026-11-07. The editor list and the payment-method
label in the payment card now; a reader's view of a product with M08D.

**Q10. What does story 40 show? (blocks Step 8)**

task.md: "B2: the person picks a category by its Hindi label." The demo
nodes have no listing published to a directory, so a search would find
nothing.

- (a) **In the listing editor.** Alice types part of the Hindi label of
  `gardening`, picks it, and the chosen chip shows the Hindi label. She
  does not save, so the story can run again on the same nodes.
  *Recommended.* ML-8 (same results in both languages) gets its own
  Playwright test in `roym-hub.spec.ts`, where a directory exists.
- (b) **In the directory search, with a listing set up by the story.**
  More set-up in a story that is about languages.

Owner answer: (a), 2026-11-07. Story 40 shows the picker in the listing
editor.

### 2.2 Points found while reading

1. **Categories today.** A listing has at most 8 categories of
   `[a-z0-9-]`, 1 to 64 bytes
   ([listing.rs:22](../../../../crates/roym_core/src/listing.rs#L22),
   `:24`, `:289` to `:296`); its doc comment says "the vocabulary is the
   group's, not the product's" (`:185` to `:186`). A request has the same
   rule ([transaction.rs:62](../../../../crates/roym_core/src/transaction.rs#L62),
   `:495` to `:501`). A search query takes at most 32
   ([directory.rs:24](../../../../crates/roym_core/src/directory.rs#L24),
   [search_ops.rs:242](../../../../crates/roym_directory/src/app/search_ops.rs#L242))
   and folds each with `normalize_category`
   ([directory.rs:248](../../../../crates/roym_core/src/directory.rs#L248)).
   A membership credential's scope categories must already be folded
   ([membership.rs:203](../../../../crates/roym_core/src/membership.rs#L203)
   to `:205`) and are matched against a listing's categories (`:473` to
   `:476`). The index also puts the category codes into the text field
   ([search_ops.rs:94](../../../../crates/roym_directory/src/app/search_ops.rs#L94)
   to `:98`). B2 changes none of this.
2. **The Hub reads categories as text** in four forms (Q6) and splits them
   at commas. The listing editor lower-cases them
   ([editor.ts:262](../../../../crates/roym_web/ui/src/listings/editor.ts#L262));
   the other three send them as typed. Units and methods are text too
   (`:183`, `:206`; [listings.ts:268](../../../../crates/roym_web/ui/src/screens/listings.ts#L268),
   `:282`, where the unit starts as `item`).
3. **Where the Hub shows a code that B2 labels.**
   - Category: the directory hit
     ([directory.ts:311](../../../../crates/roym_web/ui/src/screens/directory.ts#L311)
     to `:313`), the request card
     ([request.ts:53](../../../../crates/roym_web/ui/src/cards/templates/request.ts#L53)
     to `:56`), the credential row
     ([synorg_members.ts:256](../../../../crates/roym_web/ui/src/screens/synorg_members.ts#L256)),
     and the credential issue check boxes (`:121` to `:131`, which show
     the SynOrg's codes). B1b left these raw on purpose: "the category
     labels are the taxonomy work" (D-B1b-6).
   - Payment method: the payment acknowledgement card
     ([payment_acknowledgement.ts:65](../../../../crates/roym_web/ui/src/cards/templates/payment_acknowledgement.ts#L65)
     to `:69`). The quote card has a `payment_methods` field in its type
     but does not draw it
     ([quote.ts:28](../../../../crates/roym_web/ui/src/cards/templates/quote.ts#L28)).
   - Unit: only the listing editor.
   - Nothing in the Hub shows an attribute.
4. **Who sends codes besides the Hub.** `roymctl` takes `--method` for a
   quote ([transaction.rs:35](../../../../apps/roymctl/src/commands/roym/transaction.rs#L35))
   and for a payment acknowledgement
   ([booking.rs:94](../../../../apps/roymctl/src/commands/roym/booking.rs#L94)),
   and `--category` for SynOrg settings and for `find`
   ([directory.rs:56](../../../../apps/roymctl/src/commands/roym/directory.rs#L56),
   `:95`). It has no listing command. Its flags do not change; with Q3 (a)
   the service refuses a value that is not a code, with `-32602`.
5. **The typed inputs do not change shape.** `SetListingInput`
   ([listing_ops.rs:81](../../../../crates/roym_catalog/src/app/listing_ops.rs#L81)),
   `RequestSetInput` and `QuoteSetInput`
   ([request_ops.rs:28](../../../../crates/roym_transaction/src/app/request_ops.rs#L28),
   [quote_ops.rs:79](../../../../crates/roym_transaction/src/app/quote_ops.rs#L79))
   keep `Vec<String>` and `String` fields. Only the check inside
   `validate()` changes. So the `method_contract` rows
   (`crates/roym_web/tests/method_contract/`) do not change.
6. **Fixtures already use codes.** Every unit and payment method in Rust
   tests and e2e is a code: `hour`, `cash`, `card`, `sepa`
   ([listing/tests.rs:95](../../../../crates/roym_core/src/listing/tests.rs#L95),
   `:116`; [transaction/tests.rs:15](../../../../crates/roym_core/src/transaction/tests.rs#L15);
   [quote_ops/tests.rs:17](../../../../crates/roym_transaction/src/app/quote_ops/tests.rs#L17);
   [roym_flow.rs:169](../../../../crates/substrate/tests/common/roym_flow.rs#L169);
   [roym_restore_e2e.rs:227](../../../../crates/substrate/tests/roym_restore_e2e.rs#L227)).
   Every SynOrg category in a test is a code (`cycling`, `gardening`,
   `plumbing`). So Q3 (a) breaks no fixture. The Hub editor test uses the
   unit `item` ([editor.test.ts:48](../../../../crates/roym_web/ui/src/listings/editor.test.ts#L48)),
   which stays a valid code.
7. **e2e steps type a category.** `roym-hub.spec.ts` fills a category box
   with `cycling` at lines 476, 506, 565, 607, 645, 666, 731, 792 and 823,
   and sends `categories: ['cycling']` to `credential.issue` at 490 and
   741. `roym-hub.spec.ts:984` (test 26, "messages tab: sending a request
   posts a card …") does
   `page.locator('.request-cats-input').fill('gutters, cleaning')` —
   `gutters` is not in the taxonomy and `cleaning` is a group, so both
   must be replaced with picker calls. `roym-trust.spec.ts` does the same
   at lines 42, 82 and 113. The picker replaces these boxes, so the steps
   change (Step 8). No spec reads the text of a shown category.
8. **The Hindi sweep marks categories as data.** Its fixtures use
   `Zq-cycling` as a category
   ([hindi-screens.test.ts:80](../../../../crates/roym_web/ui/src/i18n/hindi-screens.test.ts#L80),
   `:316`, `:370`, `:385`), so the raw code is removed before the check.
   After B2 a category is a code with a label, so the fixtures use real
   taxonomy codes, and a raw taxonomy code fails the sweep (D-B2-11). The
   allowed Latin word `item` ("the unit that the listing editor starts
   with", [latin-words.ts:24](../../../../crates/roym_web/ui/src/testing/latin-words.ts#L24))
   goes, because the unit becomes a list with labels.
9. **A planning reference in a code comment.**
   [listing.rs:27](../../../../crates/roym_core/src/listing.rs#L27) to
   `:29` says "N-2: fields that were unbounded… From this slice…". It
   names a review finding and a slice, which AGENTS.md forbids. B2 edits
   these lines (the unit and method rules), so it rewrites the comment.
10. **The N-2 backlog row.** All N-2 bounds exist now:
    `MAX_PAYEE_LEN`, `MAX_PAYMENT_METHODS`, `MAX_PAYMENT_METHOD_LEN`,
    `MAX_UNIT_LEN`, `MAX_SKU_LEN`, `MAX_SERVICE_LIST_ITEMS`,
    `MAX_SERVICE_LIST_ITEM_LEN`
    ([listing.rs:30](../../../../crates/roym_core/src/listing.rs#L30) to
    `:36`) and the named-area label bound
    ([area.rs:96](../../../../crates/roym_core/src/area.rs#L96)). The row
    ([deferred-backlog.md:272](../../deferred-backlog.md)) still lists
    N-2 and links `crates/roym_core/src/listing/mod.rs`, a path that no
    longer exists. B2 removes N-2 from the row (section 8).
11. **`transaction.rs` is 776 lines**, close to the 800-line limit for a
    production file. Q3 (a) changes one condition there and renames one
    error variant, so the file does not grow. Do not add lines to it; a
    helper goes in `listing.rs` or a new file.
12. **The Hub's language loader** reads `./*.ftl` except `en.ftl`
    ([i18n.ts:16](../../../../crates/roym_web/ui/src/i18n/i18n.ts#L16) to
    `:20`), and English is imported raw (`:2`). A `Loaders` value is
    `() => Promise<string>`, and the tests pass their own loaders
    (`activateWith`, `initLanguageWith`). D-B2-4 keeps that type: a
    language's loader returns the string catalog and the taxonomy file
    joined, so no test signature changes.
13. **`hubMessage` reads only `src/i18n/<tag>.ftl`**
    ([hub-helpers.ts:99](../../../../crates/substrate/tests/e2e/hub-helpers.ts#L99)).
    The e2e steps that find a category by its label need it to read the
    taxonomy file too (Step 8).
14. **The developer guide shows category codes** in the SynOrg and `find`
    examples (`--category plumbing --category gardening`,
    [developer-guide.md:1702](../../../developer-guide.md), `:1716`). Both
    stay valid codes, and both become taxonomy codes (section 5, D-B2-2).

## 3. What B2 meets

| Criterion | What B2 does | Test |
| --- | --- | --- |
| ML-7 | Listing and request categories come from the picker as taxonomy codes (or an own code, Q4). Every place that shows a category shows its label; a code with no label in the UI language shows its English label; a code not in the taxonomy shows as the code. | `taxonomy.test.ts` ("a category shows its label in each language", "falls back to the English label", "shows an unknown code as it came", the `constructor` cases); `category_picker.test.ts`; `hindi-screens.test.ts` (each picker and each shown category in Hindi); the label cases in `directory.test.ts`, `templates.test.ts` and `synorg_members.test.ts` |
| ML-8 | The search sends the chosen codes, which do not depend on the UI language. | `directory.test.ts` ("the same category gives the same query in English and Hindi"); `roym-hub.spec.ts` ("a category search finds the same listings in English and in Hindi") |
| ML-9 | A product's unit is a code from the unit list; the editor shows unit labels; the service refuses a unit that is not a code. | `editor.test.ts`, `listings.test.ts` (unit options in each language); `listing/tests.rs` (the unit code rule); the reader's view is Q9 |
| ML-28 (taxonomy part) | Attribute names and choice values are codes with labels from the taxonomy label files, with English as fallback. | `taxonomy.test.ts` ("an attribute and its choices show their labels", fallback and unknown cases); `taxonomy/tests.rs` (the attribute checks) |

B2 also meets two parts of ML-2 for taxonomy labels: a new language needs
only a new taxonomy label file, and a missing label falls back to
English (`taxonomy.test.ts`, with a test loader). B2 names them in the
ML-2 row.

**Where decisions and deviations are written.** All four criteria are in
the roym-multilingual change doc, so every B2 decision and deviation goes
there and only there, at close-out (Step 9): decisions into "B.
Language-neutral structure" as a new "Decided <date>, the base taxonomy"
list; deviations under `## Deviations`. The deviations already known now
(they depend on the owner answers): labels in Fluent files and not in the
data file (Q1); the code rule on five fields, not two (Q3); the own-code
choice (Q4); groups that cannot be picked (Q5); the picker's search
languages (Q7); the reader's view of a unit (Q9).

## 4. Behaviour-change audit

### Wire changes

No method, field, record version, WIT, config or CLI flag changes. Five
checks get stricter (Q3 (a)). Each refusal is `-32602` with the error text
of the variant:

| # | Field | Before | After |
| --- | --- | --- | --- |
| W1 | listing `product.unit` | any string, at most 32 bytes, empty allowed | a code: 1 to 32 bytes of `[a-z0-9-]` (`ListingError::UnitShape`) |
| W2 | listing `payment.methods[]` | each any string, at most 32 bytes, empty allowed | each a code, 1 to 32 bytes (`ListingError::PaymentMethodShape`) |
| W3 | quote `terms.payment_methods[]` (and so both halves of the agreement receipt, which copy the terms) | each non-empty after trim, at most 32 bytes | each a code, 1 to 32 bytes (`TransactionError::PaymentMethodShape`) |
| W4 | payment acknowledgement `method` | non-empty, at most 32 bytes | a code, 1 to 32 bytes (`PaymentError::MethodShape`) |
| W5 | SynOrg settings `categories[]` (`directory.set-settings`) | non-empty after trim, at most 64 bytes | each a code, 1 to 64 bytes (`SynOrgSettingsError::CategoryShape`, which keeps its name and gets the code in its text) |

W1 and W2 apply wherever a listing is checked: `listing.set`,
`listing.verify`, a directory publication, the client half of a search
(`verify_envelope`, [listing.rs:519](../../../../crates/roym_core/src/listing.rs#L519)),
and `catalog.import`. So a listing that another node signed with
`unit: "Kilogram"` is now refused when a searcher verifies it. The
product is not released; there is no migration (change doc rule 4). W3
applies in `quote.set`, `quote.verify` and agreement acceptance, because
they all call `AgreedTerms::validate`. Nothing in the repository sends a
value that W1 to W5 refuse (section 2.2 point 6).

What the Hub sends changes too, with the same fields:

| # | Field | Before | After |
| --- | --- | --- | --- |
| H1 | `categories` of `listing.set`, `request.set`, `directory.find`, `directory.set-settings` | the typed words split at commas | the chosen codes, in the order chosen |
| H2 | `product.unit` of `listing.set` | typed, starts as `item` | chosen from the unit list, starts as `piece` |
| H3 | `payment.methods` of `listing.set` | typed words split at commas | the ticked codes, in list order |

### Behaviour changes (Hub only)

| # | Before | After |
| --- | --- | --- |
| 1 | Four free-text category boxes | One category picker in each form (Q6) |
| 2 | Unit and payment methods are text boxes | A unit list and payment-method check boxes |
| 3 | A shown category is the raw code (`bicycle-repair`) | Its label in the UI language ("Bicycle repair"); an own code stays raw. **English text changes here** |
| 4 | A shown payment method is the raw code (`upi`) | Its label ("UPI"); an unknown method stays raw. **English text changes here** |
| 5 | The SynOrg credential check boxes show raw codes | Labels; the box `value` stays the code |
| 6 | The placeholders of the four category boxes, the unit box and the method box | Gone with the boxes; the picker has its own messages (D-B2-8) |

### Stays the same

- Every record shape, field name and `version`. The categories of a
  listing and a request are still any well-formed code (the change doc
  rejects a taxonomy check in `validate`).
- The directory index, `normalize_category`, `category_tokens` and the AND
  meaning of a category list in a search.
- Every JSON-RPC method, every typed input and every `method_contract`
  row.
- Every English Hub string except the shown codes of rows 3 and 4 and the
  removed placeholders of row 6.
- `roymctl` flags and output.

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B2-1. The data file.** `crates/roym_core/taxonomy/base.json`, JSON
  (both `serde_json` and Vite read it with no new dependency). Shape:

  ```json
  {
    "categories": [
      { "code": "home-repair" },
      { "code": "plumbing", "parent": "home-repair" }
    ],
    "units": ["piece", "kg"],
    "payment_methods": ["cash", "upi"],
    "attributes": [
      { "code": "diet", "categories": ["food"],
        "value": { "type": "choice", "choices": ["veg", "non-veg"] } },
      { "code": "seats", "categories": ["taxi"],
        "value": { "type": "number" } },
      { "code": "home-visit", "categories": ["home-repair"],
        "value": { "type": "yes-no" } },
      { "code": "brand", "categories": ["electronics"],
        "value": { "type": "text" } }
    ]
  }
  ```

  - Order in the file is the order the Hub shows. A group comes before its
    children.
  - `value.type` is `choice` (with `choices`, at least one), `number`
    (with an optional `unit`, a code from `units`), `yes-no` or `text`
    (SH-5's four types). An attribute on a group applies to its children
    too (SH-5); B2 only stores it, and M08D reads it that way.
  - Two levels only (Q5 (a)): a category with a `parent` is not itself a
    parent. M08D may allow more levels.
  - No version field and no labels in the file (Q1 (a)).
- **D-B2-2. The first codes** (owner answer 5: 50 to 150, local services
  and goods). The implementer writes the file from this list; the owner
  checks it with the labels (D-B2-12). Groups first, then their children:
  - `home-repair`: `plumbing`, `electrical`, `carpentry`, `painting`,
    `masonry`, `waterproofing`, `appliance-repair`, `ac-repair`,
    `water-purifier-service`, `pest-control`, `locksmith`
  - `cleaning`: `home-cleaning`, `deep-cleaning`, `water-tank-cleaning`,
    `sofa-carpet-cleaning`, `laundry`, `dry-cleaning`
  - `household-help`: `cook`, `domestic-help`, `babysitting`,
    `elder-care`, `driver`, `security-guard`
  - `vehicles`: `bicycle-repair`, `two-wheeler-repair`, `car-repair`,
    `car-wash`, `taxi`, `auto-rickshaw`, `vehicle-rental`
  - `moving-delivery`: `packers-movers`, `courier`, `local-delivery`,
    `goods-transport`
  - `personal-care`: `salon`, `beauty-parlour`, `barber`, `massage`,
    `mehndi`, `tailoring`
  - `health`: `doctor`, `nursing`, `physiotherapy`, `pharmacy`,
    `diagnostic-lab`, `yoga`
  - `education`: `tuition`, `music-lessons`, `dance-lessons`,
    `language-lessons`, `computer-lessons`, `driving-lessons`
  - `events`: `catering`, `photography`, `decoration`, `tent-house`,
    `priest`, `event-music`
  - `professional`: `accounting`, `tax-filing`, `legal-help`,
    `documents-help`, `computer-repair`, `mobile-repair`, `printing`,
    `web-design`
  - `food`: `tiffin`, `home-food`, `bakery`, `sweets`,
    `fruits-vegetables`, `dairy`, `groceries`, `meat-fish`
  - `goods`: `clothing`, `footwear`, `electronics`, `mobile-phones`,
    `furniture`, `books-stationery`, `hardware`, `handicrafts`,
    `second-hand`
  - `agriculture`: `gardening`, `farm-produce`, `seeds-plants`,
    `farm-labour`, `tractor-hire`
  - `animals`: `pet-care`, `veterinary`, `livestock`

  That is 14 groups and 91 children (105 codes). Units: `piece`, `pair`,
  `set`, `dozen`, `pack`, `box`, `kg`, `g`, `quintal`, `l`, `ml`, `m`,
  `sq-ft`, `hour`, `day`, `visit`, `km`. Payment methods: `cash`, `upi`,
  `bank-transfer`, `card`, `cheque`, `wallet`. Attributes (Q8 (a)):
  `diet` (choice `veg`, `egg`, `non-veg`, `vegan`, `jain`; on `food`),
  `fabric` (choice `cotton`, `silk`, `wool`, `linen`, `synthetic`; on
  `clothing`), `size` (choice `xs`, `s`, `m`, `l`, `xl`, `xxl`; on
  `clothing`, `footwear`), `seats` (number; on `taxi`, `vehicle-rental`),
  `home-visit` (yes-no; on `home-repair`, `health`, `personal-care`,
  `education`), `brand` (text; on `electronics`, `mobile-phones`).
  The codes that tests and docs use are in the list: `bicycle-repair`,
  `gardening`, `plumbing` (Step 8; section 2.2 point 14).
- **D-B2-3. The Rust module `roym_core::taxonomy`** (`src/taxonomy.rs` and
  `src/taxonomy/tests.rs`):

  ```rust
  /// The base taxonomy that the Roym project keeps. A record may also
  /// carry a code that is not here; `ListingPayload::validate` checks only
  /// the shape of a code.
  pub const BASE_JSON: &str = include_str!("../taxonomy/base.json");

  #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct Taxonomy {
      pub categories: Vec<CategoryDef>,
      pub units: Vec<String>,
      pub payment_methods: Vec<String>,
      pub attributes: Vec<AttributeDef>,
  }

  #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct CategoryDef {
      pub code: String,
      #[serde(default)]
      pub parent: Option<String>,
  }

  #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct AttributeDef {
      pub code: String,
      pub categories: Vec<String>,
      pub value: AttributeValue,
  }

  #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
  #[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
  pub enum AttributeValue {
      Choice { choices: Vec<String> },
      Number { #[serde(default)] unit: Option<String> },
      YesNo,
      Text,
  }

  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum TaxonomyError {
      #[error("the taxonomy file does not parse: {0}")]
      Parse(String),
      #[error("{kind} code '{code}' is not a code of [a-z0-9-] that fits its field")]
      CodeShape { kind: &'static str, code: String },
      #[error("{kind} code '{code}' appears twice")]
      Duplicate { kind: &'static str, code: String },
      #[error("category '{code}' names a parent '{parent}' that is not a group")]
      BadParent { code: String, parent: String },
      #[error("category '{code}' appears before its group '{parent}'")]
      ParentAfterChild { code: String, parent: String },
      #[error("group '{0}' has no children")]
      GroupWithoutChildren(String),
      #[error("attribute '{attribute}' names an unknown category '{category}'")]
      UnknownCategory { attribute: String, category: String },
      #[error("attribute '{attribute}' names an unknown unit '{unit}'")]
      UnknownUnit { attribute: String, unit: String },
      #[error("attribute '{0}' has no categories, or a choice attribute has no choices")]
      Empty(String),
  }

  impl Taxonomy {
      pub fn parse(json: &str) -> Result<Self, TaxonomyError>; // parse, then validate
      pub fn validate(&self) -> Result<(), TaxonomyError>;
  }
  ```

  `validate` checks, using `listing::valid_token` and the record limits:
  a category code fits `MAX_CATEGORY_LEN` (64), a unit fits
  `MAX_UNIT_LEN` (32), a payment method fits `MAX_PAYMENT_METHOD_LEN`
  (32), an attribute code and a choice code fit 64; no code appears twice
  in its list (choices: within their attribute); a `parent` is a category
  with no parent of its own; a child must come after its group in the file
  (otherwise `ParentAfterChild`); every group has at least one child
  (otherwise `GroupWithoutChildren`); an attribute names at least one
  category, and each exists; a choice attribute has at least one choice; a
  number unit is in `units`. No service calls `parse` in B2, so no guest
  links the file. Keep `validate` under the 100-line limit by one small
  function per list.
- **D-B2-4. The labels** (Q1 (a)). `src/i18n/taxonomy/en.ftl` and
  `src/i18n/taxonomy/hi.ftl`. Ids:

  | Kind | Id | Example |
  | --- | --- | --- |
  | category | `taxonomy-category-<code>` | `taxonomy-category-plumbing = Plumbing` |
  | unit | `taxonomy-unit-<code>` | `taxonomy-unit-kg = kg` |
  | payment method | `taxonomy-method-<code>` | `taxonomy-method-upi = UPI` |
  | attribute | `taxonomy-attribute-<code>` | `taxonomy-attribute-diet = Diet` |
  | choice | `taxonomy-choice-<attribute>_<choice>` | `taxonomy-choice-diet_veg = Vegetarian` |

  The `_` in a choice id cannot appear in a code, so `a-b` + `c` and `a` +
  `b-c` never give the same id. No string-catalog id starts with
  `taxonomy-` (a test checks it).

  Loading (`i18n.ts`): `EN_SOURCE` becomes the string catalog and the
  English taxonomy file joined by a new line. For every other language, the
  loader in `LOADERS` returns its string catalog joined with its taxonomy
  file, or the string catalog alone when it has no taxonomy file. The
  taxonomy files are found with a second `import.meta.glob` over
  `./taxonomy/*.ftl` (minus `en`), so each is its own script file that
  loads with its language. `Loaders`, `activateWith`, `initLanguageWith`
  and every test that passes loaders stay as they are (section 2.2
  point 12). Fluent returns an error list from `addResource` for each id
  that appears twice (`allowOverrides` is false). The catalog test calls
  `addResource` on the string catalog and then on the taxonomy file for
  each real language, and expects an empty error list from each call,
  proving no id appears in both files.
- **D-B2-5. How the Hub reads the codes** (Q2 (a)).
  `src/taxonomy/taxonomy.ts` imports
  `../../../../roym_core/taxonomy/base.json` and exports:

  ```ts
  export interface CategoryGroup { code: string; children: string[] }
  export const CATEGORY_GROUPS: readonly CategoryGroup[];   // file order
  export const UNITS: readonly string[];
  export const PAYMENT_METHODS: readonly string[];
  export function isTaxonomyCategory(code: string): boolean;
  export function categoryGroupOf(code: string): string | undefined;
  export function categoryLabel(code: string): string;
  export function unitLabel(code: string): string;
  export function methodLabel(code: string): string;
  export function attributeLabel(code: string): string;
  export function choiceLabel(attribute: string, choice: string): string;
  /// Every label id that base.json implies, for the catalog test.
  export function taxonomyMessageIds(): string[];
  /// Children whose label in the UI language or in English, or whose
  /// code, holds `text`; also children whose group's label (UI language
  /// or English) or group code holds `text`. With Q4 (a) no group is
  /// ever returned.
  export function matchCategories(text: string): string[];
  ```

  Each label function is `codeWord(table, code)` over a `Map` of code to
  id, made once when the module loads (ids only, no text, so no `t()` call
  at load, B1a rule). So `codeWord` stays the one reader of code tables
  (B1b R7), an unknown code shows as it came (ML-7), and a code from
  another node such as `constructor` cannot hit an inherited property.
  These `Map`s hold computed ids, not `messageId("…")` literals. That is
  the one exception to D-B1b's rule for id tables, and the catalog test
  covers it instead: the ids of `taxonomy/en.ftl` are exactly
  `taxonomyMessageIds()`. `matchCategories` compares with
  `toLocaleLowerCase(formatLocale())` and `includes`, and keeps file
  order. A child also matches when its group's label (UI language or
  English) or its group code holds the text; the group itself is not
  returned. `vite.config.ts` gets `server: { fs: { allow: [".", "../../roym_core/taxonomy"] } }`
  so `npm run dev` and vitest may read the file; `vite build` needs
  nothing.
- **D-B2-6. The category picker**, `src/taxonomy/category_picker.ts`:

  ```ts
  export function categoryPicker(opts: {
    className: string;          // keeps today's class, e.g. "listing-categories-input"
    max: number;                // 8 for a listing or a request, 32 for a search or a SynOrg
    selected?: readonly string[];
  }): { root: HTMLElement; codes(): string[]; clear(): void };
  ```

  DOM: `div.category-picker.<className>` holds `ul.picker-chosen` (one
  `li.picker-chip[data-code]` per chosen code: its label and a remove
  button whose `aria-label` is `category-picker-remove` with `$label`),
  `input.picker-filter` (placeholder `category-picker-filter-placeholder`),
  and `div.picker-options`. When the filter is empty, the options show
  every group as a heading (`div.picker-group`) with its children as
  `button.picker-option[data-code]`. When the filter has text, only the
  matching children show, each with its group label before it. A chosen
  code does not show as an option. The no-match message
  `category-picker-no-match` shows when the filter has text and there is
  no option at all (no matching child and no own-code option). At `max`
  codes every option is disabled and `category-picker-full` (with `$max`
  as a number) shows. With Q4 (a): when the trimmed, lower-cased filter
  text is a valid code (`/^[a-z0-9-]{1,64}$/`), is not a taxonomy code,
  and is not already chosen, one more option
  `button.picker-option.picker-own-code[data-code]` shows
  `category-picker-own-code` with `$code`. A chip of an own code shows the
  code. `codes()` returns a set (no duplicates) in the order chosen.
  `clear()` empties the chips and the filter and closes the options.
  `selected` codes that are not in the taxonomy show as chips with
  the raw code (an older SynOrg setting). No `localStorage`, no network.
  The option buttons can be reached with Tab. Enter in the filter picks
  the first option when there is exactly one. The no-match and full
  messages have `aria-live="polite"` on their container.
- **D-B2-7. Units and payment methods in the editor.** The unit box
  becomes `select(UNITS.map(u => [u, unitLabel(u)]), "piece")` (the
  existing `select` in `dom.ts`, [dom.ts:77](../../../../crates/roym_web/ui/src/dom.ts#L77)).
  The methods box becomes one `checkbox(methodLabel(m))` per code in
  `PAYMENT_METHODS`, each with `box.value = m` and the class
  `payment-method-<code>`, in a `div.payment-methods`. `ProductFields.unit`
  stays a `string`; `PaymentFields.methods` becomes `string[]`;
  `ListingForm.categories` becomes `string[]`, and `buildSetListingParams`
  sends it as it is (no split, no lower-casing).
- **D-B2-8. New Hub messages** (in `en.ftl` and `hi.ftl`, prefix
  `category-picker-`): `category-picker-filter-placeholder` (English
  `Type to find a category`), `category-picker-remove` (`Remove { $label }`),
  `category-picker-own-code` (`Use the code “{ $code }”`),
  `category-picker-no-match` (`No category matches.`),
  `category-picker-full` (`You can choose up to { $max } categories.`).
  The placeholders `listing-placeholder-categories`,
  `listing-placeholder-unit`, `listing-placeholder-methods`,
  `request-categories-placeholder`,
  `directory-search-categories-placeholder` and
  `synorg-categories-placeholder` are removed with their boxes (the
  "no unused message" test needs it). `directory-search-help` says
  "Matching is on whole words in the title, summary and categories"; it
  stays true (the index still holds the codes) and does not change.
- **D-B2-9. Where a label shows** (Q6 (a)). The directory hit, the request
  card, the credential row and the credential check boxes use
  `categoryLabel`; the payment acknowledgement card uses `methodLabel`.
  A list of labels is joined with `", "`, as the codes are today, inside
  the same messages (`directory-hit-categories`, `request-categories`,
  `synorg-credential-covers`). Nothing else changes in those messages.
- **D-B2-10. The service rule** (Q3 (a)). One helper decides what a code
  is: `listing::valid_token(s, max)`
  ([listing.rs:265](../../../../crates/roym_core/src/listing.rs#L265)),
  already used for categories and slugs. The five checks of section 4 call
  it. New or renamed error variants:
  `ListingError::UnitShape(String)` (replaces `UnitTooLong`),
  `ListingError::PaymentMethodShape(String)` (replaces
  `PaymentMethodTooLong`), `TransactionError::PaymentMethodShape(String)`
  (replaces `PaymentMethodTooLong`), `PaymentError::MethodShape(String)`
  (replaces `InvalidPaymentMethod` at [payment.rs:38](../../../../crates/roym_core/src/payment.rs#L38)),
  and `SynOrgSettingsError::CategoryShape` gains the code
  (`CategoryShape(String)`). Each text says
  `… '{0}' is not 1..={MAX} chars of [a-z0-9-]`, like `CategoryShape` in
  `listing.rs:223`. Search every crate for the old variant names
  (`git grep -n 'UnitTooLong\|PaymentMethodTooLong\|InvalidPaymentMethod\|CategoryShape'`) and update each
  match and each test that reads the error text.
- **D-B2-11. The Hindi sweep and the tests use real codes.** The fixtures'
  `Zq-cycling` categories become the taxonomy code `bicycle-repair`. A raw
  taxonomy code left on a Hindi screen then fails the sweep. The data
  marker `Zq` cannot mark an own code: the match is case-sensitive
  (`leftoverLatin`, [latin-words.ts:47](../../../../crates/roym_web/ui/src/testing/latin-words.ts#L47)),
  and a code is lower case. So the sweep's own-code fixture is `x9`: the
  sweep reports only runs of two or more Latin letters
  (`/[A-Za-z]{2,}/`), so `x9` shows raw and passes with no new allowed
  word. The `item` entry leaves `latin-words.ts`.
- **D-B2-12. The taxonomy labels and their review.** An agent writes the
  Hindi labels (owner answer 3) with the rules of section 7.7. Before the
  merge the owner reviews the code list and both label files with
  `taxonomy-review.md` in this folder: one table row per code (kind, code,
  group, English, Hindi). The fix session applies the owner's changes, sets
  the first comment of `taxonomy/hi.ftl` to `# owner review: done <date>`,
  and deletes `taxonomy-review.md`. The PR does not merge before that
  (B1b D-B1b-13 did the same for `hi.ftl`).
- **D-B2-13. The comment and the doc comments in `listing.rs`.** Lines
  27 to 29 become a comment with no planning reference, for example: "A
  stranger's listing is stored on a directory owner's disk, so every field
  has a bound." The doc comment of `categories` (`:185` to `:186`) becomes:
  "Category codes, lowercase `[a-z0-9-]`. Usually from the base taxonomy
  (`crate::taxonomy`); a group may use its own code, so `validate` checks
  only the shape." `ProductDetail.unit` and `PaymentTerms.methods` get
  one-line doc comments that say they are codes.

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches. Hub: `npm run build` and
`mise run test:roym-ui` in `crates/roym_web/ui`. Paths below are from the
repository root.

### Step 1 — Commit this plan

Done in the planning session: this file and the B2 row of
[status.md](status.md).

### Step 2 — The data file and the Rust module

1. `crates/roym_core/taxonomy/base.json` (D-B2-1, D-B2-2).
2. `crates/roym_core/src/taxonomy.rs` and `src/taxonomy/tests.rs`
   (D-B2-3); `pub mod taxonomy;` in `lib.rs`.
3. Tests of section 7.2 ("taxonomy").
4. `cargo nextest run -p syneroym-roym-core taxonomy`.

### Step 3 — The code rule in the service

1. `listing.rs`: W1, W2 (D-B2-10); the comment and doc comments
   (D-B2-13).
2. `transaction.rs`: W3, with no net new line (section 2.2 point 11).
3. `payment.rs`: W4. `directory.rs`: W5.
4. Update every user of the renamed variants (D-B2-10).
5. Tests of section 7.2 ("the code rule").
6. `mise run build:roym`, then
   `cargo nextest run -p syneroym-roym-core -p syneroym-roym-catalog -p syneroym-roym-transaction -p syneroym-roym-directory -p syneroym-roym-web`
   (the last one runs the parity and `method_contract` tests).

### Step 4 — Labels and the Hub's taxonomy module

1. `crates/roym_web/ui/src/i18n/taxonomy/en.ftl` and `hi.ftl` (D-B2-4),
   one `##` section per kind, in file order of `base.json`. The first
   comment of `hi.ftl` is `# owner review: pending`.
2. `i18n.ts`: the loading of D-B2-4.
3. `src/taxonomy/taxonomy.ts` (D-B2-5); `vite.config.ts` `server.fs.allow`.
4. `catalog.test.ts` and `src/taxonomy/taxonomy.test.ts`: section 7.1.
5. `npm run build`, then check that the built main script holds the codes
   (`grep -c bicycle-repair dist/assets/*.js`) and still has no
   Devanagari letter (`grep -c '[ऀ-ॿ]'` on the main script gives 0). This
   shows the JSON import and the split work in the real build, not only
   in vitest.

### Step 5 — The category picker

1. `src/taxonomy/category_picker.ts` (D-B2-6) and the messages of D-B2-8 that it
   uses. Picker styles in `src/style.css`.
2. `src/taxonomy/category_picker.test.ts` (section 7.1).

### Step 6 — The listing editor

1. `screens/listings.ts`: the category picker (`className:
   "listing-categories-input"`, `max: 8`), the unit list and the
   payment-method boxes (D-B2-7). Remove the three placeholders (D-B2-8).
2. `listings/editor.ts`: the type changes of D-B2-7.
3. `editor.test.ts` and `listings.test.ts` (section 7.1);
   `latin-words.ts` loses `item` (D-B2-11).

### Step 7 — The other forms and the shown codes

1. `screens/messages.ts` (request form, `className:
   "request-cats-input"`, `max: 8`), `screens/directory.ts` (search,
   `className: "search-categories"`, `max: 32`), `screens/synorg.ts`
   (settings, `className: "synorg-categories"`, `max: 32`, `selected` =
   the current settings). Remove their placeholders (D-B2-8). In
   `messages.ts`, replace `catsInput.value = ""` in the post-send reset
   with `catsPicker.clear()` (D-B2-6, P2).
2. The labels of D-B2-9: `screens/directory.ts`, `cards/templates/request.ts`,
   `screens/synorg_members.ts`, `cards/templates/payment_acknowledgement.ts`.
3. `hindi-screens.test.ts` fixtures (D-B2-11) and the screen tests of
   section 7.1.

### Step 8 — e2e and story 40

1. `crates/substrate/tests/e2e/hub-helpers.ts`:
   - `hubMessage(tag, id)` reads `src/i18n/<tag>.ftl` and, when the id is
     not there, `src/i18n/taxonomy/<tag>.ftl`.
   - New `pickCategory(scope: Locator, code: string, tag = "en")`: fills
     `scope.locator(".picker-filter")` with
     `hubMessage(tag, "taxonomy-category-" + code)`, clicks
     `scope.locator('.picker-option[data-code="' + code + '"]')`, and
     expects the chip `.picker-chip[data-code="<code>"]`.
2. `roym-hub.spec.ts` and `roym-trust.spec.ts`: every step of section 2.2
   point 7 uses `pickCategory(page.locator(".<class>"), "bicycle-repair")`,
   including `roym-hub.spec.ts:984` (replace
   `.fill('gutters, cleaning')` with two `pickCategory` calls for
   `home-cleaning` and `deep-cleaning`), and the two `credential.issue`
   calls send `["bicycle-repair"]`. After the changes, run
   `git grep -n "cats-input|categories')" crates/substrate/tests/e2e`
   to confirm no fill is left.
3. `roym-hub.spec.ts`: a new test "a category search finds the same
   listings in English and in Hindi" (section 7.3).
4. Story 40 (Q10 (a)): one more step (section 7.3). The story file holds
   the exact step.
5. `mise run test:e2e` and `mise run demo` (redirect the output, AGENTS.md
   "Context Budget"); keep the summaries for the PR text.

### Step 9 — Review file, docs, size report and close-out

1. `taxonomy-review.md` (D-B2-12), made by a throwaway script in `$TMPDIR`
   from `base.json` and the two label files. Do not commit the script.
2. Size (section 7.6): before (on `main`) and after, `npm run build`, then
   for each `dist/assets/*.js` the raw bytes and `gzip -9 -c | wc -c`.
   Write the numbers into task.md (B2 result line).
3. Change doc: the decisions of section 5 into "B. Language-neutral
   structure" as a "Decided 2026-11-07, the base taxonomy" list; also
   rewrite the existing "B." sentence that says "its labels per language"
   to say "codes and structure in the data file, labels in the Hub's
   taxonomy label files", and add one line to that list: "M08D adds
   attribute labels to the same label files"; `## Deviations` (section 3);
   the Test column of ML-2, ML-7, ML-8, ML-9 and ML-28 with the real test
   names; `demo:` stays story 40; `status` stays `draft` (B6 sets `done`);
   `living-docs-touched` lists the files of section 9.
4. Living docs (section 9), deferred backlog (section 8), task.md B2
   result line and Y2 row (rewrite "labels per language" to "codes and
   structure in the data file, labels in the Hub's taxonomy label files"),
   status.md.
5. The import clean-up pass over every Rust file you edited.
6. `cargo dupes report` over the changed Rust files (AGENTS.md recipe).
7. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests (vitest, `TZ=UTC`)

`src/taxonomy/taxonomy.test.ts`

- A category, a unit, a method, an attribute and a choice each show their
  English label, and in Hindi (after `activate("hi")`) a different text
  with a Devanagari letter.
- Fallback (ML-7, ML-2): with `activateWith("xx", { xx: … })` and a test
  catalog that has no taxonomy labels, `categoryLabel("plumbing")` is the
  English label, never an id.
- Unknown codes: `categoryLabel("zz-own")` is `zz-own`; `constructor`,
  `toString` and `__proto__` are shown as they came, for each of the five
  label functions.
- `matchCategories`: `"plumb"` finds `plumbing`; the Hindi label text
  finds it in Hindi; upper case matches lower case; a group code is never
  returned (Q5 (a)); a group's label finds its children (P3: typing
  "Health" in English or the Hindi word for health returns all health
  children); the result keeps file order.
- `CATEGORY_GROUPS` has every category of `base.json`, each child under
  its group.

`src/i18n/catalog.test.ts` (added)

- The taxonomy label files: named by a canonical tag, each with a string
  catalog of the same tag; no line ends with a space; no message that
  Fluent drops.
- `taxonomy/en.ftl` has exactly the ids of `taxonomyMessageIds()`;
  `taxonomy/hi.ftl` has exactly the same ids.
- Every Hindi taxonomy label has a Devanagari letter (no exception list:
  write `किलो`, `यूपीआई`, and so on).
- No string-catalog id starts with `taxonomy-`.
- The English bundle and the Hindi bundle: calling `addResource` for the
  string catalog and then for the taxonomy file returns an empty error list
  each time (no id appears in both files).

`src/taxonomy/category_picker.test.ts`

- An empty filter shows every group as a heading and its children as
  options.
- Typing filters the options; clicking an option adds a chip with the
  label, and `codes()` returns the code; the remove button removes it.
- A chosen code is not offered again; at `max` the options are disabled
  and the full message shows.
- Q4 (a): `my-group-code` (not already chosen) offers the own-code
  option, and the chip shows the raw code; typing the same own code twice
  does not offer it the second time (already chosen); `codes()` returns a
  set, so the same code appears only once; `Plumbing` (a taxonomy label)
  and `bad code!` offer no own-code option.
- The no-match message shows when the filter has text and there is no
  option at all; tested in English (a code-shaped word not in the taxonomy
  still shows the own-code option, so no-match needs text that is not code-
  shaped and matches nothing); tested in Hindi.
- `clear()` empties the chips and `codes()` is empty.
- `selected` with a taxonomy code and an own code shows two chips, one
  label and one raw code.
- In Hindi, the chips and options show Hindi labels.

`src/listings/editor.test.ts`, `src/screens/listings.test.ts`

- `buildSetListingParams` sends `categories`, `product.unit` and
  `payment.methods` as given (codes, no split).
- The unit list starts at `piece` and has one option per unit, each with
  its label; the method boxes send the ticked codes in list order.

`src/screens/directory.test.ts`

- ML-8: with a fake `/rpc`, the same picked category sends the same
  `directory.find` query in English and in Hindi.
- A hit shows its categories as labels, and an own code as it came.

`src/cards/templates/templates.test.ts`, `src/screens/synorg_members.test.ts`

- The request card shows category labels; the payment acknowledgement card
  shows the method label; the credential row and the issue check boxes
  show labels, and the box `value` is the code.

`src/i18n/hindi-screens.test.ts` (D-B2-11)

- The listing editor (with the picker open and one chip), the request
  form, the directory search with a hit, the SynOrg settings with one
  category, and the credential row are drawn with taxonomy codes; no Latin
  word is left.
- A raw code fails: a test-only row that draws `bicycle-repair` without
  `categoryLabel` makes the sweep report it.

Existing tests keep their English text, except where a shown code becomes
a label (section 4, rows 3 and 4).

### 7.2 Rust

`crates/roym_core/src/taxonomy/tests.rs`

- `Taxonomy::parse(BASE_JSON)` is `Ok`, and it has 50 to 150 categories
  (owner answer 5).
- One test per `TaxonomyError` variant, each on a small JSON string: a bad
  code (`Plumbing`, `a b`, a 65-byte category, a 33-byte unit), a
  duplicate, a missing parent, a parent that has a parent, a child that
  appears before its group (`ParentAfterChild`), a group with no children
  (`GroupWithoutChildren`), an unknown attribute category, an unknown
  number unit, an attribute with no categories, a choice attribute with no
  choices, an unknown key (`deny_unknown_fields`).

The code rule (Step 3)

- `listing/tests.rs`: `unit` `""`, `"Kilogram"`, `"k g"` and a 33-byte code
  are refused with `UnitShape`; `"kg"` and `"x9"` pass. The same for a
  payment method. A listing whose unit is an own code (`"bundle-of-5"`)
  passes: the rule is the shape, not the taxonomy.
- `transaction/tests.rs`: a quote term with `"UPI"` or `" "` is refused
  with `PaymentMethodShape`.
- `payment/tests.rs`: an acknowledgement with `method: "Cash"` is refused
  with `MethodShape`.
- `directory.rs` tests: settings with `"Home Repair"` are refused with
  `CategoryShape`; `"plumbing"` passes.
- The parity and `method_contract` tests run unchanged after
  `mise run build:roym` (Step 3).

### 7.3 Playwright

- Every spec runs with the picker steps of Step 8.
- `roym-hub.spec.ts`, new test "a category search finds the same listings
  in English and in Hindi" (ML-8): `ensureSynOrgWithListing`, add the
  directory as a source, pick `bicycle-repair` by its English label,
  search, keep the listing ids of the hits; `chooseHubLanguage(page, "hi")`;
  open the directory tab by `hubMessage("hi", "tab-directory")`, pick the
  same code by its Hindi label (`pickCategory(…, "hi")`), search with the
  button `hubMessage("hi", "directory-search-button")`; the listing ids
  are the same, and the hit shows the Hindi label.
- Story 40 (Q10 (a)): after the Contacts step, Alice opens Listings (by
  `hubMessage("hi", "tab-listings")`), picks `gardening` with
  `pickCategory(…, "hi")`, and the chip has the text
  `hubMessage("hi", "taxonomy-category-gardening")`. No save.

### 7.4 Not tested

- Whether each Hindi label is good Hindi. The tests check form only; the
  owner review checks meaning (D-B2-12).
- How the picker looks on a small phone screen. It uses the Hub's existing
  field and button styles.
- Attribute inheritance from a group to its children: B2 only stores
  attributes; M08D reads them.

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `crates/roym_core/src/transaction.rs` | 776 | ±0 (section 2.2 point 11). Limit 800 |
| `crates/roym_core/src/listing.rs` | 547 | About +5 |
| `crates/roym_core/src/payment.rs`, `directory.rs` | 299, 356 | About ±0 |
| `crates/roym_core/src/taxonomy.rs` | new | About 170 |
| `crates/roym_core/src/taxonomy/tests.rs` | new | About 200 |
| `crates/roym_core/taxonomy/base.json` | new | About 260 |
| `crates/roym_web/ui/src/screens/listings.ts` | 572 | About +15 |
| `crates/roym_web/ui/src/screens/messages.ts` | 924 | About ±0 (TypeScript has no length gate) |
| `crates/roym_web/ui/src/taxonomy/taxonomy.ts` | new | About 110 |
| `crates/roym_web/ui/src/taxonomy/category_picker.ts` | new | About 150 |
| `crates/roym_web/ui/src/i18n/taxonomy/en.ftl`, `hi.ftl` | new | About 160 lines each |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | 1312 | About +35 (TypeScript, no gate) |

New Rust code adds to the workspace size, so the duplication ratchet may
move. If `cargo xtask check-duplication` says the percent is now lower
than the baseline, lower `MAX_EXACT_DUPLICATE_PERCENT` as its message says
(AGENTS.md "Mandatory Ratchet Update"). Never raise it.

### 7.6 Bundle size

The main script gains `base.json` (about 4 KB raw), the English labels
(about 5 KB raw), `taxonomy.ts` and `category_picker.ts`. Expected: under 5 KB more
after `gzip -9`. The Hindi taxonomy file is its own script file, under
8 KB after `gzip -9`. After B1b the main script was 45,160 bytes and the
Hindi catalog 13,980 bytes, both after `gzip -9` (task.md, B1b result).
Step 9 reports the real numbers. If either limit is passed, stop and tell
the owner.

### 7.7 Hindi labels

As in B1b (its section 7.7): common words that a Hindi speaker reads on a
phone. A label is a noun, short, about as long as the English one. A
loan word that people say in Hindi is written in Devanagari (`प्लंबिंग`,
`यूपीआई`, `किलो`), not in Latin letters. Use the same Hindi word for the
same thing in a category and in a string-catalog message (for example the
word for "listing"). `taxonomy-review.md` makes this easy to check.

### 7.8 How B2 avoids the kinds of findings S0, B1a and B1b had

- **Two copies of one helper** (B1b R5, R7): the label functions are
  `codeWord` over a `Map`; the editor uses the existing `select` and
  `checkbox`; the code rule is the existing `valid_token`.
- **A code from another node used as a key** (B1a P5, B1b R4): every label
  table is a `Map`, with the three inherited-name tests for each of the
  five functions.
- **A test that cannot see the real build** (B1a P8, P10): Step 4 checks
  the built script for the codes and for no Devanagari letter.
- **A raw code that the sweep cannot see** (B1b P2): the fixtures use
  real codes, and a test-only row shows that a raw code fails.
- **A step that breaks a later step** (B1b P4, P10): the removed
  placeholders and their callers go in the same step; `pickCategory` and
  the spec changes are in one step.
- **A claim not checked in the code** (S0 P1): every count and line in
  section 2.2 was read on `c44e055c`; the fixture claim of point 6 comes
  from a `git grep` over every `unit`, `methods`, `payment_methods` and
  `method` value.
- **Scratch files left behind** (S0 R5, B1a R2): the review-table script
  lives in `$TMPDIR`; `taxonomy-review.md` is deleted before the merge.
- **A planning reference in code** (AGENTS.md): B2 removes the one at
  `listing.rs:27` and adds none.

## 8. Deferred backlog

In [deferred-backlog.md](../../deferred-backlog.md):

- **§5, "Small M06C C5 review nits"** (line 272): remove N-2 from the
  title and the text, and remove `crates/roym_core/src/listing/mod.rs`
  and `area.rs` from its links. Add one row to "Recently resolved":
  "N-2: every signed listing field has a bound (`listing.rs:30` to `:36`,
  `area.rs`)".
- **§11, new row "A search by a category group does not find its
  children"**: `directory.search` matches each category exactly and joins
  them with AND, and the Hub lets a person pick only a child (Q5 (a)).
  Target M08D (SH-11, "category code (with sub-categories)"). Link
  `crates/roym_directory/src/app/search_ops.rs` (`build_search_filter`).
- **§10, new row "The category picker searches the labels of the UI
  language and English only"** (Q7 (a)). Target `TBD`. Link
  `crates/roym_web/ui/src/taxonomy/taxonomy.ts` (`matchCategories`).
- **§10, new row "The Hub shows no other person's product unit"** (Q9
  (a)). Target M08D (shop browse). Link
  `crates/roym_web/ui/src/directory/search.ts` (`MergedHit`).

The row "Accessibility (WCAG 2.1 AA) & localisation/i18n" (§10, line 508)
stays: B6 moves its localisation part. No new `TODO` or `FIXME` marker.
If an owner answer is not the recommendation, drop the matching row.

## 9. Living docs to update at close-out

- **`docs/system-requirements-spec.md:1462`** (`[APP-A11Y]`, "Hub
  localisation (built)"): one sentence: listing and request categories,
  units and payment methods are codes from a base taxonomy kept in the
  repository, and the Hub shows their labels in English or Hindi.
- **`docs/system-requirements-spec.md:685`** (the listing owner
  publishes "service categories"): say the categories are codes from the
  base taxonomy or a group's own code.
- **`docs/system-requirements-spec.md:730`** (product type with unit):
  add one sentence: the unit is a code from the base unit list in
  `crates/roym_core/taxonomy/base.json`.
- **`docs/system-architecture.md`**: in the Roym records part, update the
  `request` row near line 1206, the `quote` row (payment methods are
  codes), and the `payment-acknowledgement` row (`method` is a code, not
  free text). Also add, near the listing text at lines 918 and 1355, one
  paragraph on the base taxonomy: the file, the Rust check, that a record
  may carry any well-formed code, and that a unit and a payment method must
  be codes. At `:1414` (Hub languages), one sentence on the taxonomy label
  files.
- **`docs/planning/traceability-matrix.md:68`** (`[APP-A11Y]`): evidence
  adds `taxonomy.test.ts`, `category_picker.test.ts` and `taxonomy/tests.rs`.
- **`docs/developer-guide.md:1745`** ("Hub text and languages"): a
  paragraph "Taxonomy labels": the label files, the id forms of D-B2-4,
  that `taxonomy.ts` is the one place that reads them, and that a new
  language adds a taxonomy file too. A new short section next to the
  Roym directory section (near `:1690`), "Add a code to the base
  taxonomy": edit `crates/roym_core/taxonomy/base.json`, add the label to
  both taxonomy files, run `cargo nextest run -p syneroym-roym-core taxonomy`
  and `mise run test:roym-ui`. At `:1702`, one sentence: a `--category` is
  a code from the base taxonomy, or a group's own code of `[a-z0-9-]`.
- **`docs/TERMINOLOGY.md`**: a row "Taxonomy (Roym)": the base lists of
  category, unit, payment-method and attribute codes in
  `crates/roym_core/taxonomy/base.json`, with labels per language. Not to
  be confused with: free-text categories.
- **`docs/demo/demo.md:1260`** (R3b): add the category pick by its Hindi
  label.
- **The change doc**, **task.md**, **status.md**: Step 9.

`docs/roym-integrated-experience-spec.md` (S2, C5, C12 say "categories"):
no change in B2; B6 updates the integrated spec (task.md, B6).

## 10. Exit criteria

B2 is done when all of these are true:

1. The owner answered Q1 to Q10, and sections 3 to 9 match the answers.
2. `base.json` has 50 to 150 categories in two levels, the units, the
   payment methods and the attributes of D-B2-2, and
   `Taxonomy::parse(BASE_JSON)` is `Ok`; each `TaxonomyError` variant has
   a failing case.
3. The service refuses a unit, a payment method, a quote method, a payment
   `method` and a SynOrg category that is not a code (W1 to W5), and
   accepts an own code of the right shape.
4. The Hub has the picker in the four forms, the unit list and the method
   boxes; every place of section 2.2 point 3 shows labels; an unknown
   code shows as it came.
5. `taxonomy/en.ftl` and `taxonomy/hi.ftl` have exactly the ids that
   `base.json` implies; Hindi has a Devanagari letter in every label.
6. The Hindi sweep passes with real codes, and fails on a raw taxonomy
   code.
7. The ML-8 Playwright test passes: the same listing ids in English and in
   Hindi.
8. Story 40 passes with its new step in `mise run demo`.
9. The built main script holds the codes and no Devanagari letter; the
   bundle sizes are reported and within section 7.6.
10. The owner reviewed the codes and the labels with
    `taxonomy-review.md`; the first comment of `taxonomy/hi.ftl` says
    `# owner review: done <date>`; `taxonomy-review.md` is deleted.
11. The change doc, the living docs, the backlog, task.md and status.md
    are updated as in sections 8 and 9.
12. No planning reference is left in `listing.rs`.
13. `mise run verify` passes with no `--skip`.
14. One PR.

## 11. What B2 does not do

- No `lang` on records, no author translations, no profile `languages`
  (B3a).
- No check of a category against the taxonomy in `validate`. A group's own
  code stays valid.
- No search by a category group, and no OR between categories (M08D,
  SH-11).
- No attribute values on a listing, no facets, no attribute filter in a
  search (M08D).
- No reader's view of another person's product block (Q9; M08D).
- No search of labels in languages other than the UI language and English
  (Q7).
- No `web` method for the taxonomy (Q2).
- No labels in languages other than English and Hindi (task.md section 8).
- No change to named areas (`Area::Named.code`): its code system is still
  an open question of the change doc.
- No transliterated search of labels (`nal` does not find "नल"): search
  keys come in B4b.
- No change to `roymctl` flags. `roymctl` shows codes, not labels
  (localised `roymctl` text is not in scope).

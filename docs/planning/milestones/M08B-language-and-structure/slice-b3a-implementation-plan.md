# M08B slice B3a — Language-tagged records: implementation plan

*Reader: the developer (or AI) who implements slice B3a. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan review round 1 done, 2026-11-09. Owner questions answered 2026-11-05. P-items folded in.**

**Size and PR.** M, at the top of M (see Q8). One PR on
`feat/m08b-b3a-lang-records`, with the commits of section 6 in that order.
The branch starts from `main` at `4c7b699c` (B2 is merged,
[PR #353](https://github.com/syneroym/syneroym/pull/353)) and already holds
the plan commit. B3a adds a language tag to every signed Roym record that
has free text: `profile`, `listing`, `request` and `quote`. The author may
add up to 4 of their own translations inside the same signed record. The
profile says which languages the person speaks. The Hub shows the version
of a record's text that best fits the reader, says when it is a
translation by the author, lets the reader open the original, and marks
every block of record text with its language. B3a adds a new substrate
crate, `syneroym-text-lang`, with the language tag type. B3a adds no
JSON-RPC method, no WIT file and no record version.

**Base.** Every file path and line number below was checked on `main` at
`4c7b699c`. Links are relative to this folder.

**Parallel work.** B4a (substrate only) may run beside B3a, but both
touch the new crate `crates/text_lang`. [task.md](task.md) section 1 says
"the crate is created by whichever of B3a and B4a merges first. The other
slice adds to it." B4a has not started. If it starts before B3a merges, it
rebases onto B3a's crate and adds `nfc()` beside `LangTag` (D-B3a-1). B3b
must wait for B3a: it uses `LangTag` in `crates/conversation`.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code or comments: no "M08B", "B3a", "ML-10" or
    "D-B3a-3" in `.rs`, `.ts`, `.ftl`, `.json` files or test names. Do not
    copy review ids into code, comments or test names (B2 R5).
  - Comments never cite size rules. B3a splits two files (D-B3a-8); their
    comments say what the code is for, not why it moved.
  - **File size.** Production files at most 800 lines. Test files at most
    800 lines, and a file in `xtask/oversized-test-files.txt` is capped at
    its recorded count. Section 7.5 lists every file B3a touches that is
    close to a limit.
  - The import clean-up pass over the Rust files you edit, the deferred
    backlog, the living-docs close-out and the demo close-out.
  - A `roym_*` crate changes, so run `mise run build:roym` before you trust
    a parity run ([task.md](task.md) section 2, "Records").
  - Every new Hub string goes through `t()` and the build check (B1b).
  - New crate naming: `crates/text_lang`, package `syneroym-text-lang`.
- [task.md](task.md): section 1 (Y3 and "Where Y3 and Y4 live"), section 2
  ("Records", "Hub"), "B3a", section 4, section 8.
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "Rules for every layer" (rule 1 "The signed original always wins", rule
  4 "No migrations"), "C. Language-tagged free text" (all of it, including
  "Tag validation" and the two Hub rules), "Agreements across languages",
  criteria ML-5, ML-10, ML-11, ML-12 and ML-13, and `## Deviations` (the
  first entry is about this crate). Every B3a decision and deviation is
  written there, and only there (section 3).
- [demo-stories](../../changes/demo-stories/change.md), read for context
  only: B3a extends story 40 (Q7) under its rule "Link between slice plans,
  the demo and AGENTS.md". B3a meets no criterion of that doc and writes
  nothing there.
- [slice-b2-implementation-plan.md](slice-b2-implementation-plan.md) (the
  layout of this plan) and the four review files. Section 7.8 lists how
  B3a avoids their kinds of findings.

### Terms used in this plan

- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **Language tag:** a BCP 47 code for a language, optionally with a script
  and a region: `hi` (Hindi), `ta` (Tamil), `en` (English), `hi-Latn`
  (Hindi written in Latin letters).
- **Canonical form (syntax form):** the tag with fixed letter case and
  order, as `icu_locale_core` prints it: `HI-latn` becomes `hi-Latn`,
  `en-us` becomes `en-US`. Old codes are not replaced: `iw` stays `iw`
  (change doc, "Tag validation").
- **Record language:** the one `lang` tag of a record. It covers every
  free-text field of the record. A record that mixes languages is tagged
  with its main language.
- **Author translation:** a version of a record's translatable fields in
  another language, written by the author and signed inside the same
  record. It is not a machine translation. The original still binds.
- **Text version:** the original text of a record, or one author
  translation of it. The Hub shows one text version at a time.
- **Reader languages:** the languages the Hub tries, in order, when it
  picks a text version: the Hub UI language, then the browser's
  `navigator.languages`.
- **UI language:** the language of the Hub's own text (B1a), stored in the
  browser. It is not part of any record.

## 1. What B3a builds, in one paragraph

B3a adds the crate `crates/text_lang` (`syneroym-text-lang`) with one type,
`LangTag`: a BCP 47 tag in canonical form, parsed with `icu_locale_core`
2.4. `roym_core` uses it. `ProfilePayload`, `ListingPayload`,
`RequestPayload` and `AgreedTerms` (the terms inside a quote and inside
each half of an agreement receipt) get a required `lang`. `ProfilePayload`,
`ListingPayload`, `RequestPayload` and `QuotePayload` get an optional
`translations` list, at most 4 entries, each with its own `lang` and the
record's translatable fields. `ProfilePayload.locale` is replaced by
`languages`, an ordered list of at most 8 tags. The four `*.set` methods
take the same fields, and a tag that is not in canonical form is refused
with `-32602`. `directory.find` hits carry the listing's `lang` and
`translations`. In the Hub, every form that signs one of these records
gets a "text language" list that starts at the UI language; the listing
editor can add author translations; the profile screen edits the
languages the person speaks. Every place that shows record text shows the
best text version for the reader, says "translated by the author" when it
is one, has a button to show the original, and sets `lang` on the text.

## How this is shown to an end user

- Alice uses the Hub in Hindi. She writes a listing. The form shows "text
  language: Hindi", because her Hub is in Hindi. She can change it.
- In the same form she adds an English version of her title and summary
  herself, with "Add a translation".
- Bob uses the Hub in English. He finds Alice's listing in the directory.
  He sees her English title, with a line "Translated by the author from
  Hindi" and a button "Show the original". The button shows her Hindi text.
- On her profile, Alice lists the languages she speaks, most used first.
- Requests and quotes in a chat look the same as before. The browser now
  knows the language of their text, so it picks the right font and a
  screen reader reads it in the right language.

How a developer can see it work:

- `cargo nextest run -p syneroym-text-lang` runs the tag tests, including
  the 15 inputs of the S0 table (`hi_IN` refused, `HI-latn` becomes
  `hi-Latn`, `iw` stays `iw`).
- A `listing.set` with no `lang`, or with `"lang": "hi_IN"`, answers
  `-32602` with `data.field = "lang"`.
- `mise run demo` runs story `demo/40-roym-languages.spec.ts`. Its new step
  shows the text language list at Hindi and an English translation row in
  the listing editor (Q7).

**Demo:** story 40 gains one step (Q7): in the listing editor of a Hindi
Hub, the text language list shows Hindi, and Alice adds an English
translation of the title. She does not save.

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. Where do a quote's `lang` and `translations` live? (blocks Step 4)**

A quote's free text is all inside `terms` (`AgreedTerms`,
[transaction.rs:136](../../../../crates/roym_core/src/transaction.rs#L136)
to `:160`): `scope`, `cancellation_terms`, `refund_terms`, `dispute_path`
and `location.address`. Both halves of an agreement receipt copy `terms`
verbatim, and `halves_agree` compares them
([transaction.rs:611](../../../../crates/roym_core/src/transaction.rs#L611)).
A receipt card is drawn from the receipt payload only; it has no quote.

- (a) **`lang` in `AgreedTerms`, `translations` on `QuotePayload`.** The
  receipt then names the language of the text that binds, with no extra
  code, and a receipt card can set `lang` on the terms (ML-5). An author
  translation stays out of the binding terms (rule 1: "the signed original
  always wins"); a reader of the receipt finds it in the quote, whose
  `record_id` the receipt pins. The later `viewed_lang` field (B5, ML-18)
  compares against `terms.lang`. *Recommended.*
- (b) **Both on `QuotePayload`.** Every record has `lang` at the top
  level, but a receipt says nothing about the language that binds, and a
  receipt card cannot mark its text.
- (c) **Both in `AgreedTerms`.** Each receipt half then also carries every
  translation, so a signed acceptance holds text that does not bind.

Owner answer: (a), 2026-11-05. It needs P1 fixed. `lang` goes in `AgreedTerms`
and `translations` on `QuotePayload`. When a quote card shows an author
translation, it adds a warning line that the original text is what binds
(`record-text-original-binds`, P1). The gap before `viewed_lang` (B5) is
recorded as a deviation.

**Q2. Which fields does an author translation hold? (blocks Step 4)**

The change doc says "the same translatable fields as the record (for a
listing: `title` and `summary`)" and "Names (`display_name`, `payee`) and
addresses are not translated".

- (a) **Profile `about`; listing `title`, `summary`; request
  `description`; quote `scope`, `cancellation_terms`, `refund_terms`,
  `dispute_path`.** Not translated: names (`display_name`, `payee`),
  addresses (`location.address`), the named-area label
  (`Area::Named.label`, a place name), the request's `data_use_notice`
  (it records the notice the person was shown and signed), and the
  listing's service lists (`includes`, `excludes`, `prerequisites`). The
  service lists get a backlog row for the shop work (M08D), where offerings
  get their full text. *Recommended:* the smallest set that covers what a
  reader reads first.
- (b) **As (a), plus the three service lists** (each a list of at most 32
  strings). A listing translation then needs list rules too.

Owner answer: (a), 2026-11-05. Translatable fields are profile `about`;
listing `title`, `summary`; request `description`; quote `scope`,
`cancellation_terms`, `refund_terms`, `dispute_path`. The listing service
lists get a backlog row for M08D.

**Q3. Which form of a tag does the wire take? (blocks Step 3)**

The change doc: "It checks that each tag is a well-formed BCP 47 tag and
stores it in syntax form: … `HI-latn` becomes `hi-Latn`." A signed record
must hold the same bytes that the struct reads back.

- (a) **Canonical form only, everywhere.** `LangTag` deserializes only a
  tag that is already canonical, and refuses `HI-latn` with
  `language tag 'HI-latn' is not in canonical form; send 'hi-Latn'`. This
  holds for method inputs and for a stranger's record. `LangTag::parse`
  (lenient) puts a tag in canonical form; `roymctl` calls it on its
  `--lang` flag, and the Hub sends tags from a fixed list that is already
  canonical (Q6). One rule; the service never changes what the client
  sent; the signed bytes are exactly the struct. Recorded as a deviation:
  the node does not rewrite `HI-latn`, it refuses it. *Recommended.*
- (b) **The service puts an input tag in canonical form before it signs;
  a signed record must already be canonical.** Friendlier to a hand-written
  client, but the input types need a second tag type (lenient) beside the
  record type (strict).

Owner answer: (a), 2026-11-05. It carries the P10 coupling, which the plan
states: the wire accepts only canonical syntax form, refusing non-canonical
tags. This couples signed records to `icu_locale_core` printer output; the S0
table is a golden test, and any printer change on upgrade requires an owner
decision.

**Q4. Is `lang` required in every method input? (blocks Step 4)**

About 145 lines name one of the four `*.set` methods: Rust parity
tests, `method_contract` tests, substrate integration tests, `roymctl`, and
the Hub (section 2.2 point 9). Every local-only input refuses an unknown
field and a missing required field with `-32602` and `data.field`
([method.rs:108](../../../../crates/roym_core/src/method.rs#L108) to
`:141`).

- (a) **Required in `profile.set`, `listing.set`, `request.set` and
  `quote.set` (in `terms`).** `roymctl roym transaction request` and
  `quote` get a required `--lang`. ML-10 says "the signer must set it":
  the client that the signer uses sets it, the node never guesses. Every
  call site changes once (mechanical). *Recommended.*
- (b) **Optional, and the node writes `und`** (BCP 47 "undetermined").
  Fewer test edits, but a record tagged `und` defeats the point, and a
  client that forgets the field gets no error.

Owner answer: (a), 2026-11-05. `lang` is required in every method input
(`profile.set`, `listing.set`, `request.set`, `quote.set`).

**Q5. In which Hub forms can the author add translations? (blocks Step 7)**

ML-11: "The author can add up to a fixed number of translations". The Hub
has four forms that sign these records: the profile form (it has no
`about` field today,
[profile.ts:14](../../../../crates/roym_web/ui/src/screens/profile.ts#L14)
to `:33`), the listing editor, the request form and the quote form.

- (a) **The listing editor only.** A listing is what strangers find and
  read, so it is where a second language matters most. The other three
  records take translations through the API, and the Hub shows them
  (Step 8). A backlog row records the other forms. *Recommended:* keeps
  B3a an M slice.
- (b) **All four forms.** The profile form also needs a new `about`
  field.

Owner answer: (a), 2026-11-05. It needs P6 fixed: translations can be added in
the listing editor only. The Hub profile form loads `about` and `translations`
from the stored profile envelope and sends them back unchanged so a Hub save
does not erase them (P6).

**Q6. Which languages does the Hub offer for a record? (blocks Step 6)**

The Hub needs a list for the "text language" control, the translation
rows and the profile languages. The Hub's own catalogs are only `en` and
`hi`, but a person may write in any language.

- (a) **A fixed list in the Hub, `RECORD_LANGUAGES`:** `en`, `hi`,
  `hi-Latn`, `bn`, `gu`, `kn`, `ml`, `mr`, `or`, `pa`, `ta`, `ta-Latn`,
  `te`, in that order, plus the tag a record already has when it is not in
  the list (editing the own profile). Each name comes from
  `Intl.DisplayNames` in the UI language ("Hindi", "हिन्दी"), so no new
  catalog message per language, and the raw tag when `Intl` has no name.
  Urdu is left out: right-to-left is out of scope (change doc
  "Not in scope"). *Recommended.*
- (b) **Free entry of any tag**, checked with `Intl.Locale`. Any language
  works, but most people do not know BCP 47 tags.
- (c) **Only the catalog languages (`en`, `hi`).** Too few: a Tamil
  speaker cannot tag Tamil text.

Owner answer: (a), 2026-11-05. It needs P12 added: a fixed list
`RECORD_LANGUAGES` in the Hub. Unit tests verify that every entry in
`RECORD_LANGUAGES` and every catalog tag is in canonical syntax form (P12).

**Q7. What does story 40 show for B3a? (blocks Step 9)**

task.md's "Demo" paragraph (owner confirmed 2026-11-05) builds story 40 in
B1a, B2 and B4b only. AGENTS.md "Mandatory Demo Close-out" asks for a story
step when a slice adds something a viewer can see; B3a adds the text
language list, the translation rows and the "translated by the author"
line.

- (a) **One step in the listing editor:** with the Hub in Hindi, Alice
  opens Listings; the text language list shows Hindi; she adds a
  translation, picks English and types an English title; the row shows.
  She does not save, so the story can run again on the same nodes (B2 did
  the same). The best-match view (Bob sees the English title) gets its
  own Playwright test in `roym-hub.spec.ts`, where a directory exists
  (section 7.3). *Recommended.*
- (b) **No story step.** The story file stays as it is, and only the
  Playwright test shows the new parts. The change doc's `demo:` list
  already names story 40, so nothing changes there.

Owner answer: (a), 2026-11-05. One step in story 40 in the listing editor: with
Hindi Hub, Alice adds an English translation row and types an English title
without saving.

**Q8. One PR or two? (blocks Step 2)**

B3a touches one new crate, `roym_core`, four services, `roymctl`, the
federated query library, about 130 test call sites, and the Hub (four
forms, four display places, a new module). That is at the top of an M
slice.

- (a) **One PR**, as task.md says for every slice. The steps of section 6
  are ordered so that Steps 2 to 5 (Rust and the wire, with the Hub only
  sending `lang`) could become a first PR if the review asks for it.
  *Recommended.*
- (b) **Two PRs:** B3a-1 (Steps 2 to 5) and B3a-2 (Steps 6 to 10). Two
  review cycles, and a status row more.

Owner answer: (a), 2026-11-05. One PR on `feat/m08b-b3a-lang-records`.

### 2.2 Points found while reading

1. **The profile today.** `ProfilePayload`
   ([person.rs:17](../../../../crates/roym_core/src/person.rs#L17) to
   `:27`) has `display_name`, `about`, `conversation_address` and
   `locale: Option<String>` (`:26`). `validate` (`:41`) does not look at
   `locale`. `profile.set` takes `ProfileSetInput`
   ([profile_ops.rs:34](../../../../crates/roym_profile/src/app/profile_ops.rs#L34)
   to `:39`) and copies `locale` into the payload (`:136` to `:141`).
   Nothing reads it. The only other `locale` user is the shape test
   ([shapes.rs:154](../../../../crates/roym_web/tests/method_contract/shapes.rs#L154),
   `"locale": "en-GB"`). A stranger's profile is checked in
   `contacts.rs` ([contacts.rs:127](../../../../crates/roym_profile/src/app/contacts.rs#L127)
   to `:133`) with the same `validate`.
2. **The listing today.** `ListingPayload`
   ([listing.rs:178](../../../../crates/roym_core/src/listing.rs#L178) to
   `:212`) has no `deny_unknown_fields`, like every record struct. Its
   error enum is at `:214`. `verify_envelope` (`:507`) deserializes the
   payload and calls `validate`; a serde error becomes
   `refused("payload: …")`. `listing.set` takes `SetListingInput`
   ([listing_ops.rs:81](../../../../crates/roym_catalog/src/app/listing_ops.rs#L81))
   and builds the payload at `:423`.
3. **The request and the quote today.** `RequestPayload`
   ([transaction.rs:163](../../../../crates/roym_core/src/transaction.rs#L163)),
   `QuotePayload` (`:189`), `AgreedTerms` (`:136`), `TransactionError`
   (`:262` to `:375`). `request.set` takes `RequestSetInput`
   ([request_ops.rs:28](../../../../crates/roym_transaction/src/app/request_ops.rs#L28))
   and builds the payload at `:90`. `quote.set` takes `QuoteSetInput`
   ([quote_ops.rs:79](../../../../crates/roym_transaction/src/app/quote_ops.rs#L79))
   with `QuoteTermsInput` (`:38`), which `into_agreed_terms` (`:57`) copies
   field by field; the payload is built at `:189`.
4. **`transaction.rs` is 776 lines**, **`transaction/tests.rs` is 799
   lines**, and **`ListingPayload::validate` is at the clippy limit (100 lines).**
   Neither file is in `xtask/oversized-test-files.txt`, so both are capped at
   800. B3a adds fields, a variant and checks to the first, and fixtures and
   tests to the second. `ListingPayload::validate` would exceed the 100-line
   clippy limit once translation checks are added. So both files are split and
   `validate` is decomposed into helper methods first, with no change of
   behaviour (D-B3a-3, D-B3a-8, P3).
5. **Card data is the record payload.** The transaction service files a
   card with `data: Some(envelope.payload)`
   ([app.rs:504](../../../../crates/roym_transaction/src/app.rs#L504)),
   and the Hub draws it
   ([messages.ts:497](../../../../crates/roym_web/ui/src/screens/messages.ts#L497)
   to `:504`). So `lang` and `translations` reach the request, quote and
   receipt cards with no change to the card path.
6. **A directory hit is built from the verified payload** on the
   searcher's node:
   [verify.rs:138](../../../../crates/roym_federated_query/src/verify.rs#L138)
   to `:155` copies `title`, `summary` and `categories` into a `Row`, and
   [merge.rs:232](../../../../crates/roym_federated_query/src/merge.rs#L232)
   copies the winning `Row` into a `MergedHit` (`:21`). The Hub's
   `MergedHit` type is in
   [search.ts:10](../../../../crates/roym_web/ui/src/directory/search.ts#L10).
   The directory server half (`search_ops.rs`) indexes `title`, `summary`
   and categories and returns envelopes; B3a does not change it.
7. **Where the Hub shows record text.**
   - Listing: the directory hit
     ([directory.ts:309](../../../../crates/roym_web/ui/src/screens/directory.ts#L309)
     to `:310`). The own listing row shows `row.title` from
     `listing.list` (own text, not a stranger's; unchanged).
   - Request: the card description
     ([request.ts:45](../../../../crates/roym_web/ui/src/cards/templates/request.ts#L45)
     to `:51`) and the notice (`:82` to `:87`).
   - Quote: scope, address, cancellation, refund, dispute
     ([quote.ts:103](../../../../crates/roym_web/ui/src/cards/templates/quote.ts#L103)
     and below). Receipt: the same rows
     ([agreement_receipt.ts:130](../../../../crates/roym_web/ui/src/cards/templates/agreement_receipt.ts#L130)
     to `:224`).
   - Profile: only the person's own profile form. No screen shows another
     person's profile record.
8. **The value sits inside a catalog message.** `terms-scope = Scope:
   { $value }` and its siblings (`terms-address`, `terms-cancellation`,
   `terms-refund`, `request-notice`;
   [en.ftl:89](../../../../crates/roym_web/ui/src/i18n/en.ftl#L89) to
   `:102`, `:124`) put record text and Hub text in one text node, so a
   `lang` on the element would also mark the label. Every Hindi message
   has the label first
   ([hi.ftl:82](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L82) to
   `:95`, `:117`). `labelled(parent, label, ...nodes)` already exists
   ([dom.ts:50](../../../../crates/roym_web/ui/src/dom.ts#L50)) and
   `terms-payee-label` already uses it.
9. **Callers of the four set methods** (`git grep -c` of the quoted
   method name over every file but `docs/planning` and `docs/archive`;
   the numbers count lines, so a few are comments):
   - `profile.set`: 28 in 11 substrate test files
     (`crates/substrate/tests/roym_*_e2e.rs`, `common/roym_group.rs`), 4 in
     the parity test `profile.rs`, 2 in `method_contract/shapes.rs`, 1 in
     parity `catalog.rs`, the Hub (`profile.ts:42`) and the Hindi sweep.
     `roymctl` has no profile command; `address.rs:28` only prints the
     method name.
   - `listing.set`: 14 in parity `catalog.rs`, 4 in parity `fixtures.rs`,
     8 in `method_contract`, 10 in substrate tests, 4 more parity files,
     the Hub (`editor.ts:255` builds the params, `listings.ts:370`).
   - `request.set`: 22 in parity files, 3 in `method_contract`, 4 in
     substrate tests and `common/roym_flow.rs`, `roymctl`
     ([transaction.rs:176](../../../../apps/roymctl/src/commands/roym/transaction.rs#L176)),
     the Hub (`messages.ts:349`).
   - `quote.set`: 26 in parity files, 3 in `method_contract`, 4 in
     substrate tests and `roym_flow.rs`, `roymctl` (`transaction.rs:257`),
     the Hub (`messages.ts:835`).
   - The Playwright specs call none of them directly; they use the Hub
     forms. So once the Hub sends `lang`, no spec changes for it.
10. **Size caps in the test files B3a edits.** In
    `xtask/oversized-test-files.txt`: parity `profile.rs` (1112),
    `transaction.rs` (846), `directory.rs` (981), `helpers.rs` (1290).
    Not listed but close to 800: parity `fixtures.rs` (798). Adding
    `"lang": "en",` as a new line in a `json!` block would pass the cap.
    `rustfmt` does not reformat the inside of `json!({ … })`, so in these
    files the new key goes on an existing line of the same block (section
    7.5).
11. **The Roym crate allowlist.** `cargo xtask check-roym-deps` lets a
    `crates/roym_*` crate depend only on a fixed list
    ([main.rs:184](../../../../xtask/src/main.rs#L184) to `:194`).
    `crates/roym_core` is one of the checked directories (its name starts
    with `roym_`), and `syneroym-text-lang` is not on the list. So
    `syneroym-text-lang` goes into `target_independent` (Step 3).
12. **`icu_locale_core` 2.4.0 is already in `Cargo.lock`** (through `url`
    and `idna`), and `deny.toml` allows `Unicode-3.0`
    ([deny.toml:101](../../../../deny.toml#L101)). S0 measured +17,317
    bytes for a test component that parses a tag.
13. **The language name helper exists.** `languageName(tag)` in
    [picker.ts:4](../../../../crates/roym_web/ui/src/i18n/picker.ts#L4) to
    `:10` uses `Intl.DisplayNames` with a `try`/`catch`. The tag match rule
    exists: `matches(requested, available)` in
    [i18n.ts:89](../../../../crates/roym_web/ui/src/i18n/i18n.ts#L89) to
    `:97` compares language and maximized script through `Intl.Locale`,
    and a tag that does not parse matches nothing. This is exactly the
    change doc's two Hub rules: a stored `iw` and a browser `he` both
    become `he` in `Intl.Locale`, and a throw is "unknown". B3a moves both
    into one module and exports them (D-B3a-9), so there is one copy.
14. **`saveProfile` finds inputs by position and asserts on `<p>`.** The e2e step
    ([hub-helpers.ts:46](../../../../crates/substrate/tests/e2e/hub-helpers.ts#L46)
    to `:53`) fills `.profile-screen input[type="text"]` number 0 and 1, and
    checks `.profile-screen p` in strict mode. The new profile controls must be
    `<select>` and buttons, never text inputs. Furthermore, every button must have
    `type="button"` so it does not submit the form, and no new element in
    `.profile-screen` may be a `<p>` (use `<ul>`, `<li>`, `<span>`) to avoid
    strict-mode locator failure (P5).
15. **The wire-words test fills every optional field**
    ([wire_words.rs:5](../../../../crates/roym_core/src/record/tests/wire_words.rs#L5)
    to `:7`). Its samples get `translations` too.
16. **The method shape snapshots** (`crates/roym_web/tests/method_shapes/`,
    rewritten with `UPDATE_METHOD_SHAPES=1`). The ones that hold a record
    payload change: `listing_verify_good`, `request_verify_good`,
    `quote_verify_good`, `agreement_get_found`, `agreement_list_with_rows`,
    `agreement_verify_good`, `fulfilment_verify_good`,
    `directory_find_with_hits`, `transaction_thread`,
    `transaction_export`, `contacts_get_found`, `contacts_list_with_rows`.
    Review the diff: only `lang`, `languages` and `translations` keys may
    appear, and `locale` goes. `listing_list_with_rows` lists titles, not
    payloads, so it should not change; if it does, find out why.
17. **The request notice and the default quote terms are UI text.** The
    Hub signs the data-use notice and the three default quote terms in the
    UI language (change doc "Text that is signed"). If a person picks a
    text language that is not the UI language, the record mixes two
    languages. The record is tagged with its main language, as the change
    doc allows. A backlog row records it (section 8).
18. **`roym-hub.spec.ts:764` selects the first `<select>` in the listing editor.**
    ([roym-hub.spec.ts:764](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L764))
    does `page.locator('.listing-editor select').first().selectOption('draft')`.
    Today the status list is the first `<select>`. Adding `textLangSelect` before
    status would break this positional pick. The status `<select>` in `listings.ts`
    gets the class `.listing-status-select`, and `roym-hub.spec.ts:764` is updated
    to target `.listing-status-select` (P4).

## 3. What B3a meets

| Criterion | What B3a does | Test |
| --- | --- | --- |
| ML-5 (record text) | Every block of record text in the Hub for `profile`, `listing`, `request`, `quote` and fulfilment receipt (at D-B3a-14) has `lang` set to the shown version's tag, or `""` when `Intl.Locale` refuses the tag. The label of a row is a separate element in the UI language. | `record_text.test.ts` ("sets lang on the shown text", "an unknown tag gives an empty lang"); `templates.test.ts` (request, quote, receipt and fulfilment receipt rows); `directory.test.ts` (hit title and summary); `roym-hub.spec.ts` (the title of a hit has `lang`) |
| ML-10 | `lang` is required on `profile`, `listing`, `request` and on a quote's `terms` (Q1). A missing tag, a malformed tag and a tag not in canonical form fail. | `text_lang` unit tests (the S0 table); `listing/tests.rs`, `transaction/tests/lang.rs`, `person.rs` tests (missing, malformed, not canonical); `method_contract` (`-32602` with `data.field = "lang"`) |
| ML-11 | Up to 4 author translations on each of the four records, each with its own tag; the listing editor adds them (Q5); the Hub shows one as "translated by the author". | The same Rust test files (count, duplicate, same as the record, empty and long fields); `editor.test.ts`, `listings.test.ts`; `record_text.test.ts` |
| ML-12 | The Hub shows the best text version for the reader languages, always says which one it shows when the record has a translation, and has a button for the original. | `record_text.test.ts` (`bestVersion` table, the note and the button); `roym-hub.spec.ts` ("an English reader sees the author's English title of a Hindi listing and can open the original") |
| ML-13 | `ProfilePayload.languages` (ordered, at most 8, no duplicate) replaces `locale`; the profile screen edits it. | `person.rs` tests; `method_contract` (`locale` is now an unknown field); `profile.test.ts` (order, add, remove, send) |

**Where decisions and deviations are written.** All five criteria are in
the roym-multilingual change doc, so every B3a decision and deviation goes
there and only there, at close-out (Step 10): decisions into "C.
Language-tagged free text" as a new "Decided <date>, language-tagged
records" list; deviations under `## Deviations`. The deviations (from the
owner answers and plan review): the quote's `lang` is in its terms (Q1); the
list of translatable fields (Q2); a tag that is not canonical is refused, not
rewritten (Q3), which couples signed records to `icu_locale_core` printer output
(P10); translations in the listing editor only (Q5); the fixed language list
(Q6); directory search does not index author translations (D-B3a-6); the reader
languages do not include the reader's own profile `languages` (D-B3a-10); ML-5
covers record text of the four records at D-B3a-14 and the fulfilment receipt
(P2), not names, own listing rows or untagged records (P7); ML-12 shows a note
only when a record has an author translation, not when a record has a single
version (P7); and B3a shows author translations on quote cards before
`viewed_lang` exists (B5), covering the gap with the warning that the original
text binds (`record-text-original-binds`, P1).

## 4. Behaviour-change audit

### Wire changes

No method is added or removed. No record version, WIT, config key or
substrate behaviour changes. Every refusal below is `-32602`.

| # | Where | Before | After |
| --- | --- | --- | --- |
| W1 | `profile.set` input | `display_name`, `about?`, `conversation_address?`, `locale?` | `display_name`, `about?`, `conversation_address?`, **`lang`** (required), **`languages?`** (0 to 8), **`translations?`** (0 to 4). `locale` is now an unknown field: `-32602`, `data.field = "locale"` |
| W2 | `profile` record payload | `locale?` | `lang`, `languages` (omitted when empty), `translations` (omitted when empty); no `locale` |
| W3 | `listing.set` input and `listing` payload | — | `lang` (required), `translations?` (0 to 4, each `{lang, title, summary?}`) |
| W4 | `request.set` input and `request` payload | — | `lang` (required), `translations?` (each `{lang, description}`) |
| W5 | `quote.set` input | `terms` without `lang` | `terms.lang` (required), top-level `translations?` (each `{lang, scope, cancellation_terms, refund_terms, dispute_path}`) (Q1) |
| W6 | `quote` payload, both `agreement-receipt` halves, and `fulfilment-receipt` payload | `terms` without `lang` | `terms.lang`; the quote also has `translations`. A receipt and fulfilment receipt copy `terms`, so they carry `terms.lang` and no translations. `halves_agree` compares `terms`, so it now also compares `lang` (P2) |
| W7 | `directory.find` output, each hit | `title`, `summary`, … | also `lang` and `translations` of the listing (D-B3a-6) |
| W8 | Every check of a stored or received record | — | A record with no `lang`, a malformed tag, or a tag not in canonical form is refused (Q3): `listing.verify`, `request.verify`, `quote.verify`, `agreement.verify`, agreement acceptance, card filing (the card shows as refused), `contacts` profile import, the directory client half (the hit goes to `refused`), and `*.import` of a backup bundle |
| W9 | `roymctl roym transaction request` and `quote` | no language flag | a required `--lang <tag>`, put in canonical form by `LangTag::parse` before the call (Q3, Q4) |

W8 means a record signed before B3a no longer verifies. The product is
not released; there is no migration (change doc rule 4, D-S0-5). A
developer node with old listings, chats or backups shows them as refused.

What the Hub sends changes too:

| # | Field | Before | After |
| --- | --- | --- | --- |
| H1 | `lang` of all four set calls | — | the text language control (starts at the UI language; for the own profile, at the stored `lang`) |
| H2 | `translations` of `listing.set` | — | the editor's translation rows, in row order |
| H3 | `languages` of `profile.set` | — | the profile's language list, in its order |

### Behaviour changes (Hub only)

| # | Before | After |
| --- | --- | --- |
| 1 | No language control in the forms | A "text language" list in the profile form, the listing editor, the request form and the quote form |
| 2 | The profile form has a name and an address | Also a list of the languages the person speaks, with add, remove and move up |
| 3 | The listing editor has one title and summary | Also "Add a translation" rows (at most 4) |
| 4 | A directory hit shows the listing's title and summary | The best text version for the reader, with a note and a "show the original" button when the listing has a translation. **English text changes here only for a listing that has a translation** |
| 5 | Request and quote cards show the original | The same, plus the best version when the record has a translation |
| 6 | Record text has no `lang` | Every block of record text has `lang`; a row label is its own `<span>` |

### Stays the same

- Every method name, every record type and `version`, every WIT file.
- The directory server half: `directory.search`, its index rows,
  `normalize_text` and the `-32602` refusal of a non-Latin query (B4b).
- Category, unit and payment-method codes.
- Every English Hub string a test or a Playwright step reads, except the
  new messages. `terms-scope` and its siblings become label messages, but
  the English text on screen is the same: "Scope: …".
- `roymctl` output (`find` still prints the original `title`).

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B3a-1. The crate `crates/text_lang` (`syneroym-text-lang`).** One
  file, `src/lib.rs`, with tests in `src/tests.rs`. Dependencies:
  `icu_locale_core = { version = "2.4", default-features = false,
  features = ["alloc"] }`, `serde` (no derive needed), `thiserror`. It
  depends on no other Syneroym crate, so `roym_core`, `data_db` and
  `crates/conversation` can all use it.

  ```rust
  /// Longest tag a record may carry. RFC 5646 section 4.4.1 asks a reader to
  /// keep at least 35 characters.
  pub const MAX_TAG_LEN: usize = 35;

  /// A BCP 47 language tag in canonical syntax form. The CLDR form (`iw` to
  /// `he`) is not used: it needs CLDR data in every guest.
  #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
  pub struct LangTag(String);

  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum LangTagError {
      #[error("language tag is longer than {MAX_TAG_LEN} bytes")]
      TooLong,
      #[error("language tag '{0}' is not a well-formed BCP 47 tag")]
      Malformed(String),
      #[error("language tag '{given}' is not in canonical form; send '{canonical}'")]
      NotCanonical { given: String, canonical: String },
  }

  impl LangTag {
      /// Any well-formed tag, put in canonical form (`HI-latn` -> `hi-Latn`).
      pub fn parse(s: &str) -> Result<Self, LangTagError>;
      /// Only a tag already in canonical form. This is the wire rule.
      pub fn parse_canonical(s: &str) -> Result<Self, LangTagError>;
      pub fn as_str(&self) -> &str;
  }
  // Display, AsRef<str>, FromStr (= parse).
  // Serialize: the string. Deserialize: String, then parse_canonical (Q3 (a)).
  ```

  `parse` checks the length first (so no long input reaches the parser),
  then calls `icu_locale_core::Locale::try_from_str`, then prints it with
  `to_string()`, then checks the length of the result. `Locale` is used,
  not `LanguageIdentifier`, so the extension tags of the S0 table
  (`en-u-nu-deva`) pass. `und` passes: a writer may say "not known". No
  cargo feature: S0 showed that a guest that calls only the tag code does
  not pay for `nfc()` (task.md, B3a), so when B4a adds `nfc()` to this
  crate, it needs no feature either.

  The canonical text is part of what a signature covers, so `Deserialize`
  enforces `parse_canonical` on verify too (Q3 (a)). This couples signed
  records to the `icu_locale_core` printer output (P10). The S0 table is a
  golden test, and any upgrade of `icu_locale_core` that changes printer
  output requires an owner decision (for example: accepting any well-formed
  tag on verify while keeping strict canonical form on input).
- **D-B3a-2. Shared record rules in `roym_core::lang`** (new file
  `crates/roym_core/src/lang.rs`, `pub mod lang;` in `lib.rs`).

  ```rust
  pub use syneroym_text_lang::{LangTag, LangTagError};

  pub const MAX_TRANSLATIONS: usize = 4;

  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum TranslationError {
      #[error("more than {MAX_TRANSLATIONS} translations")]
      TooMany,
      #[error("a translation has the record's own language '{0}'")]
      SameAsRecord(String),
      #[error("two translations have the language '{0}'")]
      Duplicate(String),
      #[error("translation '{lang}': {field} is empty")]
      Empty { lang: String, field: &'static str },
      #[error("translation '{lang}': {field} is over {max} bytes")]
      TooLong { lang: String, field: &'static str, max: usize },
  }

  /// The count and the tags: at most MAX_TRANSLATIONS, none equal to the
  /// record's tag, no two equal.
  pub fn check_translation_langs<'a>(
      record: &LangTag,
      langs: impl ExactSizeIterator<Item = &'a LangTag>,
  ) -> Result<(), TranslationError>;

  /// One translated field, with the same rule as the original field.
  pub fn check_text(
      lang: &LangTag,
      field: &'static str,
      value: &str,
      max: usize,
      required: bool,
  ) -> Result<(), TranslationError>;
  ```

  Tags are compared as canonical strings, so `hi` and `hi-IN` are two
  different translations. The limit 4 is the "small number such as 4" of
  the change doc. A quote with 4 translations of scope and three terms at
  maximum length holds ~51.2 KB of text plus ~3 KB of other fields,
  reaching ~55 KB (about 84% of the 64 KiB `MAX_PAYLOAD_BYTES` ceiling,
  P9). A request is about 20 KB, a listing about 20 KB (with the service
  lists), a profile about 5 KB.
- **D-B3a-3. The record fields** (Q1 (a), Q2 (a)). Each translation struct
  sits beside its record, has `Serialize`, `Deserialize`, and no
  `deny_unknown_fields` (no record struct has it).

  ```rust
  // person.rs
  pub const MAX_LANGUAGES: usize = 8;
  pub struct ProfileTranslation { pub lang: LangTag, pub about: String }
  pub struct ProfilePayload {
      pub display_name: String,
      pub about: Option<String>,
      pub conversation_address: String,
      pub lang: LangTag,
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub languages: Vec<LangTag>,
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub translations: Vec<ProfileTranslation>,
  }
  // PersonError gains: TooManyLanguages, DuplicateLanguage(String),
  // TranslationWithoutAbout, Translation(#[from] TranslationError).

  // listing.rs
  pub struct ListingTranslation {
      pub lang: LangTag,
      pub title: String,
      #[serde(default, skip_serializing_if = "String::is_empty")]
      pub summary: String,
  }
  // ListingPayload gains `lang: LangTag` after `summary`, and
  // `translations: Vec<ListingTranslation>` (default, skipped when empty).
  // ListingError gains Translation(#[from] TranslationError).

  // transaction.rs
  pub struct RequestTranslation { pub lang: LangTag, pub description: String }
  pub struct QuoteTranslation {
      pub lang: LangTag,
      pub scope: String,
      pub cancellation_terms: String,
      pub refund_terms: String,
      pub dispute_path: String,
  }
  // AgreedTerms gains `lang: LangTag` (first field).
  // RequestPayload gains `lang` and `translations: Vec<RequestTranslation>`.
  // QuotePayload gains `translations: Vec<QuoteTranslation>`.
  // TransactionError gains Translation(#[from] TranslationError).
  ```

  The rules in `validate`: the record's translation tags pass
  `check_translation_langs` against the record's `lang` (for a quote,
  `terms.lang`); each translated field passes `check_text` with the bound
  and the "required" rule of its original: listing `title` (required,
  `MAX_TITLE_LEN`), `summary` (optional, `MAX_SUMMARY_LEN`); request
  `description` (required, `MAX_DESCRIPTION_LEN`); quote `scope`
  (required, `MAX_SCOPE_LEN`) and the three terms texts (required,
  `MAX_TERMS_TEXT_LEN`); profile `about` (required, `MAX_ABOUT_LEN`). A
  profile with translations and no `about` is refused
  (`TranslationWithoutAbout`). "Required" means not empty after `trim`,
  as the originals check. Field order in a struct is the order above; the
  canonical encoding sorts keys anyway. `ListingPayload::validate` is at
  the clippy line limit (100 lines), so it first decomposes its `location`
  and `relationship` checks into private helper methods
  `validate_location(&self) -> Result<(), ListingError>` and
  `validate_relationship(&self) -> Result<(), ListingError>` in Step 2 with
  no change of behaviour, and then adds `self.validate_translations()?` in
  Step 4 (P3).
- **D-B3a-4. Profile `languages`.** 0 to 8 tags, most preferred first, no
  two equal. An empty list is allowed: the person did not say. The Hub
  starts a new profile's list with the UI language (D-B3a-12).
- **D-B3a-5. The method inputs** (Q4 (a)). `ProfileSetInput` drops
  `locale` and gains `lang: LangTag`, `#[serde(default)] languages:
  Vec<LangTag>`, `#[serde(default)] translations: Vec<ProfileTranslation>`.
  `SetListingInput` gains `lang` and `translations`. `RequestSetInput`
  gains `lang` and `translations`. `QuoteTermsInput` gains `lang`, copied
  by `into_agreed_terms`; `QuoteSetInput` gains `translations`. Each op
  copies the fields into the payload; the payload's `validate` checks
  them, and its error becomes `-32602` as today. The nested translation
  structs come from `roym_core` and keep its "no `deny_unknown_fields`"
  rule; the `method_contract` check only probes top-level fields
  ([check.rs:30](../../../../crates/roym_core/src/method/check.rs#L30)), so
  it needs no change.
- **D-B3a-6. Directory hits.** `Row` and `MergedHit` in
  `roym_federated_query` gain `lang: String` (the canonical tag, empty for
  a refused row, so `Row` keeps `#[derive(Default)]`) and `translations:
  Vec<ListingTranslation>`, copied from the verified payload in
  `verify.rs` and from the winning row in `merge.rs`. The directory server
  half does not index translations: its index changes in B4b anyway, and
  B4b decides then (backlog row, section 8).
- **D-B3a-7. `roymctl`.** `roym transaction request` and `roym
  transaction quote` get a required `--lang <tag>`. `handle_request`
  extracts `build_request_params(…) -> Result<Value>` (the code before
  `call_and_print`), which accepts `lang` and calls `LangTag::parse` (P11).
  `build_quote_params` accepts `lang` the same way. Both handlers send
  `as_str()`; a bad tag stops before the call with the `LangTagError` text.
  This allows unit testing `build_request_params` and `build_quote_params`.
  `roymctl` has no profile or listing command, so nothing else changes.
  `docs/demo/demo.md:1290` and `:1294` get `--lang en`.
- **D-B3a-8. Splits and decomposition first, with no change of behaviour.**
  `TransactionError` (`transaction.rs:262` to `:375`) moves to
  `crates/roym_core/src/transaction/error.rs`, with `mod error; pub use
  error::TransactionError;` in `transaction.rs`. In
  `transaction/tests.rs`, the ten tests that sign and verify an envelope
  (the eight `verify_*` tests, `verifiers_refuse_wrong_record_type` and
  `expired_quote_envelope_verifies_with_expired_flag`) move to
  `transaction/tests/verify.rs`; the fixtures (`sample_terms`,
  `sample_request`, `sample_quote`) and `sign_envelope` stay in `tests.rs`
  as `pub(super)`. Module layout: `transaction/tests.rs` + `transaction/tests/`
  (AGENTS.md "Module layout"). In `crates/roym_core/src/listing.rs`,
  `ListingPayload::validate` moves `location` and `relationship` checks into
  private methods `validate_location` and `validate_relationship` (P3). New
  language tests go in `transaction/tests/lang.rs`.
- **D-B3a-9. Hub module `src/i18n/text_lang.ts`** (new). One place for tag
  work in the Hub:

  ```ts
  /// The languages a person can pick for a record, in this order (Q6).
  export const RECORD_LANGUAGES: readonly string[] = [
    "en", "hi", "hi-Latn", "bn", "gu", "kn", "ml", "mr", "or", "pa", "ta", "ta-Latn", "te",
  ];
  /// Same language and same script; a tag `Intl.Locale` refuses matches nothing.
  export function sameLanguage(requested: string, available: string): boolean;
  /// The name of `tag` in the language `inLanguage` (default: the UI language),
  /// or the tag itself when `Intl` has no name or refuses the tag.
  export function languageName(tag: string, inLanguage?: string): string;
  /// The value for an element's `lang`: the tag, or "" when `Intl.Locale` refuses it.
  export function langAttr(tag: string): string;
  /// A `<select>` of RECORD_LANGUAGES (plus `current` when it is not in the
  /// list), each option named by languageName, starting at `current`.
  export function textLangSelect(className: string, current: string): HTMLSelectElement;
  ```

  `sameLanguage` is `matches` moved from `i18n.ts:89` (which then imports
  it), and the language picker calls `languageName(tag, tag)` (its own
  name, as today), so there is one copy of each. Every entry of
  `RECORD_LANGUAGES` and every catalog tag is checked in unit tests to be
  in canonical syntax form (`new Intl.Locale(tag).toString()` equals the
  tag, P12).
- **D-B3a-10. Hub module `src/records/record_text.ts`** (new).

  ```ts
  export interface TextVersion<F> { lang: string; fields: F; byAuthor: boolean }
  /// The UI language, then navigator.languages.
  export function readerLanguages(): string[];
  /// Index into `versions` (0 is the original): for each reader language in
  /// order, the original if it matches, else the first translation that
  /// matches; else 0.
  export function bestVersion<F>(versions: TextVersion<F>[], reader: readonly string[]): number;
  /// Draws `draw(version)` for the best version into `host`. When there is
  /// more than one version, adds a note and a button that switches between
  /// the shown version and the original.
  export function recordText<F>(
    host: HTMLElement,
    versions: TextVersion<F>[],
    draw: (v: TextVersion<F>) => Node[],
  ): void;
  ```

  The note says "Translated by the author from { $language }." for an
  author translation and "Original text, in { $language }." for the
  original; the button says "Show the original" or "Show the author's
  translation". With one version (no translations) there is no note: the
  text is the original and nothing else exists, so ML-12's "always shows
  which one it shows" has nothing to choose between. Each `draw` sets
  `lang` with `langAttr(version.lang)` on the elements that hold record
  text. The reader languages do not include the reader's own profile
  `languages`: that needs a `profile.get` before every card (backlog row).
  Message ids use the prefix `record-text-` (like `terms-`, a prefix for
  a shared part, not a screen). On the quote card (and receipt card), when
  an author translation is shown, an additional warning line states: "The
  original text, in { $language }, is what binds."
  (`record-text-original-binds`, P1). This warns the reader before tapping
  Accept until `viewed_lang` is added in B5.
- **D-B3a-11. The text language control in the forms** (Q6 (a)). The
  profile form, the listing editor, the request form and the quote form
  each get `textLangSelect(...)` in a `field(t("<screen>-text-language"),
  …)`, with the classes `profile-text-lang`, `listing-text-lang`,
  `request-text-lang`, `quote-text-lang`. It starts at
  `currentLanguage()`; the profile form, which loads the stored profile,
  starts at the stored `lang`. It is a `<select>`, so `saveProfile` keeps
  working (section 2.2 point 14). The listing editor's status `<select>`
  receives the class `.listing-status-select`, and `textLangSelect` receives
  `.listing-text-lang`, so e2e steps avoid positional selector collision (P4).
- **D-B3a-12. The profile languages list and form persistence.** Below the name
  and address, a heading "Languages you speak", one row per language (name, a
  "move up" button for every row but the first, a "remove" button), and an "add a
  language" `<select>` of the `RECORD_LANGUAGES` not yet in the list. At 8 rows
  the add list is disabled. Every new button has `type="button"` so it does not
  submit the form, and no element in `.profile-screen` is a `<p>` (use `<ul>`,
  `<li>`, `<span>`) so `saveProfile` strict mode locator does not fail (P5).
  The form loads `about` and `translations` from the stored profile envelope
  (`env.payload`) and sends them back unchanged on save so a Hub save does not
  erase them (P6). A new profile starts with the UI language. The form sends the
  list in its order.
- **D-B3a-13. The listing editor translations** (Q5 (a)). Below the
  summary, "Translations by you" with an "Add a translation" button. Each
  row: a `textLangSelect` (class `listing-translation-lang`), a title
  input (`listing-translation-title`), a summary input
  (`listing-translation-summary`) and a remove button. At 4 rows the add
  button is disabled. `ListingForm` gains `lang: string` and
  `translations: { lang: string; title: string; summary: string }[]`.
  `buildSetListingParams` trims the texts, drops nothing, and throws a
  `ListingInputError` (catalog text) for an empty translation title, a
  translation with the record's own language, or two rows with the same
  language, so the person sees the reason before the call; the service
  checks the same rules.
- **D-B3a-14. The display places** (section 2.2 points 7 and 8). The
  directory hit draws title and summary through `recordText`. The request
  card draws the description through `recordText`, and the notice without a
  `lang` attribute (the notice is written in the writer's UI language, which
  the record does not store; page language is the best default, P8). The quote
  card draws scope, cancellation, refund and dispute through `recordText`
  (address: original only, with `lang` = `terms.lang`), and when an author
  translation is shown, adds `record-text-original-binds` (P1). The receipt
  card draws its terms (no translations) with `lang` = `terms.lang`. The
  fulfilment receipt card draws its scope with
  `labelled(p, t("terms-scope-label"), valueSpan)` and `lang = terms.lang`
  (P2). The five messages `terms-scope`, `terms-address`, `terms-cancellation`,
  `terms-refund` and `request-notice` become `-label` messages
  (`terms-scope-label = Scope:`) drawn with `labelled(p, t("…-label"), valueSpan)`,
  where `valueSpan` has the `lang`. The old ids go; the English and Hindi
  label text is the old text without `{ $value }`.
- **D-B3a-15. Test data.** `cards/samples.json` gets `lang` on the
  request, `terms.lang` on the quote and receipt, and one translation on
  the quote, so the gallery shows the note. The Hindi sweep fixtures
  (`hindi-screens.test.ts`) mark record text with `Zq` as today; language
  names come from `Intl.DisplayNames` in Hindi, so they need no allowed
  Latin word. If a name comes back in Latin letters in Node's ICU, stop and
  tell the owner (do not add it to `latin-words.ts` without a reason line).
- **D-B3a-16. Messages.** New ids, English and Hindi, added in the step
  that first uses them (B1b deviation "`common-` messages are added when
  first used"): `profile-text-language`, `profile-languages-heading`,
  `profile-languages-add`, `profile-languages-move-up`,
  `profile-languages-remove`, `listing-text-language`,
  `listing-translations-heading`, `listing-translation-add`,
  `listing-translation-remove`, `listing-label-translation-title`,
  `listing-label-translation-summary`,
  `listing-error-translation-title-required`,
  `listing-error-translation-same-language`,
  `listing-error-translation-duplicate`, `request-text-language`,
  `quote-text-language`, `record-text-by-author`, `record-text-original`,
  `record-text-original-binds`, `record-text-show-original`,
  `record-text-show-translation`, and the five `-label` ids of D-B3a-14.

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches. Hub: `npm run build` and
`mise run test:roym-ui` in `crates/roym_web/ui`. Paths are from the
repository root.

### Step 1 — Commit this plan

Done in the planning session: this file and the B3a row of
[status.md](status.md).

### Step 2 — Splits and decomposition (no change of behaviour)

1. `crates/roym_core/src/transaction/error.rs` (D-B3a-8).
2. `crates/roym_core/src/transaction/tests/verify.rs` (D-B3a-8).
3. `crates/roym_core/src/listing.rs`: decompose `ListingPayload::validate` by extracting private helper methods `validate_location` and `validate_relationship` (D-B3a-3, P3).
4. `cargo nextest run -p syneroym-roym-core`: the same tests pass, the
   same number.

### Step 3 — The crate `syneroym-text-lang`

1. `crates/text_lang/Cargo.toml` (`[lints] workspace = true`),
   `src/lib.rs`, `src/tests.rs` (D-B3a-1).
2. Root `Cargo.toml`: `syneroym-text-lang = { path = "crates/text_lang" }`
   and `icu_locale_core = { version = "2.4", default-features = false,
   features = ["alloc"] }` in `[workspace.dependencies]`.
3. `xtask/src/main.rs:184`: add `"syneroym-text-lang"` to
   `target_independent` (section 2.2 point 11).
4. Tests of section 7.2 ("text_lang").
5. `cargo nextest run -p syneroym-text-lang`, `cargo xtask
   check-dep-versions`, `cargo deny check licenses`, `cargo shear`.
6. `cargo build -p syneroym-text-lang --target wasm32-wasip2` (S0 checked
   the crate; this checks our wrapper).

### Step 4 — Records, inputs, `roymctl`, and the Hub sends `lang`

This step adds required fields, so every constructor and every caller
changes in it (otherwise the workspace or the e2e suite breaks between
steps).

1. `roym_core`: `lang.rs` (D-B3a-2); `person.rs`, `listing.rs`,
   `transaction.rs` (D-B3a-3, D-B3a-4); the `syneroym-text-lang`
   dependency in `crates/roym_core/Cargo.toml`.
2. `roym_core` tests: the fixtures of `listing/tests.rs`,
   `transaction/tests.rs`, `payment/tests.rs`, `fulfilment/tests.rs`,
   `record/tests/wire_words.rs` (with `translations` filled, section 2.2
   point 15), `person.rs`; new tests of section 7.2.
3. Services (D-B3a-5): `profile_ops.rs`, `listing_ops.rs`,
   `request_ops.rs`, `quote_ops.rs` (and `quote_ops/tests.rs`). Any other
   compile error the new fields cause (for example `roym_federated_query`
   `tests/helpers.rs:96` and `:159`).
4. `roymctl` (D-B3a-7) and its tests (`find.rs` test data is a hit, not a
   payload; it needs no change unless it fails).
5. Every Rust call site of section 2.2 point 9 sends `lang` (`"en"` unless
   the test is about another language), `quote.set` in `terms`. Capped
   files: section 7.5.
6. Hub, the smallest change: `profile.ts:42`, `editor.ts`
   (`buildSetListingParams` adds `lang`), `messages.ts:349` and `:835`
   send `lang: currentLanguage()`. The screen tests that check the sent
   params add `lang`. No new control yet.
7. `mise run build:roym`, then
   `cargo nextest run -p syneroym-roym-core -p syneroym-roym-profile -p syneroym-roym-catalog -p syneroym-roym-transaction -p syneroym-roym-directory -p syneroym-roym-federated-query -p syneroym-roym-web -p roymctl`.
8. `UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web
   --test method_contract`, then read the diff of
   `crates/roym_web/tests/method_shapes/` (section 2.2 point 16). Run again
   without the variable.
9. `cargo nextest run -p syneroym-substrate` for the `roym_*_e2e` files
   (redirect the output, AGENTS.md "Context Budget").

### Step 5 — Directory hits

1. `roym_federated_query`: `Row`, `MergedHit`, `verify.rs`, `merge.rs`
   (D-B3a-6); tests in `src/tests/merge.rs` and `src/tests/verify.rs`.
2. Parity `directory_find.rs`: one assertion that a hit carries `lang` and
   `translations` on both builds.
3. Hub `directory/search.ts`: the two fields in `MergedHit`.
4. `mise run build:roym`; shapes as in Step 4.8
   (`directory_find_with_hits`).

### Step 6 — Hub: tag helpers and the record text view

1. `src/i18n/text_lang.ts` (D-B3a-9); `i18n.ts` and `picker.ts` use it.
2. `src/records/record_text.ts` (D-B3a-10) and its messages (D-B3a-16).
3. `src/i18n/text_lang.test.ts`, `src/records/record_text.test.ts`
   (section 7.1). The existing `i18n.test.ts` and `picker.test.ts` pass
   unchanged.

### Step 7 — Hub: the forms

1. The text language control in the four forms (D-B3a-11), replacing the
   `currentLanguage()` of Step 4.6. The status `<select>` in `listings.ts`
   gets class `.listing-status-select` (P4).
2. The profile languages list (D-B3a-12): buttons have `type="button"`, no
   `<p>` element (P5); the form loads and resends stored `about` and
   `translations` (P6).
3. The listing editor translations (D-B3a-13).
4. `profile.test.ts` (new if missing), `editor.test.ts`,
   `listings.test.ts`, `messages.test.ts` (section 7.1).

### Step 8 — Hub: the display places

1. `screens/directory.ts`, `cards/templates/request.ts` (notice has no `lang`, P8),
   `quote.ts` (with `record-text-original-binds`, P1), `agreement_receipt.ts`,
   `fulfilment_receipt.ts` (drawn with `labelled` and `terms.lang`, P2) (D-B3a-14);
   the label messages in `en.ftl` and `hi.ftl`; the old ids removed in the same
   step. Run
   `git grep -n "terms-scope\|terms-address\|terms-cancellation\|terms-refund\|request-notice"`
   and expect no match outside the `-label` ids (P2).
2. `cards/samples.json` (D-B3a-15).
3. `templates.test.ts`, `directory.test.ts`, `hindi-screens.test.ts`
   (section 7.1).

### Step 9 — e2e and story 40

1. `crates/substrate/tests/e2e/hub-helpers.ts`: a new
   `addListingTranslation(page, lang, title, tag = "en")` that clicks the
   add button (by `hubMessage(tag, "listing-translation-add")`), picks
   `lang` in the last `.listing-translation-lang`, and fills the last
   `.listing-translation-title`.
2. `roym-hub.spec.ts`: update line 764 to use `.listing-status-select` instead of
   positional `.first()` (P4); add the new test of section 7.3. Run
   `git grep -n "\.first()\|nth(" crates/substrate/tests/e2e` to prove no
   positional pick is left on a screen that gained a control (P4).
3. Story 40 (Q7 (a)): one more step (section 7.3).
4. `mise run test:e2e` and `mise run demo`, output redirected; keep the
   summaries for the PR text.

### Step 10 — Sizes, docs and close-out

1. Size (section 7.6): before (on `main`) and after, `npm run build`, and
   for each `dist/assets/*.js` the raw bytes and `gzip -9 -c | wc -c`; the
   six Roym `.wasm` component sizes from `mise run build:roym`. Write the
   numbers into task.md (B3a result line).
2. Change doc: decisions into "C. Language-tagged free text" as "Decided
   <date>, language-tagged records"; `## Deviations` (section 3); rewrite
   the "Tag validation" sentence of `change.md` to state that canonical
   syntax form is required and non-canonical tags are refused with `-32602`
   (P10, P13); update the ML-10 row accordingly (P13); the Test column of
   ML-5, ML-10, ML-11, ML-12 and ML-13 with the real test names; the "Profile
   languages" paragraph says "at most 8"; `demo:` stays story 40; `status`
   stays `draft` (B6 sets `done`); `living-docs-touched` lists the files of
   section 9.
3. Living docs (section 9), deferred backlog (section 8), task.md B3a
   result line, status.md.
4. The import clean-up pass over every Rust file you edited.
5. `cargo dupes report` over the changed Rust files (AGENTS.md recipe).
   The two splits must not create a duplicate group.
6. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests (vitest, `TZ=UTC`)

`src/i18n/text_lang.test.ts`

- `sameLanguage`: `hi` and `hi-IN` match; `hi` and `hi-Latn` do not; `iw`
  and `he` match; `hi_IN` and `""` match nothing.
- `languageName("ta")` in English is "Tamil"; with `activate("hi")` it has
  a Devanagari letter; `languageName("hi_IN")` is `hi_IN`.
- `langAttr("hi-Latn")` is `hi-Latn`; `langAttr("hi_IN")` is `""`.
- `textLangSelect`: the options are `RECORD_LANGUAGES` in order; a
  `current` not in the list (`"sa"`) is added and selected; option values
  are tags, option text is names.
- Canonical syntax form: every entry of `RECORD_LANGUAGES` and every
  catalog tag from `availableLanguages()` equals
  `new Intl.Locale(tag).toString()` (P12).

`src/records/record_text.test.ts`

- `bestVersion` table: original `hi` with translation `en`: reader `["en"]`
  gives 1, `["hi"]` gives 0, `["fr"]` gives 0, `["fr", "en-GB"]` gives 1;
  original `en` with translations `hi`, `hi-Latn`: reader `["hi-Latn"]`
  gives 2; reader `["he"]` with a translation `iw` picks it; a translation
  whose tag `Intl.Locale` refuses is never picked.
- `recordText` with one version: no note, no button, `lang` set.
- With two versions and an English reader: the English translation shows,
  the note says "Translated by the author from Hindi.", the button shows
  the original with `lang="hi"` and the note "Original text, in Hindi.",
  and a second click goes back.
- In Hindi, the note and the button have Devanagari letters.

`src/screens/profile.test.ts`

- A new profile: the text language starts at the UI language, the
  languages list holds the UI language.
- A stored profile with `lang: "ta"` and `languages: ["ta", "en"]` shows
  both, in order.
- Add, move up and remove change the order; the add list has no language
  already chosen and is disabled at 8. Buttons have `type="button"` and
  clicking "move up" does not submit the form (P5).
- Save sends `lang`, `languages` in order, and no `locale`.
- Save preserves stored `about` and `translations` from the profile envelope
  and sends them back unchanged (P6).

`src/listings/editor.test.ts`, `src/screens/listings.test.ts`

- `buildSetListingParams` sends `lang` and `translations` (trimmed, in row
  order; an empty summary is sent as `""`).
- An empty translation title, a translation in the record's language and
  two rows with the same language each throw `ListingInputError` with the
  catalog text (English and Hindi).
- The add button is disabled at 4 rows; remove drops the row.

`src/screens/messages.test.ts`

- The request form and the quote form send `lang` from their control,
  starting at the UI language; the quote sends it in `terms`.

`src/cards/templates/templates.test.ts`, `src/screens/directory.test.ts`

- The request, quote, receipt and fulfilment receipt rows show the same
  English text as before ("Scope: …"), and the value `<span>` has `lang`;
  the label `<span>` has none.
- A quote with an English translation, read in English, shows the
  translation, the note, and `record-text-original-binds` (P1); a receipt
  shows the original terms with `terms.lang`.
- A fulfilment receipt displays `terms.scope` with `terms-scope-label` and
  `terms.lang` (P2).
- A directory hit with a translation shows the best version and the note;
  a hit without one shows no note.

`src/i18n/hindi-screens.test.ts`

- The profile screen (with languages), the listing editor (with one
  translation row), a directory hit with a translation, and the four
  cards with a translation or terms (including fulfilment receipt, P2)
  are drawn in Hindi; no Latin word is left.

`src/i18n/catalog.test.ts` runs unchanged: every new id is used, and
`hi.ftl` has the ids of `en.ftl`.

### 7.2 Rust

`crates/text_lang/src/tests.rs` ("text_lang")

- The S0 table, row by row: `hi`, `HI` give `hi`; `hi-latn` gives
  `hi-Latn`; `hi-Latn-IN`, `ta-IN`, `und`, `hi-Deva`, `zh-hant-tw`
  (`zh-Hant-TW`), `en-u-nu-deva` give their syntax form; `en-us` gives
  `en-US`; `iw` gives `iw`; `hi_IN`, `x-private`, `toolongsubtag` and `""`
  give `Malformed`.
- A 36-byte input gives `TooLong` without calling the parser.
- `parse_canonical("hi-latn")` gives `NotCanonical { canonical: "hi-Latn" }`;
  `parse_canonical("hi-Latn")` is `Ok`.
- Serde: `"hi-Latn"` deserializes; `"hi-latn"` fails with the
  `NotCanonical` text; a `LangTag` serializes to its string.

`crates/roym_core` ("lang")

- `listing/tests.rs`: a payload JSON with no `lang` fails to deserialize
  (`verify_envelope` gives `refused("payload: missing field `lang`…")`);
  `"lang": "hi_IN"` and `"lang": "HI"` fail; each `TranslationError`
  variant from a listing (5 translations; one with the record's tag; two
  `en`; an empty title; a 161-byte title); 4 translations pass; an empty
  translation summary passes.
- `transaction/tests/lang.rs`: the same rules for a request
  (`description`) and a quote (the four texts, against `terms.lang`); a
  receipt built from a quote has the quote's `terms.lang`; `halves_agree`
  is false when only `terms.lang` differs; a quote with all five versions
  at the maximum length, plain ASCII, signs and verifies (P9).
- `person.rs` tests: `languages` with 9 tags is `TooManyLanguages`; two
  `hi` is `DuplicateLanguage`; an empty list passes; a translation with no
  `about` is `TranslationWithoutAbout`; a payload with `locale` and no
  `lang` fails to deserialize.
- `record/tests/wire_words.rs` runs with the filled `translations`.

Services and parity

- `method_contract`: `profile.set` with `"locale"` answers `-32602` with
  `data.field = "locale"`; `listing.set` with no `lang` answers `-32602`
  with `data.field = "lang"`; `quote.set` with `terms.lang: "hi-latn"`
  answers `-32602` naming `terms.lang` and the canonical form.
- Parity `catalog.rs`: a listing with `lang: "hi"` and one `en`
  translation, set and read back on both builds, gives the same envelope
  payload.
- Parity `directory_find.rs`: Step 5.2.
- `roym_federated_query`: a hit carries the payload's `lang` and
  `translations`; a refused row has `lang: ""`.
- `roymctl`: unit tests for `build_request_params` and `build_quote_params`
  (P11): `--lang HI-latn` sends `hi-Latn` and `--lang hi_IN` stops with the
  `Malformed` text, without a call.

### 7.3 Playwright

- Every spec runs with the Hub sending `lang` (Step 4.6): no step changes
  for it, except `roym-hub.spec.ts:764` which uses `.listing-status-select`
  instead of positional `.first()` (P4).
- `roym-hub.spec.ts`, new test "an English reader sees the author's
  English title of a Hindi listing and can open the original" (ML-12,
  ML-5). The Hub stays in English. Make a listing as
  `ensureSynOrgWithListing` does, but set `.listing-text-lang` to `hi`,
  type the title "झाड़ी कटाई", and call
  `addListingTranslation(page, "en", "Hedge trimming")`; publish it to the
  directory; search by its category. Find the hit by its listing id (B2
  R4). Its title is "Hedge trimming" with `lang="en"`, and the note is
  the English `record-text-by-author` text with "Hindi". Click "Show the
  original": the title is "झाड़ी कटाई" with `lang="hi"`. Then
  `chooseHubLanguage(page, "hi")`, search again: the hit shows the Hindi
  original, with the note `record-text-original` in Hindi.
- Story 40 (Q7 (a)): after the category step, the listing editor's
  `.listing-text-lang` has the value `hi`; Alice calls
  `addListingTranslation(page, "en", "Garden work", "hi")`; the row's
  title input has the value "Garden work". No save.

### 7.4 Not tested

- Whether every browser has a name for every tag of `RECORD_LANGUAGES`
  (`Intl.DisplayNames` data). The Hub shows the tag when it has none.
- Font shaping of each script on each device (change doc, "Fonts").
- The best match with the reader's own profile `languages` (not built,
  D-B3a-10).

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `crates/roym_core/src/transaction.rs` | 776 | -115 (Step 2), then about +45. Limit 800 |
| `crates/roym_core/src/transaction/tests.rs` | 799 | down by the ten moved tests (Step 2), then about +5. Limit 800 |
| `crates/roym_core/src/listing.rs` | 549 | -25 (Step 2 decomposition), then about +35. Limit 800 (P3) |
| `crates/roym_core/src/person.rs` | 102 | about +60 (tests are in the file) |
| `crates/roym_core/src/lang.rs` | new | about 80 |
| `crates/text_lang/src/lib.rs`, `tests.rs` | new | about 120, 120 |
| `crates/roym_transaction/src/app/quote_ops.rs` | 628 | about +6 |
| `crates/roym_catalog/src/app/listing_ops.rs` | 562 | about +6 |
| `crates/roym_web/tests/dual_build_parity/profile.rs` | 1112, capped | ±0: the key goes on an existing line |
| `crates/roym_web/tests/dual_build_parity/transaction.rs` | 846, capped | ±0, the same way |
| `crates/roym_web/tests/dual_build_parity/directory.rs` | 981, capped | ±0, the same way |
| `crates/roym_web/tests/dual_build_parity/helpers.rs` | 1290, capped | ±0, the same way |
| `crates/roym_web/tests/dual_build_parity/fixtures.rs` | 798 | at most +2. Put keys on existing lines |
| `crates/roym_web/ui/src/screens/messages.ts` | 923 | about +10 (TypeScript has no length gate) |
| `crates/roym_web/ui/src/screens/listings.ts` | 587 | about +60 (translation rows) |
| `crates/roym_web/ui/src/screens/profile.ts` | 72 | about +80 |
| `crates/roym_web/ui/src/records/record_text.ts` | new | about 90 |
| `crates/roym_web/ui/src/i18n/text_lang.ts` | new | about 70 |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | 1357 | about +40 (TypeScript, no gate) |

If a capped file gets shorter, set its new count in
`xtask/oversized-test-files.txt` (AGENTS.md "Mandatory Ratchet Update").
New Rust code adds to the workspace size, so the duplication percent may
fall; if `cargo xtask check-duplication` says so, lower
`MAX_EXACT_DUPLICATE_PERCENT` as its message says. Never raise it.

### 7.6 Bundle and component size

- Hub: the main script gains `text_lang.ts`, `record_text.ts` and the
  English messages. Expected under 3 KB more after `gzip -9` (after B2 it
  was 49,207 bytes). The Hindi catalog grows by under 1 KB (14,053 after
  B2). If the main script grows by more than 5 KB, stop and tell the owner.
- Guests: every Roym component links `icu_locale_core` through
  `roym_core`. S0 measured +17,317 bytes for a test component; expect
  about that for each of the six. If one grows by more than 40 KB, stop
  and tell the owner.

### 7.7 Hindi text

As in B1b and B2: common words a Hindi speaker reads on a phone, the same
Hindi word for the same thing as `hi.ftl` already uses ("listing",
"translation"). The owner reviews the new Hindi lines before the merge;
the PR text lists them (about 25 messages), so no review file is needed.

### 7.8 How B3a avoids the kinds of findings S0, B1a, B1b and B2 had

- **Two copies of one helper** (B1b R5, R7; B2 R7): the tag match and the
  language name move into one module; the translation checks are one pair
  of functions in `roym_core::lang`; the form control is one function.
- **A value from another node used as a key** (B1a P5, B1b R4): no table
  is keyed by a tag; `bestVersion` compares through `Intl.Locale`, and a
  tag that throws matches nothing.
- **A step that breaks a later step, or the e2e suite between steps** (B1b
  P4, P10): the required fields, their callers and the Hub's `lang` go in
  one step (Step 4); old message ids go in the step that replaces them.
- **A claim not checked in the code** (S0 P1): every caller count in
  section 2.2 point 9 is from `git grep`; every line is from `4c7b699c`.
- **A test that compares the wrong thing** (B2 R4): the Playwright test
  finds the hit by listing id.
- **A UI that works in jsdom but not in a browser** (B2 R1, R10, R12): the
  new controls are plain `<select>` and `<button>` elements, with no
  open or close state; the Playwright test clicks the show-original
  button in Chromium.
- **A test name or comment with a planning id** (B2 R5): none.
- **The change doc missing decisions or real deviations** (B2 R8): Step
  10.2 writes each D-B3a item and each deviation of section 3, and the
  reviewer checks them against this list.

## 8. Deferred backlog

In [deferred-backlog.md](../../deferred-backlog.md):

- **§10, row "Language tags are not CLDR-canonicalised"** (line 535):
  update the text to state that the node refuses a tag that is not in
  canonical form and does not replace old codes; add the link
  `crates/text_lang/src/lib.rs` (`LangTag`) (P13).
- **§10, new row "Only the listing editor can add author
  translations"** (Q5 (a)): requests, quotes and profiles take them
  through the API only; the profile form has no `about` field. Target
  `TBD`. Link `crates/roym_web/ui/src/screens/listings.ts`.
- **§10, new row "Listing service lists have no author translation"** (Q2
  (a)): `includes`, `excludes`, `prerequisites`. Target M08D (shop
  offerings). Link `crates/roym_core/src/listing.rs`
  (`ListingTranslation`).
- **§10, new row "The Hub ranks author translations without the reader's
  profile languages"** (D-B3a-10). Target `TBD`. Link
  `crates/roym_web/ui/src/records/record_text.ts` (`readerLanguages`).
- **§10, new row "The signed request notice and default quote terms are
  in the UI language"** (section 2.2 point 17, P8): the notice and default
  quote terms are written in the writer's UI language, but records only store
  one `lang` tag; setting the record tag on the notice would mislabel
  mixed-language records. Target `TBD`. Link
  `crates/roym_web/ui/src/screens/messages.ts`.
- **§10, new row "Quote payload exceeding 64 KiB at signing returns internal
  error"** (P9): a quote with 4 translations approaching or exceeding
  `MAX_PAYLOAD_BYTES` (64 KiB) is refused at signing with an internal error
  rather than `-32602`. Target `TBD`. Link
  `crates/roym_transaction/src/app/quote_ops.rs`.
- **§11, new row "Directory search does not index author
  translations"** (D-B3a-6). Target M08B B4b, which rebuilds the index.
  Link `crates/roym_directory/src/app/search_ops.rs`
  (`build_index_rows`).

The row "Accessibility (WCAG 2.1 AA) & localisation/i18n" (§10, line 508)
stays: B6 moves its localisation part. No new `TODO` or `FIXME` marker. If
an owner answer is not the recommendation, drop or change the matching
row.

## 9. Living docs to update at close-out

- **`docs/system-requirements-spec.md:1461`** (`[APP-A11Y]`, the
  `locale` line): the profile carries `languages`, an ordered list of up
  to 8 language tags. **`:1462`** ("Hub localisation (built)"): one
  sentence: every signed `profile`, `listing`, `request` and quote terms
  carry a language tag, the author may add up to 4 translations, and the
  Hub shows the best version for the reader with a way to open the
  original. **`:1464`** (the "Envisioned … Translated record text"
  box): it now says only machine translation is envisioned.
- **`docs/system-architecture.md`**: Cards table (`:1204` to `:1216`) for
  `request`, `quote`, `agreement-receipt` and `fulfilment-receipt` (P2); and
  prose at `:918` and `:1138` for listing record: `lang`, `translations`,
  `languages` (P13); a short paragraph near `:1418` (Hub languages) on the text
  version view and the `lang` attribute; one sentence in the crate list
  for `syneroym-text-lang` ("the language tag type, used by `roym_core`;
  `data_db` and `conversation` will use it").
- **`docs/planning/traceability-matrix.md:68`** (`[APP-A11Y]`): replace
  "Profile records support an optional locale field" with the languages
  and record tags; evidence adds `text_lang` tests, `record_text.test.ts`
  and `transaction/tests/lang.rs`.
- **`docs/developer-guide.md:1755`** ("Hub text and languages"): a
  paragraph "Record languages": `RECORD_LANGUAGES`, `textLangSelect`,
  `recordText`, and that every new place that draws record text uses
  `recordText` and `langAttr`. **§6.6** (`directory.find`, near `:2500`):
  one sentence: each hit has `lang` and `translations`; show the version
  that fits the reader and say when it is the author's translation. Near
  "Unknown fields" (`:2542`): one sentence: every method that signs a
  `profile`, `listing`, `request` or quote takes a required `lang` in
  canonical form, and a non-canonical tag answers `-32602` with the form
  to send (P13).
- **`docs/TERMINOLOGY.md`**: rows "Language tag" (BCP 47, canonical
  syntax form, `syneroym-text-lang`) and "Author translation" (signed by
  the author inside the record; not machine translation; the original
  binds).
- **`docs/demo/demo.md`**: `:1290` and `:1294` add `--lang en`
  (D-B3a-7); R3b (`:1260`) adds the text language and translation step.
- **The change doc**, **task.md**, **status.md**: Step 10.

`docs/roym-integrated-experience-spec.md`: no change in B3a; B6 updates the
integrated spec (task.md, B6).

## 10. Exit criteria

B3a is done when all of these are true:

1. The owner answered Q1 to Q8, and sections 3 to 9 match the answers.
2. `syneroym-text-lang` passes the S0 table and builds for
   `wasm32-wasip2`; `cargo xtask check-roym-deps` passes with it.
3. The four records carry `lang` (the quote in `terms`); a missing,
   malformed or non-canonical tag is refused on input and on verify.
4. Each record takes 0 to 4 author translations, and each
   `TranslationError` variant has a failing case.
5. `ProfilePayload` has `languages` and no `locale`; `profile.set` refuses
   `locale`.
6. `directory.find` hits carry `lang` and `translations` on both builds.
7. `roymctl` request and quote take `--lang`.
8. The Hub has the text language control in four forms, the profile
   languages list and the listing translation rows; it shows the best
   text version with the note and the show-original button at the four
   places of D-B3a-14; every block of record text has `lang`.
9. The Hindi sweep passes with the new screens.
10. The new Playwright test and story 40 pass.
11. The method shape diff holds only `lang`, `languages` and
    `translations` changes, and `locale` is gone.
12. The bundle and component sizes are reported and within section 7.6.
13. The owner reviewed the new Hindi lines.
14. The change doc, the living docs, the backlog, task.md and status.md
    are updated as in sections 8 and 9.
15. `mise run verify` passes with no `--skip`.
16. One PR (or two, if Q8 says so).

## 11. What B3a does not do

- No language tags on chat messages (B3b).
- No search keys, no full-text index, no indexing of author translations
  (B4a, B4b).
- No machine translation, no translate action, no `viewed_lang` or
  `viewed_via_mt` in an acceptance (B5).
- No CLDR form: `iw` stays `iw` (backlog row).
- No tag on `opinion` (M08G) or on other records (`payment-request`,
  `booking-progress`, `fulfilment-receipt`): the change doc names only
  `profile`, `listing`, `request` and `quote`.
- No `about` field in the Hub profile form, and no translation forms for
  profiles, requests or quotes (Q5).
- No translation of names, addresses, the named-area label, the data-use
  notice or the listing service lists (Q2).
- No best match by the reader's own profile `languages` (D-B3a-10).
- No migration of records signed before B3a (rule 4).
- No right-to-left layout; Urdu is not in `RECORD_LANGUAGES`.
- No change to `roymctl` output.

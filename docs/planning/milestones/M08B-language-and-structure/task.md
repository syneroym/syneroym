# M08B: Language and structure

*Reader: developers who build this sub-milestone.*

**Status: started (2026-11-05). No slice is planned yet.** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

This sub-milestone lets people use Roym in their own language and script. It also gives Roym the shared codes (categories, attributes, units, payment methods) that the shop and matching need. It builds one change doc. Read it first: [roym-multilingual](../../changes/roym-multilingual/change.md) (criteria `ML-*`). Every slice below builds part of that change doc, and no other.

M08A is done. Every record and method that this sub-milestone changes already has its M08A names and typed inputs.

**Demo:** a new story `demo/40-roym-languages.spec.ts`, built up by three slices. B1a: a person switches the Hub to Hindi, and the Hub stays in Hindi after a reload. B2: the person picks a category by its Hindi label. B4b: a search in Latin letters finds a listing with a Devanagari title. The other Hub stories stay in English. Several Hub steps in `hub-helpers.ts` find buttons by their English text (see [demo-stories](../../changes/demo-stories/change.md)). Playwright sets no locale, so the browser asks for `en-US` and the Hub shows English. If a slice changes an English string that a step uses, it updates the step in the same PR.

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| Y1 | **String catalog and the `t()` lookup** in the Hub, with English fallback. | All Hub text; taxonomy labels (B2); widget and extension labels (M08C) | B1a |
| Y2 | **Taxonomy data file** in `roym_core`: category codes with parents, attribute definitions, unit codes, payment-method codes, labels per language. | Pickers (B2); shop attributes and facets (M08D); hard filters (M08F) | B2 |
| Y3 | **Language tag type**: parse, check, canonical form (BCP 47). | Records (B3a); chat tags (B3b); forwarding origin (M08C) | B3a |
| Y4 | **Search keys**: NFC, transliteration tables, Latin folding. | Directory search, conversation search, contacts (B4b); exchange relevance (M08F) | B4b |
| Y5 | **Full-text option in `data-layer`**: a collection declares text fields; the host keeps two FTS5 tables in sync (trigram, and `unicode61` with a 1 and 2 character prefix index); a text-match filter operator. | Directory (B4b); later the exchange and the shop browse (M08D) | B4a |
| Y6 | **`syneroym:translation` WIT and the no-op host.** | Every hook point that exists (B5); later engines (M9A) | B5 |

**Where Y3 and Y4 live.** The change doc puts both in `roym_core`. But the chat store and its search run in the host crate `crates/conversation`, and that crate cannot depend on `roym_core` (`roym_core` depends on `app_host`). Chat search (B4b) needs Y4 in the host, and chat tags (B3b) need Y3 there. So Y4, and probably Y3, must live in a substrate crate that both the host and `roym_core` can use. This is an open question for the owner (see section 6). The answer goes into the change doc's `## Deviations`.

## 2. Facts checked in the code for this plan

Checked on `main` at `76b52fe6` (2026-11-05).

**Hub** (`crates/roym_web/ui`)

- 41 source files (not counting tests), about 6,900 lines. No string catalog exists.
- Visible text is set in about 420 places: 189 `textContent = <literal>` assignments (102 of them `textContent = "..."`, in 21 files), 165 calls of the `text(tag, "...")` helper (`src/dom.ts:5`), 14 `field("...")` labels, and 52 `placeholder` or `title` literals. Four files already hold word tables: `cards/wording.ts`, `groups/words.ts`, `directory/words.ts`, `listings/labels.ts`. The change doc's figure ("about 104 assignments in 37 files") counts only one of these patterns.
- `index.html` says `lang="en"`. The page font is `system-ui, -apple-system, sans-serif` (`style.css:2`).
- Money is formatted in `formatMinor` (`src/money.ts:66`).
- Dates use `toLocaleString()` or `toLocaleTimeString()` with no locale in 4 places: `screens/listings.ts:487` and `:488`, `screens/groups.ts:126` and `:563`.
- Errors: `errText()` (`src/dom.ts:12`) has 54 callers. 22 places read `.message` from an error.
- The listing editor takes categories, units and payment methods as free text (`listings/editor.ts:181`, `:204`, `:260`).
- The quote card has an "Accept these terms" button (`screens/messages.ts:626`). The ML-19 warning goes there.

**Records** (`crates/roym_core`)

- `ProfilePayload.locale` exists (`src/person.rs:26`). `profile.set` takes it in `ProfileSetInput` (`crates/roym_profile/src/app/profile_ops.rs:38`) and stores it (`:140`). Nothing reads it.
- Categories are tokens of `[a-z0-9-]`, 1 to 64 bytes, at most 8 per listing (`src/listing.rs:22`, `:24`, `:188`).
- `ProductDetail.unit` (`src/listing.rs:104`, at most 32 bytes) and `PaymentTerms.methods` (`src/listing.rs:88`, each at most 32 bytes) are free strings. `Area::Named` has a free `label` (at most 128 bytes) and an optional `code` (`src/area.rs:29`).
- `ReceiptHalf` is at `src/transaction.rs:220`. `RECORD_TYPES` is at `src/record.rs:18`.
- Since M08A, every Roym method takes a typed input from `roym_core::method`. A local-only method refuses an unknown field (`deny_unknown_fields`). The `method_contract` tests check each service's method table. A new field on a record or a method changes these types and their contract rows.
- The dual-build parity harness loads pre-built `wasm32-wasip2` components. A slice that changes a `roym_*` crate rebuilds them (`mise run build:roym`) before it trusts a parity run.

**Search**

- Directory text search: `normalize_text` keeps only `[a-z0-9 -]` (`crates/roym_core/src/directory.rs:225`). The server half uses it to build index rows (`crates/roym_directory/src/app/search_ops.rs:78`) and to fold the query (`:259`). A query that folds to nothing is refused with `-32602` (`:264`). The filter becomes `$regex`, and `data_db` compiles that to `LIKE` (`crates/data_db/src/filter.rs:286`, `:298`).
- Since M08A, the Hub and `roymctl` call `directory.find` once. The node's client half asks every saved directory with `directory.search` in one `call-all`. Each directory's server half folds the query itself.
- Chat search already uses FTS5 with the trigram tokenizer, in the host crate `crates/conversation` (`src/store/schema.rs:115`). It indexes the raw message body, not a search key. A query shorter than 3 characters falls back to a `LIKE` scan of every message (`src/store/search.rs:47`). ML-29 forbids that scan.
- Contacts: `contacts.list` (`crates/roym_profile/src/api.rs:35`) takes `favourites_only`, `offset` and `limit`. There is no text query. ML-23 needs one.
- Guests cannot run DDL: `execute-ddl` is unreachable from a service method (backlog §11). Indexes declared by `create-collection` are not used by `query` (backlog §11). B4 does not depend on either. It adds Y5 in the host instead.
- SQLite: `rusqlite` 0.38 with `bundled-sqlcipher`, which builds `libsqlite3-sys` 0.36. Its build script sets `-DSQLITE_ENABLE_FTS5`, and the bundled SQLCipher is SQLite 3.50.4. The trigram tokenizer needs 3.34 or later. The chat store above already creates a trigram table. So FTS5 and trigram work in this build. The change doc's `unverified` note on this is now checked.

**Host capabilities**

- M08A A5 added `syneroym:events@0.1.0`: a WIT package, a trait in `crates/app_host/src/lib.rs`, a WASM host and a native host, and parity tests. Y6 follows the same pattern.
- The conversation `message` WIT record has `content-type: string` (`crates/wit_interfaces/wit/conversation/conversation.wit:67`). Chat tags go next to it.

**Docs that run as tests**

- The developer guide's "Build a Roym client" section runs as demo story `30-roym-client.sh`. It shows `directory.find` and lists its query fields. A slice that changes a method the guide shows updates the guide in the same PR.

## 3. Slices

Every slice is one PR that passes `mise run verify`. Sizes are S, M and L, as in M08A.

### S0 — Checks and owner decisions (docs only, S)

- Check that `icu_locale_core` builds for `wasm32-wasip2` with no extra features. Check the size `@fluent/bundle` adds to the Hub bundle.
- Look for a pure-Rust transliteration crate for Devanagari and Tamil that builds for `wasm32-wasip2`. Decide: use it, or write fixed tables (ISO 15919 as the start).
- Get the owner's answers to the open questions in section 6 that block B1a, B2, B3a or B4b.
- Collect the first spelling test set, or record who collects it and when it is due (before B4b).
- Record the results in this file and in the change doc. The FTS5 and trigram check is done (section 2).

Change doc: roym-multilingual.

### B1a — Hub localisation: the mechanism (M)

- Y1: the i18n module, Fluent catalogs `en` and `hi`, and `t()` with English fallback.
- The language setting: first value from `navigator.languages`, kept as a local preference, survives a reload.
- `Intl` formats for dates, times, numbers and money with the Hub language. `formatMinor` stays the one place that turns minor units into text. The 4 `toLocale*` calls use the Hub language.
- `lang` on the page.
- Error codes to catalog messages, through `errText()` and the 22 `.message` reads.
- Move the four word tables and the card text into the catalog. Screens follow in B1b.
- Criteria: ML-2, ML-3, ML-4, ML-5 (page `lang` only), ML-6.
- Demo: starts story 40 (switch to Hindi, reload, still Hindi).

Change doc: roym-multilingual.

### B1b — Hub localisation: every screen string and the build check (M)

- Move every remaining visible string (section 2: about 420 places in all) into the `en` and `hi` catalogs. English text stays the same, so the e2e steps that find buttons by text keep working.
- The build check (ML-1): `npm run build` fails when a string literal is set as visible text outside the catalog.
- From this slice on, every new Hub string goes through `t()`.
- Criteria: ML-1.

Change doc: roym-multilingual.

### B2 — Taxonomy, units, payment methods, attributes (M)

- Y2 with `en` and `hi` labels. The Hub reads the same data (a build step or a `web` method; the plan decides).
- Category, unit and payment-method pickers in the Hub. Units and payment methods become codes in the listing. `ListingPayload::validate` still accepts any well-formed token (the change doc rejects checking against the taxonomy).
- `request.categories` uses the same codes.
- Most bounds in backlog row "Small M06C C5 review nits" (N-2) already exist (`listing.rs:20` to `:36`). This slice trims that row.
- Criteria: ML-7, ML-8, ML-9, ML-28 (taxonomy attribute names and choice values only).
- Demo: story 40 picks a category by its Hindi label.

Change doc: roym-multilingual.

### B3a — Language-tagged records (M)

- Y3. `lang` (required) and author `translations` on `profile`, `listing`, `request` and `quote`. `languages` replaces `locale` in `ProfilePayload` and `ProfileSetInput`.
- Typed method inputs and `method_contract` rows change to match.
- Best-match display in the Hub, with "translated by the author" and a way to open the original. `lang` on each block of record text.
- Criteria: ML-5 (record text), ML-10, ML-11, ML-12, ML-13.

Change doc: roym-multilingual.

### B3b — Language tags on chat messages (M)

- Up to 3 language tags on a chat message, main language first: the conversation WIT `message` record, the stored row in `crates/conversation`, the signed message between nodes, and both builds.
- The Hub sets the first tag from the UI language, and the sender can change it. A message with no tag shows "language not stated". The Hub sets `lang` on the message text from the first tag.
- Criteria: ML-14, ML-25.

Change doc: roym-multilingual.

### B4a — Full-text option in `data-layer` (M)

- Y5, a substrate change only: the `data-layer` WIT gains a text-field declaration on a collection and a text-match filter operator. `data_db` keeps the two FTS5 tables in sync on create, update and delete. The native shim and the WASM host behave the same (parity tests).
- A query term of 1 or 2 characters goes to the `unicode61` prefix index. Longer terms go to the trigram index. No query falls back to a scan.
- Check whether [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md) needs an amendment for the new operator.
- Criteria: none alone. Its tests are the storage half of ML-20 and ML-29.

Change doc: roym-multilingual.

### B4b — Search in every script (L)

- Y4, in the crate the owner picks (section 1).
- The directory server half moves from `normalize_text` and `$regex` to the B4a text-match operator over search keys. The `-32602` refusal for a non-Latin query goes away. The index is rebuilt in place (no migration).
- Chat search in `crates/conversation` indexes search keys and gets the short-word index, which removes the `LIKE` scan.
- `contacts.list` gains a text query over name search keys.
- Hub: names sorted with `Intl.Collator` and grouped by script.
- Closes the backlog row "The directory text index is ASCII-folded".
- Criteria: ML-20, ML-21, ML-22, ML-23, ML-24, ML-29.
- Demo: story 40 finds a Devanagari title with a Latin search.

Change doc: roym-multilingual.

### B5 — Translation interface, hook points, agreements (M)

- Y6 and the host implementation that answers `unavailable`, linked for both builds. The local-only Roym method the Hub calls (the change doc suggests `web`).
- Call the interface at the four hook points that exist in M08B: search query, record display (`listing`, `request`, `quote`), chat message, and agreement review. The Hub hides the translate action when `supported-targets` is empty.
- `viewed_lang` and `viewed_via_mt` in each party's signed acceptance (`ReceiptHalf`). The Hub warning before accepting through MT (ML-19), tested with a stub engine in a Hub unit test.
- Criteria: ML-15, ML-16, ML-17, ML-18, ML-19, ML-26 (the four hook points above), ML-27.
- Demo: nothing new is visible with the v1 engine. The plan says so.

Change doc: roym-multilingual.

### B6 — Close-out (S)

- Playwright e2e: a Hindi Hub user finds and books an English listing by category, and a Latin-letter search finds a Devanagari title.
- Check that every `ML-*` row has a test, or a named later sub-milestone (section 5).
- Living docs updated (`[APP-A11Y]` in the requirements, the integrated spec, the architecture doc). Change doc `status: done`.

Change doc: roym-multilingual.

## 4. Order and parallel work

```text
S0 ─▶ B1a ─▶ B1b ─▶ B2 ─▶ B3a ─▶ B3b ─▶ B4b ─▶ B5 ─▶ B6
  └─▶ B4a (substrate only) ──────────────▲
```

- B4a touches only `data_db`, the `data-layer` WIT and the hosts. It can run beside B1a to B3b.
- Everything else runs in order, because the slices share the Hub and `roym_core`:
  - B1b comes before B2, so every new Hub string from B2 on goes straight into the catalog under the build check.
  - B2 and B3a both change `ListingPayload` and the listing editor.
  - B3b and B4b both change `crates/conversation` and the Hub messages screen.
  - B5 needs B3a (tags on records), B3b (chat tags) and B4b (the search path).

## 5. Criteria that a later sub-milestone finishes

Some `ML-*` criteria name things that do not exist until later. M08B builds the mechanism; the later sub-milestone shows it working. Record each in the change doc's `## Deviations` when its slice is planned.

| Criterion | Part built in M08B | Part finished later |
| --- | --- | --- |
| ML-26 | Hook points: search query, record display, chat message, agreement review (B5) | Widget answer: M08C C4. Opinion: M08G. Offering, want and storefront display: M08D. |
| ML-28 | Taxonomy attribute names and choice values (B2) | Widget labels and extension step names: M08C C4 |

## 6. Open questions for the owner

These come from the change doc's open questions and from section 2. Each names the first slice it blocks.

1. **Where Y3 and Y4 live** (blocks B3a, B3b, B4b). Recommendation: one new substrate crate (for example `crates/text_lang`, package `syneroym-text-lang`) that holds the language tag type and the search keys. `roym_core`, `data_db` and `crates/conversation` all use it. Then `data_db` computes the search keys itself for a declared text field, and a guest sends raw text.
2. **Catalog format: Fluent** (blocks B1a). The change doc recommends it.
3. **Who checks the Hindi text** (blocks B1a and B2). An agent can write the first Hindi catalog. A person who reads Hindi should review it before B1b merges.
4. **Native digits** (blocks B1a). Recommendation: Latin digits (`123`) by default.
5. **Who maintains the taxonomy, and the first taxonomy** (blocks B2). Recommendation: the Roym project keeps a small hand-made base taxonomy (50 to 150 codes) in the repository.
6. **Spelling test data** (blocks B4b). Who collects it, and how many pairs (the change doc suggests 200 per language).
7. **The demo story 40 plan above** (B1a). Confirm it.
8. **Named area codes.** No M08B slice needs this. It stays open in the change doc.

## 7. Deferred-backlog rows for M08B

| Row | Section | What M08B does |
| --- | --- | --- |
| Accessibility (WCAG 2.1 AA) & localisation/i18n | §10 | Localisation is built in B1a to B3b. B6 moves the localisation part to "Recently resolved". Accessibility stays. |
| The directory text index is ASCII-folded | §11 | B4b closes it. |
| Small M06C C5 review nits (N-2: unbounded listing fields) | §5 | The bounds exist now. B2 trims the row. |
| Three list methods return a bare JSON array | §10 | Not triggered: B4b adds an input field to `contacts.list`, not an output field. If a plan adds an output field, it wraps the three lists then. |
| More demo stories | §13 | Story 40 adds one. |

## 8. Not in this plan

- A real translation engine (with M9A local inference).
- Right-to-left scripts, voice, catalogs beyond `en` and `hi`.
- Translating the taxonomy into languages other than English and Hindi.
- Localised error text from the substrate or from `roymctl`.

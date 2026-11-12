# M08B: Language and structure

*Reader: developers who build this sub-milestone.*

**Status: started (2026-11-05). No slice is planned yet.** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

This sub-milestone lets people use Roym in their own language and script. It also gives Roym the shared codes (categories, attributes, units, payment methods) that the shop and matching need. It builds one change doc. Read it first: [roym-multilingual](../../changes/roym-multilingual/change.md) (criteria `ML-*`). Every slice below builds part of that change doc, and no other.

M08A is done. Every record and method that this sub-milestone changes already has its M08A names and typed inputs.

**Demo** (owner confirmed, 2026-11-05)**:** a new story `demo/40-roym-languages.spec.ts`, built up by three slices. B1a: a person switches the Hub to Hindi, and the Hub stays in Hindi after a reload. B2: the person picks a category by its Hindi label. B4b: a search in Latin letters finds a listing with a Devanagari title. The other Hub stories stay in English. Several Hub steps in `hub-helpers.ts` find buttons by their English text (see [demo-stories](../../changes/demo-stories/change.md)). Playwright sets no locale, so the browser asks for `en-US` and the Hub shows English. If a slice changes an English string that a step uses, it updates the step in the same PR.

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| Y1 | **String catalog and the `t()` lookup** in the Hub, with English fallback. | All Hub text; taxonomy labels (B2); widget and extension labels (M08C) | B1a |
| Y2 | **Taxonomy data file** in `roym_core`: category codes with parents, attribute definitions, unit codes, payment-method codes. Codes and structure in the data file; labels in the Hub's taxonomy label files (`src/i18n/taxonomy/<tag>.ftl`). | Pickers (B2); shop attributes and facets (M08D); hard filters (M08F) | B2 |
| Y3 | **Language tag type** in `syneroym-text-lang`: parse, check, canonical form (BCP 47). | Records (B3a); chat tags (B3b); forwarding origin (M08C) | B3a |
| Y4 | **Search keys** in `syneroym-text-lang`: NFC, transliteration tables, Latin folding. | Directory search, conversation search, contacts (B4b); exchange relevance (M08F) | B4b |
| Y5 | **Full-text option in `data-layer`**: a collection declares text fields; the host keeps two FTS5 tables in sync (trigram, and `unicode61` with a 1 and 2 character prefix index); a text-match filter operator. | Directory (B4b); later the exchange and the shop browse (M08D) | B4a |
| Y6 | **`syneroym:translation` WIT and the no-op host.** | Every hook point that exists (B5); later engines (M9A) | B5 |

**Where Y3 and Y4 live (owner decision, 2026-11-05).** The change doc puts both in `roym_core`. But the chat store and its search run in the host crate `crates/conversation`, and that crate cannot depend on `roym_core` (`roym_core` depends on `app_host`). Chat search (B4b) needs Y4 in the host, and chat tags (B3b) need Y3 there. So Y3 and Y4 live in one new substrate crate, `crates/text_lang` (package `syneroym-text-lang`). `roym_core`, `data_db` and `crates/conversation` all use it. `data_db` computes the search keys itself for a declared text field, so a guest sends raw text, both when it writes and when it queries. The crate is created by whichever of B3a and B4a merges first. The other slice adds to it. The change doc's `## Deviations` records this.

**Component size limit (owner decision, 2026-11-10).** Each slice that changes a `roym_*` crate reports the six Roym component sizes for `wasm32-wasip2`, before (on `main`) and after, built in the same build folder. Two limits apply:

- **Shared code.** A component that gets no new record code in the slice shows what the shared libraries add (in B3a, conversation). If it grows by more than 40 KB, stop and tell the owner.
- **Record code.** A component that gets new record fields or methods may grow more, because it pays for that code. If one grows by more than 15% of its size before, stop and tell the owner. If every component gets new record code, only this limit applies.

B3a set this rule. Its four record components grew by 6% to 11% (+76 KB to +112 KB), and the owner accepted that.

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
  - **Result ([PR](https://github.com/syneroym/syneroym/pull/348)):** `icu_locale_core` 2.4.0 builds for `wasm32-wasip2` and runs in `wasmtime` with `default-features = false, features = ["alloc"]`. The parse functions need `alloc`. No other feature is needed. A test component grows by 17,317 bytes (75,645 to 92,962). The canonical form is the syntax form only: `hi-latn` becomes `hi-Latn`, and `iw` and `hi-Deva` stay as given. `hi_IN` (underscore) is refused. Of 15 inputs, the only difference from the browser's `Intl.Locale` is `iw` (Rust keeps `iw`, the browser gives `he`). The table is in the change doc, "Tag validation" (D-S0-1).
  - **Result:** NFC (added to S0 by owner question Q3): `icu_normalizer` 2.4.0 with `default-features = false, features = ["compiled_data"]` builds for `wasm32-wasip2`, runs natively, and gave the expected output for six inputs (`U+0958` becomes `U+0915 U+093C`; `U+0B95 U+0BC6 U+0BBE` becomes `U+0B95 U+0BCA`; `U+0065 U+0301` becomes `U+00E9`; the others do not change). `unicode-normalization` 0.1.26 gave the same output. Size added to a test component: 68,734 bytes for `icu_normalizer`, 124,338 bytes for `unicode-normalization`. D-S0-2: `icu_normalizer`. A guest that uses only the tag type does not pay for NFC code it never calls. We measured a library with a public `nfc()` that calls `icu_normalizer`, and a guest that calls only the tag function: 93,305 bytes with and without `nfc()` in the library, and 162,069 bytes (+68,764) when the guest also calls `nfc()`. (The code review measured the same case in another build folder: 93,827 bytes both ways and +68,659.) Compare only numbers from one build folder: the same build gave 92,962 bytes above and 92,994 bytes in a second folder, because the paths inside the binary differ by 32 bytes.
  - **Result:** `@fluent/bundle` 0.19.1 (Apache-2.0, no runtime dependencies) adds 11,443 bytes raw and 4,205 bytes after `gzip -9` to the Hub JavaScript (114,463 to 125,906 raw; 31,769 to 35,974 gzipped). The limit was 25 KB after `gzip -9`. There is no `eval`, `Function(` or `new Function` in the built code, before or after. The lock file gains only `@fluent/bundle`, and Vite 5.4.21 and Rollup 4.63.0 do not change. D-S0-4: Fluent stays.
- Look for a pure-Rust transliteration crate for Devanagari and Tamil that builds for `wasm32-wasip2`. Decide: use it, or write fixed tables (ISO 15919 as the start).
  - **Result ([PR](https://github.com/syneroym/syneroym/pull/348)):** D-S0-3: we write our own tables, with ISO 15919 as the start. One crate, `lipilekhika`, met all seven rules (rule 7 from a summary of its changelog only, and its 1.1.0 release broke its API). We still write our own tables: its output is close to ISO 15919 but differs in places we do not want (see below), and every stored search key depends on the exact output. Seven crates were tried in one `wasm32-wasip2` build (dates from crates.io, 2026-11-05). "P" is pass, "F" is fail, "?" is not clear, "-" is not checked because an earlier rule failed.

    | Crate | Version | License | Last release | 1 | 2 | 3 | 4 | 5 | 6 | 7 | Adds to a test component |
    | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
    | `icu_experimental` | 0.7.0 | Unicode-3.0 | 2026-10-22 | P | P | P | P | P | P | F | 2,212,625 bytes |
    | `lipilekhika` | 1.1.2 | MIT | 2026-05-31 | P | P | P | P | P | P | P | 194,388 bytes |
    | `vidyut-lipi` | 0.2.0 | MIT | 2025-01-08 | P | P | P | P | F | P | - | 189,990 bytes |
    | `shlesha` | 0.5.7 | MIT OR Apache-2.0 | 2026-07-06 | ? | P | P | F | P | P | F | 648,864 bytes |
    | `transliteratio` | 0.1.0 | MIT OR Apache-2.0 | 2026-10-19 | P | P | P | F | P | P | - | 294,200 bytes |
    | `deunicode` | 1.6.2 | BSD-3-Clause | 2025-04-27 | P | P | P | F | F | P | - | 484,210 bytes |
    | `any_ascii` | 0.3.3 | ISC | 2025-06-29 | P | P | P | F | F | P | - | 583,903 bytes |

    Why each failure: `icu_experimental` has Devanagari and Tamil rule sets (`und-Latn-t-und-deva`, `und-Latn-t-und-taml`) but is a 0.x crate with an unstable data API (`try_new_unstable`), and it had four minor releases (0.4 to 0.7) in the last 12 months, where a 0.x minor release may change the API or the data (rule 7). We did not compare its output across versions. It is also the largest. `vidyut-lipi` has had no release for 22 months. `shlesha` writes wrong Tamil (`சென்னை` gives `ceனனai`), its 0.5.5 release changed Tamil output, and the `cc` crate is in its build graph through `blake3` and `psm` (we did not check whether it compiles C for this target). `transliteratio` leaves Tamil unchanged. `deunicode` and `any_ascii` drop vowels (`नल` gives `nl`) and are older than 12 months. `lipilekhika` passes all rules: its build script is pure Rust, and its changelog for 1.0.8, 1.1.0 and 1.1.2 lists no output change (we read a summary of the changelog, not the file). The 1.1.0 release did break its API. Its output is close to ISO 15919, but it differs where we want the standard form: `kzilā` for `क़िला` (ISO 15919: `qilā`), `cheṉṉai` for `சென்னை` (`ceṉṉai`, as the other two working crates give) and `ṁ` for Tamil `ம்` in `piḷaṁpar` (Tamil has no anusvara). Our own tables give the ISO form directly.

    Sample output (as printed; `lipilekhika` is the `Romanized` target):

    | Input | `lipilekhika` | `vidyut-lipi` | `icu_experimental` | `shlesha` | `transliteratio` | `deunicode` | `any_ascii` |
    | --- | --- | --- | --- | --- | --- | --- | --- |
    | `नल वाला` | `nala vālā` | `nala vālā` | `nala vālā` | `nala vālā` | `nala vālā` | `nl vaalaa` | `nl vala` |
    | `राम` | `rāma` | `rāma` | `rāma` | `rāma` | `rāma` | `raam` | `ram` |
    | `कमला` | `kamalā` | `kamalā` | `kamalā` | `kamalā` | `kamalā` | `kmlaa` | `kmla` |
    | `क़िला` | `kzilā` | `qilā` | `qilā` | `kailā` | `qilā` | `k'ilaa` | `k'ila` |
    | `प्लंबर` | `plaṁbara` | `plaṁbara` | `plambara` | `plaṁbara` | `plaṁbara` | `plNbr` | `plmbr` |
    | `தமிழ்` | `tamiḻ` | `tamiḻ` | `tamiḻ` | `tamiḻ` | `தமிழ்` | `tmilll` | `tmil` |
    | `காந்தி` | `kānti` | `kānti` | `kānti` | `kānti` | `காந்தி` | `kaanti` | `kanti` |
    | `சென்னை` | `cheṉṉai` | `ceṉṉai` | `ceṉṉai` | `ceனனai` | `சென்னை` | `cennnnnnai` | `cennai` |
    | `பிளம்பர்` | `piḷaṁpar` | `piḷampar` | `piḷampar` | `piளmpar` | `பிளம்பர்` | `pillmpr` | `pilmpr` |

    The three crates that work (`lipilekhika`, `vidyut-lipi`, `icu_experimental`) agree on the letters. They differ on the nukta (`क़`), on `ं` before `ब` (`ṁ` or `m`) and on a few Tamil letters. These are the places where the folding rules must be tested.
- The owner answered questions 1 to 7 in section 6 (2026-11-05).
  - **Result:** the S0-only questions (Q1 to Q7 in the S0 plan) were answered on 2026-11-05 and are recorded in the S0 plan, section 2.1. D-S0-5: record versions stay at 1 (ADR-0018 section 5 is the project-wide "no migration" position).
- Draft the spelling test set: 200 word pairs per language, for Hindi and Tamil typed in Latin letters. Each pair is a word in its own script and one common Latin spelling. Add spelling variants that must match each other (for example `nal wala`, `naal waala`, `nalwala`). Write it to `spelling-pairs.tsv` in this folder. The owner checks it before the B4b plan is written. B4b then moves it into the test data of `syneroym-text-lang`.
  - **Result ([PR](https://github.com/syneroym/syneroym/pull/348)):** [spelling-pairs.tsv](spelling-pairs.tsv) has 200 `pair` rows per language, plus 274 Hindi and 266 Tamil `variant` rows, 20 `joined` rows each, and 43 Hindi and 29 Tamil `distinct` rows. The check script found no format error and reported 17 conflicts: a spelling edit that a `variant` or `joined` row says must meet, and a `distinct` row in another group says must not meet (for example `a→e` and `u→a`). They are listed in the file's `# conflicts:` block, each with "owner decision: pending". The file says `# owner check: pending`.
- Record the results in this file and in the change doc. The FTS5 and trigram check is done (section 2).
  - **Result:** done. The change doc's `unverified` notes for the four checks, the trigram tokenizer and the record version rule are replaced by the results above. The `unicode61` table with `prefix='1 2'` is not checked yet; B4a checks it first. The old code facts in the change doc were corrected. One backlog row was added ("Language tags are not CLDR-canonicalised").

**S0 result:** done, docs only ([PR #348](https://github.com/syneroym/syneroym/pull/348)). D-S0-1 to D-S0-5 are recorded, and `spelling-pairs.tsv` is drafted. The owner check of the spelling file and its 17 conflicts is still to do, before the B4b plan. No end user sees a change.

Change doc: roym-multilingual.

### B1a — Hub localisation: the mechanism (M)

- Y1: the i18n module, Fluent catalogs `en` and `hi`, and `t()` with English fallback.
- The language setting: first value from `navigator.languages`, kept as a local preference, survives a reload.
- `Intl` formats for dates, times, numbers and money with the Hub language. `formatMinor` stays the one place that turns minor units into text. The 4 `toLocale*` calls use the Hub language.
- `lang` on the page.
- Error codes to catalog messages, through `errText()` and the 22 `.message` reads.
- Move the four word tables and the card text into the catalog. Screens follow in B1b.
- B1a reports the `gzip -9` size of `@fluent/bundle` and the two catalogs together, not only the library (S0 measured the library alone: +4,205 bytes; the limit is 25 KB for the library, D-S0-4).
- Criteria: ML-2, ML-3, ML-4, ML-5 (page `lang` only), ML-6.
- Demo: starts story 40 (switch to Hindi, reload, still Hindi).

Change doc: roym-multilingual.

**B1a result:** done on the branch `feat/m08b-b1a-localization-mechanism`. Size with `gzip -9`: the main Hub script grew from 31,769 to 39,819 bytes (+8,050, library and English catalog), and the Hindi catalog is its own script file of 4,763 bytes (+12,813 in all). The new `hub-ui` gate of `mise run verify` (`mise run test:roym-ui`) took about 26 seconds with a warm npm cache (`npm ci` and 230 tests in 27 files).

### B1b — Hub localisation: every screen string and the build check (M)

- Move every remaining visible string (section 2: about 420 places in all) into the `en` and `hi` catalogs. English text stays the same, so the e2e steps that find buttons by text keep working.
- The build check (ML-1): `npm run build` fails when a string literal is set as visible text outside the catalog.
- From this slice on, every new Hub string goes through `t()`.
- Criteria: ML-1.

Change doc: roym-multilingual.

**B1b result:** done on the branch `feat/m08b-b1b-localization-screens`. Every screen, the listing editor, the directory and membership words, the sign-in errors and the Hub-made errors are in `en.ftl` and `hi.ftl` (644 messages each, up from 159). `npm run build` runs `visible-text.test.ts` and fails on a literal that reaches shown text; `hindi-screens.test.ts` draws each screen in Hindi. Size with `gzip -9`, measured on the same base and after: the main Hub script went from 39,888 to 45,160 bytes (+5,272; limit +10 KB) and the Hindi catalog from 4,763 to 13,980 bytes (limit 30 KB). The main script still has no Devanagari letter. The Hindi sweep found that Hindi times show Latin `am` and `pm` (backlog row). Code review: 3 rounds, 7 minor items, all fixed. The owner reviewed all of `hi.ftl` on 2026-11-07 with no changes.

### B2 — Taxonomy, units, payment methods, attributes (M)

- Y2 with `en` and `hi` labels. The Hub reads the same data (a build step or a `web` method; the plan decides).
- Category, unit and payment-method pickers in the Hub. Units and payment methods become codes in the listing. `ListingPayload::validate` still accepts any well-formed token (the change doc rejects checking against the taxonomy).
- `request.categories` uses the same codes.
- Most bounds in backlog row "Small M06C C5 review nits" (N-2) already exist (`listing.rs:20` to `:36`). This slice trims that row.
- Criteria: ML-7, ML-8, ML-9, ML-28 (taxonomy attribute names and choice values only).
- Demo: story 40 picks a category by its Hindi label.

Change doc: roym-multilingual.

**B2 result:** done on the branch `feat/m08b-b2-taxonomy`. Size with `gzip -9`: the main Hub script went from 45,160 to 49,207 bytes (+4,047; limit 5 KB), the Hindi catalog is 14,053 bytes (was 13,980 in B1b), and the lazy Hindi taxonomy chunk is 2,518 bytes (limit 8 KB). Base taxonomy has 105 categories (14 groups, 91 leaf categories), 17 product units, 6 payment methods, and 6 attributes with 16 choices (150 codes in all). Unit tests (374 UI tests, 21 Rust taxonomy tests) and E2E tests (including Story 40 category pick in Hindi) pass. Code review: 4 rounds, 12 items, all fixed. The owner reviewed the taxonomy codes and Hindi labels on 2026-11-08 with no changes.

### B3a — Language-tagged records (M)

- Y3, in `syneroym-text-lang` (section 1). `lang` (required) and author `translations` on `profile`, `listing`, `request` and `quote`. `languages` replaces `locale` in `ProfilePayload` and `ProfileSetInput`.
- Y3 uses `icu_locale_core` 2.4.0 with `default-features = false, features = ["alloc"]`, syntax form only (D-S0-1). The underscore form `hi_IN` is refused. `iw` stays `iw`. The Hub follows two rules for tags from other nodes: it wraps every `new Intl.Locale(tag)` in `try`/`catch` and treats a throw as an unknown tag, and it compares both the stored tag and its `Intl.Locale` form when it matches languages.
- With a library that has a public `nfc()` calling `icu_normalizer`, a guest that calls only the tag function is 93,305 bytes, with or without `nfc()` in the library. A guest that also calls `nfc()` is 162,069 bytes. So a cargo feature to split the two is not needed on size grounds (the plan decides).
- Typed method inputs and `method_contract` rows change to match.
- Best-match display in the Hub, with "translated by the author" and a way to open the original. `lang` on each block of record text.
- Criteria: ML-5 (record text), ML-10, ML-11, ML-12, ML-13.

Change doc: roym-multilingual.

**B3a result:** done on the branch `feat/m08b-b3a-lang-records`. Size with `gzip -9`: the main Hub script went from 49,207 to 51,902 bytes (+2,695; limit 5 KB), the Hindi catalog is 14,377 bytes (was 14,053 in B2; +324 bytes, limit 1 KB), and the lazy Hindi taxonomy chunk is 2,539 bytes. The six Roym components build for `wasm32-wasip2` (before on `main` at `4c7b699c` -> after): web 253,711 -> 255,652 bytes (+1,941), conversation 750,997 -> 769,589 bytes (+18,592), profile 690,350 -> 766,889 bytes (+76,539), catalog 840,266 -> 927,674 bytes (+87,408), transaction 1,393,521 -> 1,505,300 bytes (+111,779), directory 1,376,221 -> 1,463,108 bytes (+86,887). The growth in profile, catalog, transaction and directory is over the +40 KB stop limit of the slice plan. The owner accepted it on 2026-11-10 (change doc, Deviations). Conversation's +18,592 bytes is the shared cost of the tag type; the rest is the new record code. Unit tests, dual-build parity tests, Playwright tests and Story 40 pass.

### B3b — Language tags on chat messages (M)

- Up to 3 language tags on a chat message, main language first: the conversation WIT `message` record, the stored row in `crates/conversation`, the signed message between nodes, and both builds.
- The Hub sets the first tag from the UI language, and the sender can change it. A message with no tag shows "language not stated". The Hub sets `lang` on the message text from the first tag.
- Criteria: ML-14, ML-25.

Change doc: roym-multilingual.

**B3b result:** done on the branch `feat/m08b-b3b-chat-lang-tags`. Size with `gzip -9`: the main Hub script went from 51,902 to 52,297 bytes (+395 bytes; limit 5 KB), the Hindi catalog is 14,447 bytes (was 14,377 in B3a; +70 bytes, limit 1 KB), and the lazy Hindi taxonomy chunk is 2,539 bytes. The six Roym components build for `wasm32-wasip2` (before in B3a -> after): web 255,652 -> 255,652 bytes (+0), conversation 769,589 -> 810,737 bytes (+41,148 bytes, +5.3%; limit 15%), profile 766,889 -> 767,699 bytes (+810 bytes; limit 40 KB), catalog 927,674 -> 927,744 bytes (+70 bytes; limit 40 KB), transaction 1,505,300 -> 1,507,245 bytes (+1,945 bytes; limit 40 KB), directory 1,463,108 -> 1,463,187 bytes (+79 bytes; limit 40 KB). All sizes within limits. Unit tests, dual-build parity tests, Playwright tests and Story 40 pass.

### B4a — Full-text option in `data-layer` (M)

- Y5, a substrate change only: the `data-layer` WIT gains a text-field declaration on a collection and a text-match filter operator. `data_db` keeps the two FTS5 tables in sync on create, update and delete. It indexes the search key of each declared field, from `syneroym-text-lang`. In B4a the key is NFC and lower case only. B4b adds transliteration and folding, and B4a's tables need no change for it. The native shim and the WASM host behave the same (parity tests).
- NFC comes from `icu_normalizer` 2.4.0 (D-S0-2). First check that the `rusqlite` `bundled-sqlcipher` build supports a `unicode61` table with `prefix='1 2'` (one- and two-letter prefix queries return the right rows). S0 checked only the trigram table.
- A query term of 1 or 2 characters goes to the `unicode61` prefix index. Longer terms go to the trigram index. No query falls back to a scan.
- Check whether [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md) needs an amendment for the new operator.
- Criteria: none alone. Its tests are the storage half of ML-20 and ML-29.

Change doc: roym-multilingual.

**B4a result:** done on the branch `feat/m08b-b4a-full-text` (2026-11-11 B4a Implemented Gemini 3.8 Flash). The six Roym components build for `wasm32-wasip2` (before on main at `bf23a422` -> after on branch in the same build folder): web 255,652 -> 255,811 bytes (+159), conversation 810,737 -> 810,779 bytes (+42), profile 767,699 -> 768,511 bytes (+812), catalog 927,744 -> 928,547 bytes (+803), transaction 1,507,245 -> 1,507,226 bytes (-19), directory 1,463,187 -> 1,463,980 bytes (+793). All sizes within limits (+40 KB limit). Unit tests, dual-build parity tests, Playwright tests and Story 40 pass.

### B4b — Search in every script (L)

- Y4, in `syneroym-text-lang` (section 1): transliteration tables and Latin folding on top of B4a's NFC and lower-case key. We write our own tables, ISO 15919 as the start (D-S0-3). The test data is `spelling-pairs.tsv` after the owner check.
- Joined words: a trigram match finds the query as a substring of the key, and `nalwala` is not a substring of `nal wala`. So the trigram table can index the key with spaces removed, and the `unicode61` table the key with spaces. That changes the key function, not the table shape.
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

1. **Where Y3 and Y4 live.** Answered 2026-11-05: one new substrate crate, `syneroym-text-lang` (section 1).
2. **Catalog format.** Answered 2026-11-05: Fluent (`@fluent/bundle`).
3. **Who checks the Hindi text.** Answered 2026-11-05: an agent writes the first Hindi catalog and the Hindi taxonomy labels. The owner reviews them before B1b merges (catalog) and before B2 merges (labels).
4. **Native digits.** Answered 2026-11-05: Latin digits (`123`) by default in every language.
5. **Who maintains the taxonomy, and the first taxonomy.** Answered 2026-11-05: the Roym project keeps a small hand-made base taxonomy, 50 to 150 codes, in the repository. A group may use its own codes at once (they show as raw codes) and proposes them for the base taxonomy through the normal contribution process.
6. **Spelling test data.** Answered 2026-11-05: 200 word pairs per language (Hindi and Tamil in Latin letters). An agent drafts them in S0, and the owner checks them before the B4b plan is written.
7. **The demo story 40 plan above.** Answered 2026-11-05: confirmed.
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

# M08B: Language and structure

*Reader: developers who build this sub-milestone.*

**Status: planning (2026-10-26).** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

This sub-milestone lets people use Roym in their own language and script, and gives Roym the shared codes (categories, attributes, units) that the shop and matching need. It builds one change doc. Read it first: [roym-multilingual](../../changes/roym-multilingual/change.md) (criteria `ML-*`).

It starts after M08A's rename (A1) and typed methods (A2), because it adds fields to the same records and methods.

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| Y1 | **String catalog and the `t()` lookup** in the Hub, with English fallback. | All Hub text; widget and extension labels (M08C); taxonomy labels | B1 |
| Y2 | **Taxonomy data file** in `roym_core`: category codes with parents, attribute definitions, unit codes, payment-method codes, labels per language. | Pickers (B2); shop attributes and facets (M08D); hard filters (M08F) | B2 |
| Y3 | **Language tag type** in `roym_core`: parse, check, canonical form (BCP 47). | Records (B3); chat tags (B3); forwarding origin (M08C) | B3 |
| Y4 | **Search keys**: NFC, transliteration tables, Latin folding, in `roym_core`. | Directory search (B4); conversation search, contacts (B4); exchange relevance (M08F) | B4 |
| Y5 | **Full-text option in `data-layer`**: a collection declares text fields; the host keeps an FTS5 table in sync; a text-match filter operator. | Directory (B4); later the exchange, the shop browse (M08D) | B4 |
| Y6 | **`syneroym:translation` WIT and the no-op host.** | Every hook point (B5); later engines (M9A) | B5 |

## 2. Facts checked in the code for this plan

- Hub: about 104 `textContent = "..."` assignments in 37 source files. `index.html` says `lang="en"`. Money is formatted in `formatMinor` (`crates/roym_web/ui/src/money.ts:66`).
- `ProfilePayload.locale` exists and nothing reads it (`crates/roym_core/src/person.rs:26`).
- Categories are free `[a-z0-9-]` tokens, at most 8 per listing (`crates/roym_core/src/listing.rs:22`).
- Directory text search: `normalize_text` keeps only `[a-z0-9 -]` (`crates/roym_core/src/directory.rs:232`); the filter becomes `LIKE` (`crates/data_db/src/filter.rs:286`). Backlog §11 row "The directory text index is ASCII-folded" closes in B4.
- Guests cannot run DDL (`execute-ddl` unreachable from a service method, backlog §11). Collection indexes declared by `create-collection` are not used by `query` (backlog §11). B4 must not depend on either. It adds Y5 in the host instead.
- SQLite runs only in the host: `rusqlite` 0.38 with `bundled-sqlcipher`. `unverified`: FTS5 and the trigram tokenizer are compiled in.

## 3. Slices

### S0 — Spikes and decisions (docs only, S)

- Check FTS5 and the trigram tokenizer in the bundled SQLCipher build. If missing, find the build flag.
- Check that `icu_locale_core` builds for `wasm32-wasip2`. Check `@fluent/bundle` size in the Hub bundle.
- Look for a pure-Rust transliteration crate for Devanagari and Tamil that builds for `wasm32-wasip2`. Decide: use it, or write fixed tables.
- Collect the first spelling test set (open question in the change doc).
- Owner decisions still open in the change doc: taxonomy ownership, first taxonomy size, named area codes.

### B1 — Hub localisation (M)

- Y1. Fluent catalogs `en` and `hi`. Language setting. `Intl` formats for dates, numbers and money. `lang` on the page and on user text. Error codes to catalog messages. The build check for visible string literals.
- Criteria: ML-1 to ML-6.

### B2 — Taxonomy, units, payment methods, attributes (M)

- Y2 with `en` and `hi` labels. Category, unit and payment-method pickers in the Hub. Units and payment methods become codes in the listing.
- Criteria: ML-7 to ML-9, ML-28 (taxonomy part).

### B3 — Language-tagged records and chat (M)

- Y3. `lang` and author `translations` on `profile`, `listing`, `request`, `quote`. `languages` replaces `locale`. Up to 3 tags on chat messages (conversation WIT and stored row). Best-match display in the Hub.
- Criteria: ML-10 to ML-14, ML-25.

### B4 — Search in every script (L)

- Y5 first (a substrate change in `data_db` and the `data-layer` WIT), then Y4, then the directory switch from `LIKE` to FTS5 over search keys. Conversation search and contact search use the same keys. Names in any script, collation and grouping in the Hub.
- Criteria: ML-20 to ML-24.

### B5 — Translation interface, hook points, agreements (M)

- Y6. Call it at every hook point. The Hub hides the translate action. `viewed_lang` and `viewed_via_mt` in each party's signed acceptance. The Hub warning before accepting through MT.
- Criteria: ML-15 to ML-19, ML-26, ML-27.

### B6 — Close-out (S)

- Playwright e2e: a Hindi Hub user finds and books an English listing by category; a Latin-letter search finds a Devanagari title. Living docs updated. Change doc `status: done`.

## 4. Order and parallel work

```text
S0 ─▶ B1 ───────────────▶ B6
  ├─▶ B2 ─▶ B3 ─▶ B5 ───▶ B6
  └─▶ B4 (Y5 first) ─────▶ B6
```

- B1 touches only the Hub. It can start while M08A's last slices run.
- B4's `data-layer` part touches only the substrate. It can run beside B2 and B3.
- B5 needs B3 (language tags on records).

## 5. Not in this plan

- A real translation engine (with M9A local inference).
- Right-to-left scripts, voice, catalogs beyond `en` and `hi`.

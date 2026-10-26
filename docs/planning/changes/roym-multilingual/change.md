---
status: draft
living-docs-touched:
---
# Roym multilingual foundation

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change lets people use Roym in their own language. It also lets two people who speak different languages trade with each other. People can mix languages and scripts, for example Hindi typed in Latin letters, in chat, in contact names and in search. It is built as **M08B**, right after [Roym open foundation](../roym-open-foundation/change.md) (M08A) and before the [Value Exchange](../roym-value-exchange/change.md) work, so every screen and record that M8 adds is multilingual from the start.

Terms used here:

- **Roym:** the SynApp for local commerce and chat. See [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md).
- **SynApp, SynOrg, substrate:** see [TERMINOLOGY.md](../../../TERMINOLOGY.md).
- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **Signed record:** a Roym record (`profile`, `listing`, `request`, `quote`, and others) inside a signed envelope. The list is `RECORD_TYPES` in `crates/roym_core/src/record.rs:18`.
- **Language tag:** a code for a language, in [BCP 47](https://www.rfc-editor.org/info/bcp47) form. Examples: `hi` (Hindi), `ta` (Tamil), `mr` (Marathi), `en` (English), `hi-Latn` (Hindi written in Latin letters, often called Hinglish).
- **Taxonomy:** the shared list of category codes, with a label for each code in each language.
- **String catalog:** the file of all UI text for one language.
- **Machine translation (MT):** text translated by software, not by the author.
- **Script:** a writing system, such as Devanagari, Tamil or Latin. A BCP 47 tag can name it (`hi-Latn` is Hindi in Latin letters).
- **Transliteration:** writing the same words in another script, by sound (`राम` and `ram`). It is not translation.
- **Search key:** a form of a text that search compares instead of the text itself: Unicode NFC, lower case, and a Latin transliteration for non-Latin scripts.
- **Translation hook point:** a place in a Roym flow where the translation interface is called. With no engine, the call does nothing.

## Problem

Roym users in India speak many languages. Many people want to use Roym fully in their own language, with others who speak it. People also need to trade across languages. A Tamil-speaking buyer may need a Hindi-speaking plumber.

Today Roym is English only:

- The Hub has hard-coded English strings. There is no string catalog. The 37 Hub source files have about 104 `textContent = "..."` assignments alone. `index.html` says `lang="en"`.
- Dates use `toLocaleString()` with the browser default (`screens/listings.ts:468`). Money shows as `35.50 INR` (`money.ts:66`, `formatMinor`), not as `₹35.50` with Indian digit grouping.
- `ProfilePayload.locale: Option<String>` exists (`crates/roym_core/src/person.rs:26`). `profile.set` stores it (`crates/roym_profile/src/app/profile_ops.rs:99`). Nothing reads it.
- Listing categories are free-form English tokens of `[a-z0-9-]` (`crates/roym_core/src/listing.rs:185`). Each group picks its own words. A Hindi speaker cannot search for "plumber" in Hindi.
- No signed record says which language its free text is in.
- Directory text search keeps only ASCII letters, digits, spaces and hyphens (`normalize_text`, `crates/roym_core/src/directory.rs:232`). A query in Devanagari or Tamil script has no searchable characters. `directory.search` refuses it with `-32602` (`crates/roym_directory/src/app/search_ops.rs:256`). A listing title in Hindi is never found by text search. The [deferred backlog](../../deferred-backlog.md) already has a row for this.

The requirement `[APP-A11Y]` in [system-requirements-spec.md](../../../system-requirements-spec.md) lists localisation as "Envisioned".

## The four layers

The design has four layers. Each layer removes a part of the problem.

| Layer | What it does | Needs translation? |
| --- | --- | --- |
| **A. UI localisation** | The Hub shows its own text in the user's language, with local date, number and `₹` formats and readable Indic fonts. | No. A human translates the string catalog once. |
| **B. Language-neutral structure** | Categories, units, areas and (in M8) commitment kinds are **codes**. Each code has a label per language. | No. Two people who search, filter and compare by code need no translation. |
| **C. Language-tagged free text** | Every record with free text says which language the text is in. The author may add their own signed translations. | Only by the author, by choice. |
| **D. Machine translation** | A pluggable engine translates free text for the reader, on the reader's own node. | Yes, but only for display. Not built in v1. |

Most cross-language trade needs only layers A and B. The buyer searches by category code and area. The listing's price, slot, unit and payment terms are structured fields. Each party reads them in their own language. Free text (title, summary, chat) is the only part that needs C or D.

## Scope and non-goals

- In scope:
  - A string catalog for the Hub, with English and Hindi. Hindi proves the design works for an Indic script.
  - Local formats in the Hub: dates, times, numbers with Indian grouping, and money with the currency sign (`₹`).
  - A UI language setting in the Hub. It is a local preference, not signed.
  - A category taxonomy: codes with labels in English and Hindi. The process to add a code (see [Open questions](#open-questions)).
  - Units of measure as codes, with labels.
  - A language tag on every signed record type that has free text today: `profile`, `listing`, `request`, `quote`. And on `opinion` when M8 adds it.
  - Optional translations signed by the author, on the same records.
  - `languages` on the profile: an ordered list of language tags the person speaks. It replaces `locale`.
  - Optional language tags on chat messages, up to 3 per message, for messages that mix languages.
  - Names in any script: contact names, display names and shop names, with search keys so a name typed in one script finds the same name in another.
  - Search with SQLite FTS5 over search keys, so text search works in every script and across scripts by transliteration.
  - Attribute definitions in the taxonomy (used by the [shop](../roym-shop/change.md) change).
  - Widget and extension labels from the string catalog (see [Roym extensions](../roym-extensions/change.md)).
  - A fixed list of translation hook points (see [Translation hook points](#translation-hook-points)). All of them do nothing in v1.
  - A new substrate host interface for machine translation, with one implementation that always answers "unavailable". The Hub hides the translate action when no engine is available.
  - The agreement records which language each party viewed it in.
- Not in scope:
  - A real translation engine. It comes with or after M9A (local inference).
  - Voice input, voice output, speech-to-text.
  - Right-to-left scripts. Urdu (`ur`) is right-to-left. The Hub layout is not checked for it in this change. Urdu is recorded as known future work.
  - String catalogs for languages other than English and Hindi. Adding one later is a data change only (see ML-2).
  - Translating the taxonomy into every Indian language. This change ships English and Hindi labels.
  - Localised error text from the substrate or from `roymctl`. Only the Hub is localised.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **UI localisation** | | |
| ML-1 | Every user-visible string in the Hub comes from the string catalog. A check fails the UI build when a string literal is set as visible text outside the catalog. | |
| ML-2 | Adding a new language needs only a new catalog file and taxonomy labels. No code change. A missing message in that language falls back to English, never to a raw message id. | |
| ML-3 | A user switches the Hub language between English and Hindi. The choice survives a reload. The first choice comes from the browser's preferred languages. | |
| ML-4 | Dates, times and numbers use the Hub language's format. ₹1,00,000.50 shows with Indian digit grouping and the `₹` sign in both English and Hindi. | |
| ML-5 | The Hub sets `lang` on the page, and on each block of user text to that text's language tag, so the browser picks a correct font and screen readers read it in the right language. | |
| ML-6 | The Hub shows service errors by error code from the catalog. It does not show English error text from a service to a Hindi user, except as an extra detail line. | |
| **Language-neutral structure** | | |
| ML-7 | A listing's categories are codes from the taxonomy. The Hub shows each code's label in the reader's language. A code with no label in that language shows its English label. A code not in the taxonomy shows as the code itself. | |
| ML-8 | A Hindi-language Hub user and an English-language Hub user search with the same category code and get the same results. | |
| ML-9 | A product listing's unit is a code from a unit list. The Hub shows its label in the reader's language. | |
| **Language-tagged free text** | | |
| ML-10 | `profile`, `listing`, `request` and `quote` each carry a `lang` field. The signer must set it. A record with a missing or malformed tag fails validation. | |
| ML-11 | The author can add up to a fixed number of translations of the free-text fields, inside the same signed record. Each translation has its own language tag and is shown as "translated by the author". | |
| ML-12 | The Hub shows the version of a record's text that best matches the reader's languages: the original, or an author translation. It always shows which one it shows, and the reader can always open the original. | |
| ML-13 | A profile carries `languages`, an ordered list of language tags. `locale` is removed. | |
| ML-14 | A chat message may carry up to 3 language tags, main language first. A message without one is shown as "language not stated". | |
| **Machine translation** | | |
| ML-15 | A WIT interface for translation exists. Roym services can call it through the host. | |
| ML-16 | The v1 host implementation answers "unavailable" to every call. With it, the Hub shows no translate action anywhere. | |
| ML-17 | No code path sends a chat message, a record or any user text to a remote host for translation unless the node owner has configured a remote engine by name. v1 has no such configuration. | |
| **Agreements** | | |
| ML-18 | Each party's signed acceptance of an agreement records the language tag of the text it viewed, and whether that text was a machine translation. | |
| ML-19 | Before a party accepts an agreement whose free text it viewed through machine translation, the Hub shows a warning that the original, in the named language, is what binds. | |
| **Search** | | |
| ML-20 | Text search finds a listing by a word of its title in Devanagari, and in Tamil script. | |
| **Mixed languages and scripts** | | |
| ML-21 | A search for `nal wala` (Latin letters) finds a listing titled `नल वाला`, and a search for `नल वाला` finds a listing tagged `hi-Latn` with the title `nal wala`. | |
| ML-22 | Common spelling variants in Latin letters match each other (for example `naal waala` and `nalwala` match `nal wala`). The folding rules are a fixed table, tested with real samples. | |
| ML-23 | A contact, display name or shop name in any script is stored as written. Contact search finds `राम` with `ram`, and `Ram` with `राम`. | |
| ML-24 | The Hub sorts names with the browser's collation for the UI language. Names in other scripts are grouped by script, not mixed by byte order. | |
| ML-25 | A message that mixes Hindi and English words keeps its text as written. Its tags list both languages. The Hub sets `lang` from the first tag. | |
| ML-29 | A query word of 1 or 2 characters (after folding) is matched as a whole word or a word prefix through a second index, not through the trigram index. A query made only of such words still returns results, and never runs a full table scan. | |
| **Translation hook points** | | |
| ML-26 | Each hook point in [Translation hook points](#translation-hook-points) calls the translation interface. With the v1 implementation, each call returns `unavailable`, and the flow continues with the original text. A test covers every hook point. | |
| ML-27 | A translate action appears in the Hub only at a hook point, and only when `supported-targets` is not empty. | |
| **Extension and taxonomy labels** | | |
| ML-28 | Taxonomy attribute names and choice values, widget labels and extension step names come from the string catalog and show in the reader's language, with the English label as fallback. | |

## Design

### Rules for every layer

1. **The signed original always wins.** A record means what its author signed, in the language its author tagged. A translation by software is for display only. It is labelled. It is never signed as the author's words.
2. **Translation runs on the reader's own node.** Chat is end-to-end encrypted. Roym never sends user content to a server for translation by default. A remote engine is allowed only if the node owner chooses it by name, and its output is labelled with that name.
3. **Structure before text.** If a field can be a code, it is a code. Codes are never translated. Only their labels are.
4. **No migrations.** The product is not released. Records change in place. Every signed record already has an explicit `version`. This change keeps `version` at 1 for each record type it changes, as earlier unreleased changes did (`unverified`: confirm the team's rule for an in-place change before release).

### A. UI localisation

**String catalog.** One catalog file per language under `crates/roym_web/ui/src/i18n/`. Each message has a stable id. The Hub looks up every visible string by id. The recommended format is [Fluent](https://projectfluent.org/) (see [Which catalog format](#which-catalog-format)).

**Language choice.** The Hub keeps the user's UI language as a local preference. The first value comes from `navigator.languages`, matched against the catalogs the build ships. The UI language is a device setting. It is separate from the profile's `languages` list, which tells other people what the person speaks.

**Formats.** All formatting goes through the browser's `Intl` API with the Hub language:

- Dates and times: `Intl.DateTimeFormat`. Replace every `toLocaleString()` call with no locale.
- Numbers: `Intl.NumberFormat`. For `en-IN` and `hi-IN` this gives Indian grouping (`1,00,000`). `unverified` in every target browser.
- Money: keep `formatMinor` as the one place that turns minor units into text. Change its output to `Intl.NumberFormat` with `style: "currency"`. Amounts stay integer minor units on the wire.
- Digits: Latin digits (`0-9`) by default in every language. Native digits (for example Devanagari `०-९`) are a later user option.

**Fonts.** Today the Hub uses `font-family: system-ui, -apple-system, sans-serif` (`style.css:2`). Android, iOS, macOS and Windows ship fonts for the major Indic scripts (`unverified` for older Android versions). The first step uses system fonts and the correct `lang` attribute (ML-5). A bundled web font is added only if a test device shows broken shaping. See [Open questions](#open-questions).

**Errors.** Services return English error text. The Hub maps known error codes to catalog messages (ML-6). About 25 places in the Hub read `.message` from an error today. Each one is checked.

### B. Language-neutral structure

**Categories.** A category stays a token of `[a-z0-9-]`, as today. The wire shape of `ListingPayload.categories` does not change. What changes is where the tokens come from:

- Roym ships a **base taxonomy**: a data file in `crates/roym_core` with each code, its parent code (for a simple tree), and its labels per language.
- The Hub category picker offers only taxonomy codes. It shows labels in the user's language, and it searches those labels in any language the taxonomy has.
- `ListingPayload::validate` still accepts any well-formed token. A code outside the base taxonomy is allowed, so a group can use its own codes before they are added. The Hub shows such a code as the raw code (ML-7).
- The same codes are used in `request.categories` and, in M8, in wants.

Rejected: validating categories against the taxonomy inside `validate`. Then every taxonomy change would need a new build on every node before old nodes could read new listings.

**Units.** `ProductDetail.unit` is a free string today (`crates/roym_core/src/listing.rs:104`, up to 32 bytes). It becomes a code from a short unit list in `crates/roym_core` (for example `kg`, `g`, `l`, `ml`, `piece`, `dozen`, `hour`). Each code has labels per language. Using [UN/CEFACT Recommendation 20](https://unece.org/trade/uncefact/cl-recommendations) codes was considered. They are hard to read in records (`KGM`, `H87`), and Roym needs only a few units. Roym keeps its own short codes.

**Areas.** Geometric areas (`Bbox`, `Circle`) are already language-neutral. `Area::Named` has a free `label` and an optional `code` (`crates/roym_core/src/area.rs:29`). The label is free text in the record's language. The `code` is how a named area becomes language-neutral (for example an Indian PIN code). Which code system to use is not decided here.

**Payment methods.** `PaymentTerms.methods` holds short free strings today (`listing.rs:88`). They become codes (for example `upi`, `cash`, `bank-transfer`) with labels, the same way as units.

**Commitment kinds and extension fields** (M8). The Value Exchange adds resource kinds and commitment kinds. Roym extensions add schema fields, step types and widgets. Each of these is a code with catalog labels, never free text. This change states the rule. The deal core (M08C) and the extensions follow it.

**Attributes.** The taxonomy also defines attributes per category, for the [shop](../roym-shop/change.md) change. A choice attribute's values are codes with labels, like categories.

### C. Language-tagged free text

**One tag per record.** Each record type with free text gets a required field:

```text
lang: string   // BCP 47 tag of all free text in this record, e.g. "hi", "ta", "hi-Latn"
```

One tag covers every free-text field in the record. A person writes one record in one language. A tag per field was rejected: it adds a field to every string, and mixed-language records are rare. A record that mixes languages is tagged with its main language.

Free-text fields, by record, today:

| Record | Free-text fields |
| --- | --- |
| `profile` | `display_name`, `about` |
| `listing` | `title`, `summary`, `service.includes`, `service.excludes`, `service.prerequisites`, `Area::Named.label`, `payment.payee` |
| `request` | `description`, `data_use_notice` |
| `quote` | `terms.scope`, `terms.cancellation_terms`, `terms.refund_terms`, `terms.dispute_path`, `location.address` |
| `opinion` (M8) | its text fields |

Names (`display_name`, `payee`) and addresses are not translated. They are covered by the tag so the Hub can mark the text with the right `lang` (ML-5).

**Author translations.** A record may carry `translations`: a short list. Each entry has its own `lang` and the same translatable fields as the record (for a listing: `title` and `summary`). The author signs them with the record. The Hub shows them as "translated by the author". The original `lang` text still binds. The maximum number of entries is set in the plan (a small number such as 4, to keep record size bounded, as other limits in `listing.rs` do).

**Tag validation.** The Rust side checks that each tag is a well-formed BCP 47 tag and stores it in canonical form. Recommended crate: `icu_locale_core` from the ICU4X project. It is pure Rust and widely used. `unverified`: that it builds for `wasm32-wasip2` with no extra features. The Hub uses `Intl.Locale` to parse tags.

**Profile languages.** `ProfilePayload.locale: Option<String>` is replaced by:

```text
languages: Vec<String>   // BCP 47 tags, most preferred first, at most 8 (limit set in the plan)
```

The Hub shows it on the profile ("Speaks Hindi, Marathi"). The Hub may use it to rank author translations and, later, to offer MT.

**Chat messages.** A message is not a signed record. The conversation `message` WIT record has a `content-type` (`crates/wit_interfaces/wit/conversation/`). This change adds an optional list of up to 3 language tags next to it, main language first. People in India often mix languages in one message, so one tag is not enough. A signed record still has one tag: its main language. The Hub sets it from the UI language, and the sender can change it. Putting the tag inside `content-type` as a parameter was rejected: it is not a standard parameter, and code that compares content types would break.

### D. Machine translation

**Host interface.** A new WIT package under `crates/wit_interfaces/wit/translation/`, in the style of `signing`. A first draft:

```wit
package syneroym:translation@0.1.0;

interface translation {
    variant translation-error {
        /// No engine on this node. The v1 implementation always returns this.
        unavailable,
        /// The engine cannot translate between these two languages.
        unsupported-pair(tuple<string, string>),
        /// The text is longer than the engine accepts.
        too-long(u32),
        engine-failed(string),
    }

    record engine-info {
        /// Shown to the reader next to every translation.
        name: string,
        /// True when the text leaves this node.
        remote: bool,
    }

    record translated {
        text: string,
        /// The source language the engine used (stated or detected).
        source-lang: string,
        engine: engine-info,
    }

    record detected {
        lang: string,
        /// 0..=1000.
        confidence: u16,
    }

    /// Empty when no engine is configured. The Hub hides the translate
    /// action when this is empty.
    supported-targets: func() -> list<string>;
    detect-language: func(text: string) -> result<list<detected>, translation-error>;
    translate: func(text: string, source-lang: option<string>, target-lang: string)
        -> result<translated, translation-error>;
}
```

The exact names are set in the plan. The rules are fixed here:

- The host owns the engine. A guest cannot choose a remote engine.
- The v1 implementation returns `unavailable` from every call and an empty list from `supported-targets`.
- Every successful result names its engine and says whether it was remote. The Hub labels every translation with this.
- The output is a plain string for display. No API takes a translation as input to signing.

**How the Hub reaches it.** The Hub calls a local-only Roym method, and that service calls the host interface. Which Roym service holds the method is set in the plan. `web` is the likely home, because it already serves the Hub. The method is local-only (`roym_core::admit::require_internal`), like every Roym method except the four `directory` methods.

**Later engines.** Two kinds are expected:

1. A local model on the node. This fits the M9A local inference work ([meta-implementation-plan.md](../../meta-implementation-plan.md), Milestone 9A).
2. A remote service that the node owner chooses by name in the substrate config. Its results show a label such as "translated by `<engine name>`, sent off this device".

Neither is built in this change.

### Agreements across languages

Each party agrees to the original text, in the language the author tagged. Structured fields (price, currency, slot, unit, category, commitment kinds) need no translation. Each party sees them in its own UI language.

For free text, each party's signed acceptance records what that party viewed:

```text
viewed_lang: string          // the language tag of the text shown
viewed_via_mt: bool          // true if it was a machine translation
```

Today an `agreement-receipt` is made of halves, one signed by each party (`ReceiptHalf`, `crates/roym_core/src/transaction.rs:205`). The two fields go into the payload each party signs. The deal core (M08C) changes the canonical bytes of the quote and the receipt for selective disclosure (see the [Value Exchange](../roym-value-exchange/change.md) change, "Agreements made of commitments"). This change adds these fields first. M08C keeps them.

Before acceptance through MT, the Hub shows: the original binds, it is in the named language, and the shown text is a machine translation (ML-19). An author translation (layer C) is not MT. It is signed by the author, so `viewed_via_mt` is false. The original still binds.

### Search

Category search is language-neutral after layer B. Text search is not. Today:

- The directory index stores an ASCII-folded `text` field. A search builds a `$regex` filter. `data_db` compiles `$regex` to `json_extract(payload, ?) LIKE '%pattern%'` (`crates/data_db/src/filter.rs:286`). There is no SQLite FTS table. So the SQLite FTS tokenizer question does not apply today.
- `normalize_text` drops every non-ASCII character. So Devanagari and Tamil text is not indexed, and such a query is refused.

**Decision (2026-10-26): search moves to SQLite FTS5 over search keys, in this change.**

- **Search key.** Each indexed text is turned into a search key: Unicode NFC, lower case, then a Latin transliteration for every non-Latin script, then a loose folding of Latin spelling (for example long and short vowels fold together, doubled letters fold to one). Combining marks (Unicode `Mn` and `Mc`, such as Devanagari vowel signs and the virama) are kept until the transliteration step, so words do not break. The index stores both the NFC original and the key.
- **The query gets the same treatment.** A query in Devanagari and a query in Latin letters both become the same kind of key. So `nal wala`, `naal waala` and `नल वाला` meet (ML-21, ML-22).
- **FTS5 with the trigram tokenizer.** It works for any script, with no word list per language, and finds a word inside a longer word. It replaces the `$regex` and `LIKE` path for text search. Category, area and attribute filters stay structured.
- **Short words.** The trigram index cannot match a query term shorter than 3 characters, and short transliterated words are common (`ek`, `do`, `na`). So each text field is also indexed in a second FTS5 table with the `unicode61` word tokenizer and a prefix index for 1 and 2 characters, over the same search keys. Search keys are in Latin letters, so word tokens work for every script. A query term of 1 or 2 characters goes to this index as a whole-word or prefix match. Longer terms go to the trigram index. A query with only short terms never falls back to a scan (ML-29).
- **The transliteration table** is a fixed table in `roym_core`, one per script, starting with Devanagari and Tamil. It is lossy by design: it only has to make two spellings of the same word meet. ISO 15919 is the starting point. The folding rules are tested with a set of real spellings collected for the test data (`unverified`: whether a pure-Rust crate already does this well enough; the plan checks before writing one).
- `unverified`: that the SQLite build used by `data_db` (`rusqlite` with `bundled-sqlcipher`) has FTS5 and the trigram tokenizer enabled. SQLite always runs in the host, so this one build serves both the native and the WASM build of Roym. The plan checks this first.
- **A guest cannot create an FTS5 table today.** Guests reach storage only through `data-layer` collections. `execute-ddl` is unreachable from a service method (deferred-backlog §11). So `data-layer` gains a full-text option on a collection (the fields to index, kept in sync by the host) and a text-match filter operator. This is a substrate change, and it is planned first.
- The index shape changes. The product is unreleased, so the index is rebuilt in place.

The same search keys serve the conversation search (`conversation.search`) and the contact list.

### Names in any script

A name (contact, display name, shop name) is stored exactly as written, in any script. Next to it, the node stores its search key (see [Search](#search)). Contact search and the contact picker compare keys, so `ram` finds `राम` (ML-23). The Hub sorts names with `Intl.Collator` for the UI language, and groups names by script (ML-24). A name is never translated. The reader's node may show a transliteration next to a name in a script the reader does not read. This is a display choice, not a signed field.

### Translation hook points

The translation interface is called at these points, and at no others. With the v1 engine, every call returns `unavailable`, and the flow uses the original text (ML-26).

| Hook point | What is translated | Where it runs |
| --- | --- | --- |
| Search query | the query text, into the languages of the indexed records | the searching participant's node, before the query is sent |
| Record display | free text of an offering, want, storefront, quote | the reader's node |
| Chat message | the message text | the reader's node |
| Widget answer | free-text fields of a widget answer or step record | the reader's node |
| Opinion | the opinion text | the reader's node |
| Agreement review | the free text of an agreement before acceptance, with ML-18 and ML-19 | the reader's node |

A translation is always labelled with its engine and is never signed (rule 1 above).

### Which catalog format

Recommended: **Fluent** (`@fluent/bundle`), from Mozilla.

- It lets a translator add plural and gender variants in one language without a code change. Hindi verbs agree with the gender of the subject. English does not. With Fluent the Hindi file can have the variant and the English file does not need it.
- The file format is plain text and easy for non-developers to read and edit.
- It is mature. Firefox uses it.
- It has no framework dependency. The Hub has none either.

Considered:

- **ICU MessageFormat 1** (for example `intl-messageformat`): widely used. Its syntax is harder for translators. A gender or plural variant needs the developer to put the selector in every language's message.
- **Unicode MessageFormat 2**: the new standard, and partly based on Fluent's ideas. Browser support (`Intl.MessageFormat`) is still a proposal (`unverified` as of this writing). A later move from Fluent to MF2 is a file conversion.
- **A plain JSON map of id to string**: simplest, but it has no plurals or gender. Every such case would become code in the Hub.

Bundle size of `@fluent/bundle` is `unverified`. It is checked before the choice is final.

## Rejected options

- **Translate messages on a server.** Chat is end-to-end encrypted. A server would see every message. It would also create a central service that a peer-to-peer product should not depend on.
- **Translate structured fields.** Price, slot, unit and category are codes or numbers. Translating them adds error and gains nothing. Each reader sees their labels.
- **Keep free-form categories.** Then cross-language search on category would need translation. Each group would also keep its own words for the same thing, so search across groups (M8 federation) would not match.
- **Sign machine translations.** A signature means "the signer said this". A machine translation is not what the author said. Signing it would make an agreement bind to words nobody wrote.
- **Validate categories against a fixed list in `ListingPayload::validate`.** Every taxonomy change would then need every node updated first. See [Categories](#b-language-neutral-structure).
- **A language tag per field.** See [One tag per record](#c-language-tagged-free-text).
- **Keep `locale` as one value.** Many people in India speak two or three languages. One value cannot say that.

## Open questions

- **Search over Indic scripts, and cross-script search.** Decided 2026-10-26: in scope, with FTS5 and search keys (see [Search](#search)).
- **Spelling test data.** The folding rules need real samples of Hindi and Tamil typed in Latin letters. Who collects them, and how many are enough (a first value: 200 word pairs per language).
- **Who maintains the taxonomy and its labels.** Recommendation: the Roym project maintains the base taxonomy in the repository. A group can use its own codes at once (they show as raw codes). A group then proposes a code for the base taxonomy through the normal contribution process. Labels in a new language come with that language's string catalog. The owner decides. A later option: an exchange (M8) publishes labels for its own group codes.
- **The first taxonomy.** Start from a small hand-made list of local services and goods (for example 50 to 150 codes), or adapt an existing product taxonomy. The licence of any existing taxonomy is `unverified`.
- **Named area codes.** Is a PIN code the right language-neutral code for `Area::Named.code`? Or a code from India's Local Government Directory? Not decided.
- **Font size and bundle cost.** If system fonts are not enough on some devices, a Noto font for each script must be bundled or loaded on demand. Size per script is `unverified`. Loading on demand means a network fetch from the node, which is allowed (the Hub is served by the node).
- **Native digits.** Do Hindi users want `१२३` or `123` by default? Recommendation: `123`, with a later option. `unverified`; ask users.
- **Catalog format.** Fluent is recommended above. The owner confirms.

## Tasks

- [ ] Write the M08B slice plans with exact field names, limits and WIT names.
- [ ] Hub: add the i18n module, the Fluent catalogs (`en`, `hi`), and the language setting. Move every visible string into the catalog.
- [ ] Hub: add a build check that fails on a visible string literal outside the catalog (ML-1).
- [ ] Hub: format dates, numbers and money with `Intl` and the Hub language. Set `lang` on the page and on user text.
- [ ] Hub: map service error codes to catalog messages.
- [ ] `roym_core`: add the base taxonomy, unit codes and payment-method codes, each with `en` and `hi` labels.
- [ ] Hub: category, unit and payment-method pickers use codes and show labels.
- [ ] `roym_core`: add `lang` and `translations` to `profile`, `listing`, `request`, `quote`. Validate tags.
- [ ] `roym_core`: replace `ProfilePayload.locale` with `languages`. Update `profile.set`.
- [ ] Conversation: add up to 3 optional language tags to the `message` WIT record and the stored row.
- [ ] `roym_core`: search keys (NFC, transliteration tables for Devanagari and Tamil, Latin folding) and their tests with real samples.
- [ ] FTS5 index over search keys for the directory (and later the exchange), conversation search and contacts.
- [ ] Names in any script: search keys on contacts, collation and grouping in the Hub.
- [ ] Taxonomy attribute definitions with `en` and `hi` labels.
- [ ] Call the translation interface at every hook point. A test for each.
- [ ] Add the `syneroym:translation` WIT package and the host implementation that answers `unavailable`. Link it for native and WASM builds.
- [ ] Add the local-only Roym method that calls it. The Hub hides translate when `supported-targets` is empty.
- [ ] Add `viewed_lang` and `viewed_via_mt` to each party's signed acceptance. Add the Hub warning (ML-19).
- [ ] Replace `normalize_text` and the `LIKE` search with the FTS5 path (ML-20 to ML-22). Close the matching deferred-backlog row.
- [ ] Integration and Hub tests for each ML row. Playwright e2e: a Hindi Hub user finds and books a listing written in English, by category.
- [ ] Update `[APP-A11Y]` in the requirements, the integrated spec and the architecture doc at close-out.

## Deviations

## Close-out

Before merge, update the living docs (requirements, architecture, user guide) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).
